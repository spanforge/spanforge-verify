use crate::{
    model::{AssertionResult, AssertionStatus},
    schema::StreamRule,
    strict_json,
};

fn result(
    id: &str,
    ok: bool,
    reason: &str,
    expected: Option<String>,
    observed: Option<String>,
) -> AssertionResult {
    AssertionResult {
        check_id: id.into(),
        status: if ok {
            AssertionStatus::Pass
        } else {
            AssertionStatus::Fail
        },
        reason_code: if ok { None } else { Some(reason.into()) },
        expected_summary: expected,
        observed_summary: observed,
    }
}
fn json_error(id: &str, error: strict_json::Error) -> AssertionResult {
    let reason = if matches!(error, strict_json::Error::Limit(_)) {
        "assertion_input_limit"
    } else {
        "invalid_json"
    };
    result(id, false, reason, None, Some(error.to_string()))
}
pub fn evaluate_stream(
    id: &str,
    rule: &StreamRule,
    observed: &[u8],
    complete: bool,
    expected: impl Fn(&str) -> Option<Vec<u8>>,
) -> AssertionResult {
    if !complete {
        return AssertionResult {
            check_id: id.into(),
            status: AssertionStatus::NotEvaluated,
            reason_code: Some("incomplete_capture".into()),
            expected_summary: None,
            observed_summary: None,
        };
    }
    match rule {
        StreamRule::ExactFile { expected_file } => match expected(expected_file) {
            Some(bytes) => result(
                id,
                bytes == observed,
                "assertion_mismatch",
                Some(format!("{} exact bytes", bytes.len())),
                Some(format!("{} observed bytes", observed.len())),
            ),
            None => result(id, false, "missing_snapshot", None, None),
        },
        StreamRule::JsonEqualsFile { expected_file } => {
            let actual = match strict_json::parse(observed) {
                Ok(value) => value,
                Err(error) => return json_error(id, error),
            };
            let approved = match expected(expected_file).and_then(|b| strict_json::parse(&b).ok()) {
                Some(value) => value,
                None => return result(id, false, "missing_snapshot", None, None),
            };
            result(
                id,
                actual == approved,
                "assertion_mismatch",
                Some("approved JSON value".into()),
                Some("observed JSON value".into()),
            )
        }
        StreamRule::JsonPointers { values } => {
            let actual = match strict_json::parse(observed) {
                Ok(value) => value,
                Err(error) => return json_error(id, error),
            };
            for (pointer, literal) in values {
                let Some(value) = actual.pointer(pointer) else {
                    return result(
                        id,
                        false,
                        "pointer_missing",
                        Some(format!("pointer {pointer}")),
                        Some("missing".into()),
                    );
                };
                if strict_json::parse(literal.as_bytes()).as_ref() != Ok(value) {
                    return result(
                        id,
                        false,
                        "assertion_mismatch",
                        Some(format!("pointer {pointer} = {literal}")),
                        Some("different JSON value".into()),
                    );
                }
            }
            result(id, true, "", None, None)
        }
        _ => {
            let text = match std::str::from_utf8(observed) {
                Ok(text) => text,
                Err(_) => return result(id, false, "invalid_encoding", Some("UTF-8".into()), None),
            };
            let (normalization, payload) = match rule {
                StreamRule::TextEquals { text, normalize }
                | StreamRule::Contains { text, normalize }
                | StreamRule::NotContains { text, normalize } => (normalize, text),
                StreamRule::Regex { pattern, normalize } => (normalize, pattern),
                _ => unreachable!(),
            };
            let text = if normalization.is_empty() {
                text.into()
            } else {
                text.replace("\r\n", "\n")
            };
            let ok = match rule {
                StreamRule::TextEquals { .. } => text == *payload,
                StreamRule::Contains { .. } => text.contains(payload),
                StreamRule::NotContains { .. } => !text.contains(payload),
                StreamRule::Regex { .. } => {
                    regex::Regex::new(payload).is_ok_and(|re| re.is_match(&text))
                }
                _ => unreachable!(),
            };
            result(
                id,
                ok,
                "assertion_mismatch",
                Some(format!("{payload}; normalize={normalization:?}")),
                Some(text),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_is_explicit_and_preserves_lone_cr() {
        let rule = StreamRule::TextEquals {
            text: "a\nb\r".into(),
            normalize: vec!["crlf_to_lf".into()],
        };
        assert_eq!(
            evaluate_stream("stdout", &rule, b"a\r\nb\r", true, |_| None).status,
            AssertionStatus::Pass
        );
        let rule = StreamRule::TextEquals {
            text: "a\n".into(),
            normalize: vec![],
        };
        assert_eq!(
            evaluate_stream("stdout", &rule, b"a\r\n", true, |_| None).status,
            AssertionStatus::Fail
        );
    }
    #[test]
    fn incomplete_negative_match_never_passes() {
        let rule = StreamRule::NotContains {
            text: "secret".into(),
            normalize: vec![],
        };
        assert_eq!(
            evaluate_stream("stdout", &rule, b"fine", false, |_| None).status,
            AssertionStatus::NotEvaluated
        );
        assert_eq!(
            evaluate_stream("stdout", &rule, &[255], true, |_| None)
                .reason_code
                .as_deref(),
            Some("invalid_encoding")
        );
    }
    #[test]
    fn json_numeric_and_duplicate_semantics() {
        let rule = StreamRule::JsonEqualsFile {
            expected_file: "expected".into(),
        };
        assert_eq!(
            evaluate_stream("stdout", &rule, b"1e0", true, |_| Some(b"1.0".to_vec())).status,
            AssertionStatus::Pass
        );
        assert_eq!(
            evaluate_stream("stdout", &rule, br#"{"a":1,"a":1}"#, true, |_| Some(
                b"{}".to_vec()
            ))
            .reason_code
            .as_deref(),
            Some("invalid_json")
        );
    }
}
