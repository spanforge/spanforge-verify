//! Exact-by-default, bounded diagnostics for private comparison captures.
use crate::{
    privacy::Masker,
    strict_json::{self, Value},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub json_stdout: bool,
    pub json_stderr: bool,
    pub crlf_to_lf: bool,
    pub ignore_json_pointers: Vec<String>,
    pub json_files: Vec<String>,
    pub text_files: Vec<String>,
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if self.ignore_json_pointers.len() > 64
            || self
                .ignore_json_pointers
                .iter()
                .any(|p| p.is_empty() || p.len() > 4096 || !strict_json::valid_pointer(p))
            || (!self.ignore_json_pointers.is_empty()
                && !self.json_stdout
                && !self.json_stderr
                && self.json_files.is_empty())
        {
            return Err("Ignored pointers require JSON stream mode, at most 64 valid non-root JSON pointers of at most 4096 bytes".into());
        }
        if self.json_files.len() + self.text_files.len() > 64
            || !crate::inputs::unique_names(
                self.json_files
                    .iter()
                    .chain(&self.text_files)
                    .map(String::as_str),
            )
        {
            return Err("File diff modes require at most 64 distinct workspace paths".into());
        }
        for path in self.json_files.iter().chain(&self.text_files) {
            crate::schema::workspace_path(path)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ContentDifference {
    pub case_id: String,
    pub field: String,
    pub path: String,
    pub kind: String,
    pub baseline: Option<String>,
    pub candidate: Option<String>,
    pub impact: crate::compatibility::Impact,
    pub rule_id: Option<String>,
    pub policy_reason: Option<String>,
}

pub struct Differences<'a> {
    pub items: Vec<ContentDifference>,
    pub omitted: u64,
    masker: &'a Masker,
    policy: Option<&'a crate::compatibility::Policy>,
    pub blocking: u64,
    pub allowed: u64,
    pub breaking: u64,
}
impl<'a> Differences<'a> {
    pub fn new(masker: &'a Masker) -> Self {
        Self {
            items: vec![],
            omitted: 0,
            masker,
            policy: None,
            blocking: 0,
            allowed: 0,
            breaking: 0,
        }
    }
    pub fn with_policy(
        masker: &'a Masker,
        policy: Option<&'a crate::compatibility::Policy>,
    ) -> Self {
        Self {
            policy,
            ..Self::new(masker)
        }
    }
    pub fn add(
        &mut self,
        case: &str,
        field: &str,
        path: &str,
        kind: &str,
        old: Option<String>,
        new: Option<String>,
    ) {
        let mandatory_regression = matches!(kind, "contract_regression" | "format_regression");
        let rule = self
            .policy
            .and_then(|p| p.classify(case, field, path))
            .filter(|r| {
                !(mandatory_regression && r.impact == crate::compatibility::Impact::Allowed)
            });
        let impact = if matches!(kind, "contract_regression" | "format_regression") {
            crate::compatibility::Impact::Breaking
        } else {
            rule.map_or(crate::compatibility::Impact::Changed, |r| r.impact)
        };
        match impact {
            crate::compatibility::Impact::Allowed => self.allowed += 1,
            crate::compatibility::Impact::Breaking => {
                self.breaking += 1;
                self.blocking += 1;
            }
            crate::compatibility::Impact::Changed => self.blocking += 1,
        }
        if self.items.len() >= 1000 {
            self.omitted += 1;
            return;
        }
        self.items.push(ContentDifference {
            case_id: self.masker.diagnostic(case).text,
            field: field.into(),
            path: if (matches!(field, "stdout" | "stderr") && path.starts_with('/'))
                || (field == "workspace" && path.contains('#'))
            {
                masked_pointer(self.masker, path)
            } else {
                self.masker.diagnostic(path).text
            },
            kind: kind.into(),
            baseline: old.map(|s| self.masker.diagnostic(&s).text),
            candidate: new.map(|s| self.masker.diagnostic(&s).text),
            impact,
            rule_id: rule.map(|r| self.masker.diagnostic(&r.id).text),
            policy_reason: rule.map(|r| self.masker.diagnostic(&r.reason).text),
        });
    }
    pub fn stream(
        &mut self,
        case: &str,
        field: &str,
        old: &[u8],
        new: &[u8],
        options: &Options,
    ) -> Result<(), String> {
        let json = if field == "stdout" {
            options.json_stdout
        } else {
            options.json_stderr
        };
        if json {
            let old = strict_json::parse(old)
                .map_err(|_| format!("Baseline {field} is not supported strict JSON"))?;
            let new = match strict_json::parse(new) {
                Ok(value) => value,
                Err(strict_json::Error::Invalid(_)) => {
                    self.add(
                        case,
                        field,
                        "",
                        "format_regression",
                        Some("Valid strict JSON".into()),
                        Some("Invalid strict JSON".into()),
                    );
                    return Ok(());
                }
                Err(_) => {
                    return Err(format!(
                        "Candidate {field} exceeds supported strict JSON resources"
                    ));
                }
            };
            self.json(case, field, "", Some(&old), Some(&new), options);
        } else {
            let original = (old, new);
            let normalized_old;
            let normalized_new;
            let (old, new) = if options.crlf_to_lf {
                normalized_old = normalize(old);
                normalized_new = normalize(new);
                (normalized_old.as_slice(), normalized_new.as_slice())
            } else {
                (old, new)
            };
            if old != new {
                let offset = old
                    .iter()
                    .zip(new)
                    .position(|(a, b)| a != b)
                    .unwrap_or(old.len().min(new.len()));
                // Byte offsets accompany bounded text diagnostics. No lossy
                // decoding or masking is used to decide equality.
                let (before, after) = excerpts(
                    original.0,
                    original.1,
                    offset,
                    self.masker,
                    options.crlf_to_lf,
                );
                self.add(
                    case,
                    field,
                    &format!("byte:{offset}"),
                    "content_changed",
                    Some(before),
                    Some(after),
                );
            }
        }
        Ok(())
    }
    pub fn file(
        &mut self,
        case: &str,
        path: &str,
        content: (&[u8], &[u8]),
        json: bool,
        options: &Options,
    ) -> Result<(), String> {
        let (old, new) = content;
        if json {
            let a = strict_json::parse(old)
                .map_err(|_| "Baseline selected file is not supported strict JSON")?;
            let b = match strict_json::parse(new) {
                Ok(value) => value,
                Err(strict_json::Error::Invalid(_)) => {
                    self.add(
                        case,
                        "workspace",
                        &format!("{path}#"),
                        "format_regression",
                        Some("Valid strict JSON".into()),
                        Some("Invalid strict JSON".into()),
                    );
                    return Ok(());
                }
                Err(_) => {
                    return Err(
                        "Candidate selected file exceeds supported strict JSON resources".into(),
                    );
                }
            };
            let mut file_options = options.clone();
            file_options.ignore_json_pointers = options
                .ignore_json_pointers
                .iter()
                .map(|p| format!("{path}#{p}"))
                .collect();
            self.json(
                case,
                "workspace",
                &format!("{path}#"),
                Some(&a),
                Some(&b),
                &file_options,
            );
        } else if old != new {
            let offset = old
                .iter()
                .zip(new)
                .position(|(a, b)| a != b)
                .unwrap_or(old.len().min(new.len()));
            let (a, b) = excerpts(old, new, offset, self.masker, false);
            self.add(case, "workspace", path, "content_changed", Some(a), Some(b));
        }
        Ok(())
    }
    fn json(
        &mut self,
        case: &str,
        field: &str,
        path: &str,
        old: Option<&Value>,
        new: Option<&Value>,
        options: &Options,
    ) {
        if options.ignore_json_pointers.iter().any(|p| p == path) || old == new {
            return;
        }
        match (old, new) {
            (Some(Value::Object(a)), Some(Value::Object(b))) => {
                let keys = a.keys().chain(b.keys()).collect::<BTreeSet<_>>();
                for key in keys {
                    let pointer = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    self.json(case, field, &pointer, a.get(key), b.get(key), options);
                }
            }
            (Some(Value::Array(a)), Some(Value::Array(b))) => {
                for i in 0..a.len().max(b.len()) {
                    self.json(
                        case,
                        field,
                        &format!("{path}/{i}"),
                        a.get(i),
                        b.get(i),
                        options,
                    );
                }
            }
            _ => self.add(
                case,
                field,
                path,
                if old.is_none() {
                    "added"
                } else if new.is_none() {
                    "removed"
                } else {
                    "changed"
                },
                old.map(|v| summary(v, self.masker)),
                new.map(|v| summary(v, self.masker)),
            ),
        }
    }
}
pub fn masked_pointer(masker: &Masker, pointer: &str) -> String {
    let decoded = pointer.replace("~1", "/").replace("~0", "~");
    if masker.mask(&decoded) != decoded {
        "[REDACTED POINTER]".into()
    } else {
        masker.diagnostic(pointer).text
    }
}
fn summary(value: &Value, masker: &Masker) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(v) => v.to_string(),
        Value::String(v) => serde_json::to_string(&masker.mask(v)).unwrap(),
        Value::Number(v) => v.to_string(),
        Value::Array(v) => format!("array ({} elements)", v.len()),
        Value::Object(v) => format!("object ({} fields)", v.len()),
    }
}
fn normalize(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"\r\n") {
            result.push(b'\n');
            i += 2;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    result
}
fn excerpts(
    old: &[u8],
    new: &[u8],
    offset: usize,
    masker: &Masker,
    normalize_display: bool,
) -> (String, String) {
    match (std::str::from_utf8(old), std::str::from_utf8(new)) {
        (Ok(a), Ok(b)) => {
            // Mask whole strings before choosing an excerpt; never split a secret
            // into separate masking inputs. Equality was decided on raw bytes.
            let mut a = masker.mask(a);
            let mut b = masker.mask(b);
            if normalize_display {
                a = a.replace("\r\n", "\n");
                b = b.replace("\r\n", "\n");
            }
            let at = a
                .bytes()
                .zip(b.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(a.len().min(b.len()));
            (text_excerpt(&a, at, offset), text_excerpt(&b, at, offset))
        }
        _ => {
            use sha2::{Digest, Sha256};
            let describe = |bytes: &[u8]| {
                format!(
                    "binary ({} bytes), sha256={:x}, first difference at byte {offset}",
                    bytes.len(),
                    Sha256::digest(bytes)
                )
            };
            (describe(old), describe(new))
        }
    }
}
fn text_excerpt(text: &str, at: usize, original_offset: usize) -> String {
    let mut start = at.saturating_sub(256).min(text.len());
    let mut end = at.saturating_add(768).min(text.len());
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let line = text[..start].bytes().filter(|b| *b == b'\n').count() + 1;
    format!(
        "first difference at byte {original_offset}; excerpt starts at masked line {line}: {}{}{}",
        if start > 0 { "..." } else { "" },
        &text[start..end],
        if end < text.len() { "..." } else { "" }
    )
}
