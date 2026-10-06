//! Versioned JSON tasks. Agent completion text remains self-reported evidence.
use crate::{model::*, schema::Case};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub schema_version: u32,
    pub input: String,
    #[serde(default)]
    pub context: BTreeMap<String, String>,
    #[serde(default)]
    pub claims: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Protocol::is_json")]
    pub protocol: Protocol,
    #[serde(default = "event_limit", skip_serializing_if = "is_event_limit")]
    pub max_events: usize,
}
fn event_limit() -> usize {
    256
}
fn is_event_limit(value: &usize) -> bool {
    *value == event_limit()
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    Json,
    Jsonl,
}
impl Protocol {
    fn is_json(&self) -> bool {
        *self == Self::Json
    }
}
fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn validate(case: &Case) -> Result<(), String> {
    if let Some(t) = &case.agent {
        if t.schema_version != 1
            || !(2..=1024).contains(&t.max_events)
            || (t.protocol == Protocol::Json && t.max_events != event_limit())
            || t.input.is_empty()
            || t.input.len() > 65536
            || t.context.len() > 32
            || t.context.iter().any(|(k, v)| !id(k) || v.len() > 4096)
            || t.claims.len() > 32
            || t.claims
                .iter()
                .any(|(k, v)| !id(k) || !case.verify.contains(v))
        {
            return Err("Invalid agent task version, limits or claim mapping".into());
        }
        if !case.steps.is_empty()
            || case.stdin_text.is_some()
            || case.stdin_file.is_some()
            || !case.extract.is_empty()
        {
            return Err(
                "Agent tasks cannot declare stdin, extraction or parent scenario execution".into(),
            );
        }
    }
    Ok(())
}
pub fn request(t: &Task, run: &str, attempt: &str) -> Vec<u8> {
    let mut request = serde_json::json!({"schema_version":1,"run_id":run,"attempt_id":attempt,"task":{"input":t.input,"context":t.context},"capabilities":{"tool_gateway":false,"events":t.protocol == Protocol::Jsonl}});
    if t.protocol == Protocol::Jsonl {
        request["protocol"] = "jsonl".into();
        request["event_limits"] = serde_json::json!({"max_events":t.max_events,"max_record_bytes":81920,"source":"agent_self_reported"});
    }
    serde_json::to_vec(&request).expect("JSON task serialization")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema_version: u32,
    run_id: String,
    attempt_id: String,
    response: String,
    #[serde(default)]
    claims: Vec<String>,
}
pub fn evaluate(
    t: &Task,
    bytes: &[u8],
    run: &str,
    attempt: &str,
    complete: bool,
    checks: &[AssertionResult],
) -> Vec<AssertionResult> {
    if t.protocol == Protocol::Jsonl {
        return evaluate_jsonl(t, bytes, run, attempt, complete, checks);
    }
    let mut protocol = AssertionResult {
        check_id: "agent_protocol".into(),
        status: AssertionStatus::NotEvaluated,
        reason_code: Some("process_terminated".into()),
        expected_summary: Some(
            "Agent JSON protocol v1 with matching run and attempt identity".into(),
        ),
        observed_summary: None,
    };
    if !complete {
        return vec![protocol];
    }
    let reply = crate::strict_json::parse(bytes)
        .ok()
        .and_then(|_| serde_json::from_slice::<Reply>(bytes).ok());
    let Some(r) = reply.filter(|r| {
        r.schema_version == 1
            && r.run_id == run
            && r.attempt_id == attempt
            && r.response.len() <= 65536
            && r.claims.len() <= 32
            && r.claims.iter().all(|s| id(s))
            && r.claims.iter().collect::<HashSet<_>>().len() == r.claims.len()
    }) else {
        protocol.status = AssertionStatus::Fail;
        protocol.reason_code = Some("invalid_agent_response".into());
        return vec![protocol];
    };
    protocol.status = AssertionStatus::Pass;
    protocol.reason_code = None;
    protocol.observed_summary =
        Some("Final response received; prose remains unverified self-report".into());
    let mut results = vec![protocol];
    for claim in r.claims {
        let check = t.claims.get(&claim).and_then(|id| {
            checks
                .iter()
                .find(|c| c.check_id == format!("verifier:{id}"))
        });
        results.push(AssertionResult {
            check_id: format!("claim:{claim}"),
            status: check
                .map(|c| c.status.clone())
                .unwrap_or(AssertionStatus::NotEvaluated),
            reason_code: match check {
                Some(c) if c.status == AssertionStatus::Fail => {
                    Some("false_completion_claim".into())
                }
                Some(c) => c.reason_code.clone(),
                None => Some("unverified_claim".into()),
            },
            expected_summary: t
                .claims
                .get(&claim)
                .map(|id| format!("Claim mapped to verifier:{id}")),
            observed_summary: None,
        });
    }
    results
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    schema_version: u32,
    run_id: String,
    attempt_id: String,
    sequence: usize,
    event_id: String,
    kind: EventKind,
    name: Option<String>,
    summary: Option<String>,
    response: Option<String>,
    claims: Option<Vec<String>>,
}
#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    Started,
    Tool,
    Model,
    Final,
}

