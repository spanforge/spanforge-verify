//! Literal masking precedes excerpt truncation and terminal/XML escaping.
use regex::{Regex, RegexBuilder};
pub const DIAGNOSTIC_BYTES: usize = 4096;
pub fn suite_masker(file: &std::path::Path) -> Result<Masker, String> {
    let values = crate::inputs::read_bounded(file, 1024 * 1024)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
        .and_then(|v| {
            v.get("redact_values_env")
                .and_then(toml::Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
        .iter()
        .filter_map(toml::Value::as_str)
        .filter_map(|name| std::env::var(name).ok())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    Masker::new(values)
}
pub struct Masker {
    pattern: Option<Regex>,
}
pub struct Diagnostic {
    pub text: String,
    pub truncated: bool,
}
impl Masker {
    /// Conservative detection for exporting binary as well as text inputs.
    pub fn contains_bytes(&self, bytes: &[u8]) -> bool {
        self.pattern
            .as_ref()
            .is_some_and(|p| p.is_match(&String::from_utf8_lossy(bytes)))
    }
    pub fn new(values: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut values: Vec<_> = values.into_iter().collect();
        if values.iter().any(String::is_empty) {
            return Err("Empty masking values are forbidden".into());
        }
        values.sort_by_key(|s| std::cmp::Reverse(s.len()));
        values.dedup();
        let pattern = if values.is_empty() {
            None
        } else {
            let pattern = values
                .iter()
                .map(|s| regex::escape(s))
                .collect::<Vec<_>>()
                .join("|");
            Some(
                RegexBuilder::new(&pattern)
                    .size_limit(16 * 1024 * 1024)
                    .build()
                    .map_err(|_| "Masking pattern exceeds supported resources")?,
            )
        };
        Ok(Self { pattern })
    }
    pub fn mask(&self, text: &str) -> String {
        match &self.pattern {
            Some(regex) => regex.replace_all(text, "[REDACTED]").into(),
            None => text.into(),
        }
    }
    pub fn diagnostic(&self, text: &str) -> Diagnostic {
        let mut text = self.mask(text);
        let truncated = text.len() > DIAGNOSTIC_BYTES;
        if truncated {
            let marker = "… [truncated]";
            let mut end = DIAGNOSTIC_BYTES - marker.len();
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str(marker);
        }
        Diagnostic { text, truncated }
    }
}
pub fn terminal_escape(text: &str) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            escaped.push_str(&format!("\\u{{{:04x}}}", c as u32));
        } else {
            escaped.push(c);
        }
    }
    escaped
}
pub fn xml_sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if matches!(c, '\t' | '\n' | '\r')
                || ('\u{20}'..='\u{d7ff}').contains(&c)
                || ('\u{e000}'..='\u{fffd}').contains(&c)
                || c >= '\u{10000}'
            {
                c
            } else {
                '\u{fffd}'
            }
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_masked_before_truncation_with_longest_literal_match() {
        let masker = Masker::new(["abc".into(), "abcdef".into(), "f.*".into()]).unwrap();
        assert_eq!(
            masker.mask("abcdef abc f.* fXX"),
            "[REDACTED] [REDACTED] [REDACTED] fXX"
        );
        let text = format!("{}abcdef", "x".repeat(4093));
        let diagnostic = masker.diagnostic(&text);
        assert!(diagnostic.truncated);
        assert!(diagnostic.text.len() <= DIAGNOSTIC_BYTES);
        assert!(!diagnostic.text.contains("abc"));
    }
    #[test]
    fn utf8_excerpt_and_control_escaping() {
        let masker = Masker::new([]).unwrap();
        let diagnostic = masker.diagnostic(&"😀".repeat(2000));
        assert!(diagnostic.text.len() <= DIAGNOSTIC_BYTES);
        assert!(diagnostic.truncated);
        assert_eq!(terminal_escape("\x1b[31m\r"), "\\u{001b}[31m\\u{000d}");
        assert_eq!(xml_sanitize("a\0b\t\n"), "a�b\t\n");
        assert!(Masker::new([String::new()]).is_err());
    }
}
