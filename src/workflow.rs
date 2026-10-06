//! Bounded workflow compilation and typed runtime bindings.
use crate::schema::{Case, FileKind, StreamRule, Suite};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
pub fn one() -> u32 {
    1
}
pub fn is_one(n: &u32) -> bool {
    *n == 1
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: String,
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub stdin_text: Option<String>,
    pub stdin_file: Option<String>,
    pub fixture_dir: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub group: String,
    pub number: u32,
    pub total: u32,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub pointer: String,
    pub kind: ScalarType,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    String,
    Number,
    Boolean,
    Null,
}
fn name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn selected(case: &Case, selection: &str) -> bool {
    case.id == selection
        || case.origin.as_deref() == Some(selection)
        || case.attempt.as_ref().is_some_and(|a| a.group == selection)
}
fn substitute(
    s: &mut String,
    prefix: &str,
    values: &BTreeMap<String, String>,
) -> Result<(), String> {
    // Single pass: inserted values are data and are never recursively expanded.
    let marker = format!("{{{{{prefix}.");
    let mut remaining = s.as_str();
    let mut result = String::new();
    while let Some(start) = remaining.find(&marker) {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + marker.len()..];
        let end = remaining.find("}}").ok_or("Unclosed substitution")?;
        let value = values
            .get(&remaining[..end])
            .ok_or("Unknown or unavailable substitution binding")?;
        if result.len() + value.len() > 1024 * 1024 {
            return Err("Expanded substitution exceeds 1 MiB".into());
        }
        result.push_str(value);
        remaining = &remaining[end + 2..];
    }
    if result.len() + remaining.len() > 1024 * 1024 {
        return Err("Expanded substitution exceeds 1 MiB".into());
    }
    result.push_str(remaining);
    *s = result;
    Ok(())
}
fn strings(
    case: &mut Case,
    prefix: &str,
    values: &BTreeMap<String, String>,
    paths: bool,
) -> Result<(), String> {
    for arg in &mut case.args {
        substitute(arg, prefix, values)?;
    }
    for v in case.env.values_mut() {
        substitute(v, prefix, values)?;
    }
    if let Some(s) = &mut case.stdin_text {
        substitute(s, prefix, values)?;
    }
    for rule in [&mut case.stdout, &mut case.stderr].into_iter().flatten() {
        match rule {
            StreamRule::TextEquals { text, .. }
            | StreamRule::Contains { text, .. }
            | StreamRule::NotContains { text, .. } => substitute(text, prefix, values)?,
            _ => {}
        }
    }
    if paths {
        for s in [&mut case.fixture_dir, &mut case.cwd, &mut case.stdin_file]
            .into_iter()
            .flatten()
        {
            substitute(s, prefix, values)?;
        }
        for rule in [&mut case.stdout, &mut case.stderr].into_iter().flatten() {
            if let StreamRule::ExactFile { expected_file }
            | StreamRule::JsonEqualsFile { expected_file } = rule
            {
                substitute(expected_file, prefix, values)?;
            }
        }
        for rule in &mut case.files {
            substitute(&mut rule.path, prefix, values)?;
            if let FileKind::File { expected_file, .. } = &mut rule.rule {
                substitute(expected_file, prefix, values)?;
            }
        }
    }
    for step in &mut case.steps {
        strings(step, prefix, values, paths)?;
    }
    Ok(())
}
pub fn expand(suite: &mut Suite) -> Result<(), String> {
    let mut expanded = Vec::new();
    let mut compiled_bytes = 0usize;
    let mut mappings = BTreeMap::new();
    let original = std::mem::take(&mut suite.cases);
    for mut case in original {
        validate_scenario(&case)?;
        if case.origin.as_ref().is_some_and(|origin| {
            !name(origin)
                || case.repeat != 1
                || !case.matrix.is_empty()
                || !case.id.strip_prefix(origin).is_some_and(|suffix| {
                    suffix.starts_with("--matrix-") || suffix.starts_with("--repeat-")
                })
        }) {
            return Err("Invalid expanded-case origin".into());
        }
        if !name(&case.id) || !(1..=100).contains(&case.repeat) || case.matrix.len() > 64 {
            return Err("Invalid workflow ID, repeat count or matrix size".into());
        }
        if let Some(a) = &case.attempt
            && (case.repeat != 1
                || !case.matrix.is_empty()
                || !name(&a.group)
                || !(2..=100).contains(&a.total)
                || a.number == 0
                || a.number > a.total
                || case.id != format!("{}--repeat-{:03}", a.group, a.number))
        {
            return Err("Invalid attempt identity".into());
        }
        let source = case.id.clone();
        let count = case.repeat;
        let variants = std::mem::take(&mut case.matrix);
        let mut rows = Vec::new();
        let mut row_bytes = 0usize;
        if variants.is_empty() {
            strings(&mut case, "matrix", &BTreeMap::new(), true)?;
            rows.push(case);
        } else {
            let mut names = BTreeSet::new();
            for variant in variants {
                if !name(&variant.id)
                    || !names.insert(variant.id.clone())
                    || variant.values.len() > 32
                    || variant
                        .values
                        .iter()
                        .any(|(k, v)| !name(k) || v.len() > 4096 || v.contains('\0'))
                    || variant.stdin_text.is_some() && variant.stdin_file.is_some()
                {
                    return Err("Invalid or duplicate matrix row".into());
                }
                let mut row = case.clone();
                row.id = format!("{}--matrix-{}", case.id, variant.id);
                if let Some(args) = variant.args {
                    row.args = args;
                }
                for (k, v) in variant.env {
                    row.env
                        .retain(|old, _| !crate::inputs::ordinal_cmp(old, &k).is_eq());
                    row.env.insert(k, v);
                }
                if let Some(text) = variant.stdin_text {
                    row.stdin_file = None;
                    row.stdin_text = Some(text);
                }
                if let Some(file) = variant.stdin_file {
                    row.stdin_text = None;
                    row.stdin_file = Some(file);
                }
                if let Some(dir) = variant.fixture_dir {
                    row.fixture_dir = Some(dir);
                }
                strings(&mut row, "matrix", &variant.values, true)?;
                row_bytes += serde_json::to_vec(&row).map_err(|e| e.to_string())?.len();
                if row_bytes > 16 * 1024 * 1024 {
                    return Err("Expanded matrix configuration exceeds 16 MiB".into());
                }
                rows.push(row);
            }
        }
        let mut ids = Vec::new();
        for row in rows {
            for n in 1..=count {
                let mut attempt = row.clone();
                attempt.repeat = 1;
                if count > 1 {
                    attempt.id = format!("{}--repeat-{n:03}", row.id);
                    attempt.attempt = Some(Attempt {
                        group: row.id.clone(),
                        number: n,
                        total: count,
                    });
                }
                if attempt.id != source {
                    attempt.origin = Some(source.clone());
                }
                if !name(&attempt.id) || expanded.len() >= suite.limits.max_cases {
                    return Err("Expanded matrix/repeat IDs or case count exceed limits".into());
                }
                compiled_bytes += serde_json::to_vec(&attempt)
                    .map_err(|e| e.to_string())?
                    .len();
                if compiled_bytes > 16 * 1024 * 1024 {
                    return Err("Expanded workflow configuration exceeds 16 MiB".into());
                }
                ids.push(attempt.id.clone());
                expanded.push(attempt);
            }
        }
        if mappings.insert(source, ids).is_some() {
            return Err("Duplicate source case ID".into());
        }
    }
    for contract in &mut suite.contracts {
        let mut ids = Vec::new();
        for id in &contract.cases {
            ids.extend(
                mappings
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| vec![id.clone()]),
            );
        }
        contract.cases = ids;
    }
    suite.cases = expanded;
    Ok(())
}
pub fn validate_scenario(case: &Case) -> Result<(), String> {
    if case.steps.is_empty() {
        if !case.extract.is_empty() || case.continue_on_failure {
            return Err("Extraction and scenario policy require steps".into());
        }
        let mut checked = case.clone();
        strings(&mut checked, "steps", &BTreeMap::new(), false)?;
        return Ok(());
    }
    if case.steps.len() > 32
        || !case.args.is_empty()
        || case.expect.exit_code != 0
        || case.stdout.is_some()
        || case.stderr.is_some()
        || case.stdin_text.is_some()
        || case.stdin_file.is_some()
        || case.http.is_some()
        || !case.files.is_empty()
        || !case.extract.is_empty()
    {
        return Err("Scenario parent must contain only workspace/environment, limits and steps; args=[] and expect.exit_code=0".into());
    }
    let mut ids = BTreeSet::new();
    let mut bindings = BTreeMap::new();
    for step in &case.steps {
        if !name(&step.id)
            || !ids.insert(&step.id)
            || !step.steps.is_empty()
            || !step.matrix.is_empty()
            || step.repeat != 1
            || step.attempt.is_some()
            || step.origin.is_some()
            || step.fixture_dir.is_some()
            || !step.inherit_env.is_empty()
            || step.continue_on_failure
            || step.extract.len() > 32
        {
            return Err("Invalid scenario step; nested workflows or step fixtures/inheritance are unsupported".into());
        }
        let mut checked = step.clone();
        strings(&mut checked, "steps", &bindings, false)?;
        for (key, extraction) in &step.extract {
            if !name(key)
                || bindings.contains_key(key)
                || bindings.len() >= 32
                || extraction.pointer.len() > 4096
                || !crate::strict_json::valid_pointer(&extraction.pointer)
            {
                return Err("Invalid, duplicate or oversized extraction".into());
            }
            bindings.insert(key.clone(), "placeholder".into());
        }
    }
    Ok(())
}
pub fn bind(case: &Case, values: &BTreeMap<String, String>) -> Result<Case, String> {
    let mut bound = case.clone();
    strings(&mut bound, "steps", values, false)?;
    if bound
        .args
        .iter()
        .chain(bound.env.values())
        .any(|s| s.contains('\0'))
        || bound
            .stdin_text
            .as_ref()
            .is_some_and(|s| s.len() > 1024 * 1024)
    {
        return Err("Invalid or oversized runtime binding".into());
    }
    Ok(bound)
}
pub fn extract(
    bytes: &[u8],
    fields: &BTreeMap<String, Extraction>,
) -> Result<BTreeMap<String, String>, String> {
    use crate::strict_json::Value;
    let json =
        crate::strict_json::parse(bytes).map_err(|_| "Extraction requires strict JSON stdout")?;
    let mut values = BTreeMap::new();
    for (name, field) in fields {
        let value = json
            .pointer(&field.pointer)
            .ok_or("Extraction pointer is missing")?;
        let text = match (&field.kind, value) {
            (ScalarType::String, Value::String(s)) => s.clone(),
            (ScalarType::Number, Value::Number(n)) => n.to_string(),
            (ScalarType::Boolean, Value::Bool(b)) => b.to_string(),
            (ScalarType::Null, Value::Null) => "null".into(),
            _ => return Err("Extraction type mismatch".into()),
        };
        if text.len() > 4096 || text.contains('\0') {
            return Err("Invalid or oversized extracted scalar".into());
        }
        values.insert(name.clone(), text);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extraction_keeps_exact_numbers_and_enforces_scalar_types() {
        let fields = BTreeMap::from([
            (
                "number".into(),
                Extraction {
                    pointer: "/n".into(),
                    kind: ScalarType::Number,
                },
            ),
            (
                "bool".into(),
                Extraction {
                    pointer: "/b".into(),
                    kind: ScalarType::Boolean,
                },
            ),
            (
                "null".into(),
                Extraction {
                    pointer: "/z".into(),
                    kind: ScalarType::Null,
                },
            ),
            (
                "string".into(),
                Extraction {
                    pointer: "/s".into(),
                    kind: ScalarType::String,
                },
            ),
        ]);
        let values = extract(
            br#"{"n":9007199254740993,"b":true,"z":null,"s":"hello"}"#,
            &fields,
        )
        .unwrap();
        assert_eq!(values["number"], "9007199254740993e0");
        assert_eq!(values["bool"], "true");
        assert_eq!(values["null"], "null");
        assert!(
            extract(
                br#"{"n":"9007199254740993","b":true,"z":null,"s":"hello"}"#,
                &fields
            )
            .is_err()
        );
        assert!(extract(br#"{"n":1,"n":2}"#, &fields).is_err());
    }
    #[test]
    fn substituted_values_are_data_not_recursively_expanded() {
        let mut text = "prefix {{steps.first}} suffix".to_owned();
        let values = BTreeMap::from([
            ("first".into(), "{{steps.second}}".into()),
            ("second".into(), "wrong".into()),
        ]);
        substitute(&mut text, "steps", &values).unwrap();
        assert_eq!(text, "prefix {{steps.second}} suffix");
        assert!(substitute(&mut "{{steps.missing}}".to_owned(), "steps", &values).is_err());
    }
}