fn evaluate_jsonl(
    t: &Task,
    bytes: &[u8],
    run: &str,
    attempt: &str,
    complete: bool,
    checks: &[AssertionResult],
) -> Vec<AssertionResult> {
    let mut receipts = Vec::new();
    let mut ids = HashSet::new();
    let mut final_reply = None;
    let mut error = None;
    let mut offset = 0;
    for raw in bytes.split_inclusive(|b| *b == b'\n') {
        offset += raw.len();
        // A terminated process may leave a partial final record. Preserve only
        // records that were fully framed, without inventing a completion event.
        if !raw.ends_with(b"\n") {
            if complete {
                error = Some("agent_event_unterminated");
            }
            break;
        }
        let line = raw.strip_suffix(b"\n").unwrap();
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if receipts.len() >= t.max_events {
            error = Some("agent_event_limit");
            break;
        }
        if line.len() > 81920 {
            error = Some("agent_event_record_limit");
            break;
        }
        let event = crate::strict_json::parse(line)
            .ok()
            .and_then(|_| serde_json::from_slice::<Event>(line).ok());
        let Some(event) = event else {
            error = Some("invalid_agent_event");
            break;
        };
        let payload_valid = match event.kind {
            EventKind::Started => {
                receipts.is_empty()
                    && event.name.is_none()
                    && event.response.is_none()
                    && event.claims.is_none()
            }
            EventKind::Tool | EventKind::Model => {
                !receipts.is_empty()
                    && event.name.as_deref().is_some_and(|name| {
                        !name.is_empty() && name.len() <= 128 && !name.contains(['\0', '\r', '\n'])
                    })
                    && event.response.is_none()
                    && event.claims.is_none()
            }
            EventKind::Final => {
                !receipts.is_empty()
                    && event.name.is_none()
                    && event.summary.is_none()
                    && event.response.as_ref().is_some_and(|s| s.len() <= 65536)
                    && event.claims.as_ref().is_none_or(|claims| {
                        claims.len() <= 32
                            && claims.iter().all(|s| id(s))
                            && claims.iter().collect::<HashSet<_>>().len() == claims.len()
                    })
            }
        };
        if event.schema_version != 1
            || event.run_id != run
            || event.attempt_id != attempt
            || event.sequence != receipts.len()
            || !id(&event.event_id)
            || !ids.insert(event.event_id.clone())
            || final_reply.is_some()
            || !payload_valid
            || event.summary.as_ref().is_some_and(|s| s.len() > 4096)
        {
            error = Some("invalid_agent_event");
            break;
        }
        let kind = match event.kind {
            EventKind::Started => "started",
            EventKind::Tool => "tool",
            EventKind::Model => "model",
            EventKind::Final => "final",
        };
        receipts.push(AssertionResult { check_id: format!("event:{}:{}",event.sequence,event.event_id), status: AssertionStatus::Pass, reason_code: None, expected_summary: Some("Valid JSONL envelope; source=agent_self_reported; action execution and completeness unverified".into()), observed_summary: Some(format!("kind={kind}; name={}; summary={}",event.name.as_deref().unwrap_or(""),event.summary.as_deref().unwrap_or(""))) });
        if event.kind == EventKind::Final {
            final_reply = Some(
                serde_json::json!({"schema_version":event.schema_version,"run_id":event.run_id,"attempt_id":event.attempt_id,"response":event.response.unwrap(),"claims":event.claims.unwrap_or_default()}),
            );
        }
    }
    if !complete {
        receipts.insert(
            0,
            AssertionResult {
                check_id: "agent_protocol".into(),
                status: AssertionStatus::NotEvaluated,
                reason_code: Some("process_terminated".into()),
                expected_summary: Some("Complete JSONL stream with final response".into()),
                observed_summary: Some(format!(
                    "Retained {} validated self-reported records; observation completeness unknown",
                    receipts.len()
                )),
            },
        );
        return receipts;
    }
    if error.is_none() && (final_reply.is_none() || offset != bytes.len()) {
        error = Some("agent_final_missing");
    }
    if let Some(reason) = error {
        receipts.insert(
            0,
            AssertionResult {
                check_id: "agent_protocol".into(),
                status: AssertionStatus::Fail,
                reason_code: Some(reason.into()),
                expected_summary: Some(
                    "Ordered JSONL records with unique IDs and exactly one final response".into(),
                ),
                observed_summary: Some(format!(
                    "Stream invalid at record {}; retained prefix is self-reported and incomplete",
                    receipts.len()
                )),
            },
        );
        return receipts;
    }
    let reply = serde_json::to_vec(&final_reply.unwrap()).expect("JSON final serialization");
    let mut json_task = t.clone();
    json_task.protocol = Protocol::Json;
    let mut result = evaluate(&json_task, &reply, run, attempt, true, checks);
    result[0].expected_summary = Some(
        "Validated JSONL stream and final response; action coverage remains self-reported".into(),
    );
    result.extend(receipts);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> Task {
        Task {
            schema_version: 1,
            input: "task".into(),
            context: BTreeMap::new(),
            claims: BTreeMap::new(),
            protocol: Protocol::Jsonl,
            max_events: 256,
        }
    }
    fn event(seq: usize, kind: &str) -> serde_json::Value {
        let mut value = serde_json::json!({"schema_version":1,"run_id":"run","attempt_id":"attempt","sequence":seq,"event_id":format!("event-{seq}"),"kind":kind});
        if kind == "tool" || kind == "model" {
            value["name"] = "operation".into();
        }
        if kind == "final" {
            value["response"] = "Complete".into();
        }
        value
    }
    fn lines(records: &[serde_json::Value]) -> Vec<u8> {
        records
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>()
            .into_bytes()
    }
    #[test]
    fn jsonl_model_events_and_crlf_are_valid_but_always_self_reported() {
        let stream = String::from_utf8(lines(&[
            event(0, "started"),
            event(1, "model"),
            event(2, "final"),
        ]))
        .unwrap()
        .replace('\n', "\r\n");
        let result = evaluate(&task(), stream.as_bytes(), "run", "attempt", true, &[]);
        assert!(result.iter().all(|a| a.status == AssertionStatus::Pass));
        assert!(
            result[0]
                .expected_summary
                .as_deref()
                .unwrap()
                .contains("self-reported")
        );
    }
    #[test]
    fn duplicate_keys_unterminated_records_and_payload_abuse_fail() {
        let start = event(0, "started");
        let final_event = event(1, "final");
        let bytes = lines(&[start.clone(), final_event]);
        let truncated = &bytes[..bytes.len() - 1];
        assert_eq!(
            evaluate(&task(), truncated, "run", "attempt", true, &[])[0]
                .reason_code
                .as_deref(),
            Some("agent_event_unterminated")
        );
        let duplicate = format!("{start}\n{{\"schema_version\":1,\"schema_version\":1}}\n");
        assert_eq!(
            evaluate(&task(), duplicate.as_bytes(), "run", "attempt", true, &[])[0].status,
            AssertionStatus::Fail
        );
        for (key, value) in [
            ("sequence", serde_json::json!(2)),
            ("run_id", serde_json::json!("wrong")),
            ("attempt_id", serde_json::json!("wrong")),
            ("schema_version", serde_json::json!(2)),
            ("source", serde_json::json!("gateway")),
            ("response", serde_json::json!("unexpected")),
        ] {
            let mut tool = event(1, "tool");
            tool[key] = value;
            assert_eq!(
                evaluate(
                    &task(),
                    &lines(&[start.clone(), tool, event(2, "final")]),
                    "run",
                    "attempt",
                    true,
                    &[]
                )[0]
                .status,
                AssertionStatus::Fail,
                "{key}"
            );
        }
    }
    #[test]
    fn oversized_records_are_rejected_and_partial_tail_is_not_invented() {
        let mut huge = event(1, "final");
        huge["response"] = "x".repeat(81920).into();
        assert_eq!(
            evaluate(
                &task(),
                &lines(&[event(0, "started"), huge]),
                "run",
                "attempt",
                true,
                &[]
            )[0]
            .reason_code
            .as_deref(),
            Some("agent_event_record_limit")
        );
        let mut bytes = lines(&[event(0, "started")]);
        bytes.extend_from_slice(b"{partial");
        let result = evaluate(&task(), &bytes, "run", "attempt", false, &[]);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].status, AssertionStatus::NotEvaluated);
        assert!(result[1].check_id.starts_with("event:0:"));
    }
}
