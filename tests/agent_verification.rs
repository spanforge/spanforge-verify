use sha2::{Digest, Sha256};
use spanforge_verify::{model::*, runner, schema};
use std::{
    fs,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

fn digest(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}
fn setup(agent: &str, verifier: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap();
    fs::write(dir.path().join("accepted.txt"), "correct\nalternate").unwrap();
    let expected = if agent == "alternate" {
        "alternate"
    } else {
        "correct"
    };
    fs::write(dir.path().join("expected.txt"), expected).unwrap();
    let exe = env!("CARGO_BIN_EXE_spanforge-verify-fixture").replace('\\', "/");
    let suite = format!(
        r#"schema_version=1
suite_id='agent-integration'
program='{exe}'
[[verifiers]]
id='outcome'
schema_version=1
executable={{path='{exe}',sha256='{exe_hash}'}}
args=['verify-task','{verifier}','{{{{dependency.accepted}}}}']
timeout_ms={verifier_timeout}
max_output_bytes=4096
[verifiers.dependencies.accepted]
path='accepted.txt'
sha256='{dependency_hash}'
[[cases]]
id='task'
args=['agent-task','{agent}']
expect={{exit_code=0}}
verify=['outcome']
files=[{{path='answer.txt',kind='file',mode='exact_file',expected_file='expected.txt'}}]
[cases.agent]
schema_version=1
input='Write an accepted answer, then report completion'
[cases.agent.claims]
completed='outcome'
"#,
        exe_hash = digest(Path::new(&exe)),
        dependency_hash = digest(&dir.path().join("accepted.txt")),
        verifier_timeout = if verifier == "timeout" { 100 } else { 5000 }
    );
    let file = dir.path().join("suite.toml");
    fs::write(&file, suite).unwrap();
    (dir, file)
}
fn execute(file: &Path) -> RunResult {
    runner::execute(file, None, Arc::new(AtomicBool::new(false))).unwrap()
}
#[test]
fn independently_verifies_success_false_completion_and_alternate_outcome() {
    for agent in ["good", "alternate", "wrong", "noop"] {
        let (_dir, file) = setup(agent, "check");
        let result = execute(&file);
        assert_eq!(
            result.status,
            if matches!(agent, "good" | "alternate") {
                Status::Pass
            } else {
                Status::Fail
            }
        );
        let case = &result.cases[0];
        assert_eq!(
            case.assertions
                .iter()
                .find(|a| a.check_id == "agent_protocol")
                .unwrap()
                .status,
            AssertionStatus::Pass
        );
        let claim = case
            .assertions
            .iter()
            .find(|a| a.check_id == "claim:completed")
            .unwrap();
        assert_eq!(
            claim.status,
            if matches!(agent, "good" | "alternate") {
                AssertionStatus::Pass
            } else {
                AssertionStatus::Fail
            }
        );
        if claim.status == AssertionStatus::Fail {
            assert_eq!(claim.reason_code.as_deref(), Some("false_completion_claim"));
        }
    }
}
#[test]
fn evaluator_faults_are_inconclusive_and_preserve_agent_findings() {
    for verifier in [
        "crash",
        "malformed",
        "unknown",
        "timeout",
        "overflow",
        "mutate",
    ] {
        let (_dir, file) = setup("good", verifier);
        let result = execute(&file);
        assert_eq!(result.status, Status::Inconclusive, "{verifier}");
        let case = &result.cases[0];
        let check = case
            .assertions
            .iter()
            .find(|a| a.check_id == "verifier:outcome")
            .unwrap();
        assert_eq!(check.status, AssertionStatus::NotEvaluated, "{verifier}");
        assert_eq!(
            check.reason_code.as_deref(),
            Some(if verifier == "unknown" {
                "verifier_inconclusive"
            } else {
                "evaluator_error"
            })
        );
        assert_eq!(case.raw_exit_code, Some(0));
    }
}
#[test]
fn invalid_unselected_contracts_and_changed_pins_never_launch() {
    for replacement in [
        ("verify=['outcome']", "verify=['missing']"),
        ("schema_version=1\ninput=", "schema_version=2\ninput="),
        ("completed='outcome'", "completed='missing'"),
        ("id='task'", "id='task'\nstdin_text='conflict'"),
        ("max_output_bytes=4096", "max_output_bytes=0"),
    ] {
        let (_dir, file) = setup("good", "check");
        let text = fs::read_to_string(&file)
            .unwrap()
            .replace(replacement.0, replacement.1);
        fs::write(
            &file,
            format!("{text}\n[[cases]]\nid='other'\nargs=[]\nexpect={{exit_code=0}}\n"),
        )
        .unwrap();
        assert!(
            schema::validate(&file, Some("other")).is_err(),
            "{replacement:?}"
        );
    }
    let (dir, file) = setup("good", "check");
    fs::write(dir.path().join("accepted.txt"), "wrong").unwrap();
    assert!(
        schema::validate(&file, None)
            .unwrap_err()
            .contains("SHA256")
    );
}
#[test]
fn forged_identity_fails_and_unmapped_claim_stays_unverified() {
    let (_dir, file) = setup("forge", "check");
    let result = execute(&file);
    assert_eq!(result.status, Status::Fail);
    assert!(
        result.cases[0]
            .assertions
            .iter()
            .any(|a| a.reason_code.as_deref() == Some("invalid_agent_response"))
    );
    let (_dir, file) = setup("good", "check");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("completed='outcome'", "");
    fs::write(&file, text).unwrap();
    let result = execute(&file);
    assert_eq!(result.status, Status::Pass);
    let claim = result.cases[0]
        .assertions
        .iter()
        .find(|a| a.check_id == "claim:completed")
        .unwrap();
    assert_eq!(claim.status, AssertionStatus::NotEvaluated);
    assert_eq!(claim.reason_code.as_deref(), Some("unverified_claim"));
}
#[test]
fn verifier_bundles_fail_explicitly_instead_of_losing_dependencies() {
    let (dir, file) = setup("good", "check");
    let error =
        spanforge_verify::reproduction::create(&file, "task", &dir.path().join("capsule"), false)
            .unwrap_err();
    assert_eq!(error.0, 2);
    assert!(error.1.contains("not supported"));
    assert!(!dir.path().join("capsule").exists());
}

#[test]
fn agent_protocol_keeps_repeat_and_scenario_identities() {
    for (scenario, jsonl) in [(false, false), (true, false), (false, true), (true, true)] {
        let (_dir, file) = setup("good", "check");
        let mut text = fs::read_to_string(&file).unwrap();
        if scenario {
            text = text
                .replace(
                    "[[cases]]",
                    "[[cases]]\nid='workflow'\nargs=[]\nexpect={exit_code=0}\n[[cases.steps]]",
                )
                .replace("[cases.agent", "[cases.steps.agent");
        } else {
            text = text.replace("id='task'", "id='task'\nrepeat=2");
        }
        fs::write(&file, text).unwrap();
        if jsonl {
            use_jsonl(&file);
        }
        let result = execute(&file);
        assert_eq!(result.status, Status::Pass, "{result:?}");
        assert_eq!(
            if scenario {
                result.cases[0].steps.len()
            } else {
                result.cases.len()
            },
            if scenario { 1 } else { 2 }
        );
    }
}

#[test]
fn pinned_dependency_tampering_cannot_manufacture_a_pass() {
    let (dir, file) = setup("tamper", "check");
    let dependency = dir
        .path()
        .join("accepted.txt")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let text = fs::read_to_string(&file).unwrap().replace(
        "args=['agent-task','tamper']",
        &format!("args=['agent-task','tamper','{dependency}']"),
    );
    fs::write(&file, text).unwrap();
    let result = execute(&file);
    assert!(matches!(result.status, Status::Fail | Status::Inconclusive));
    assert!(
        !result.cases[0]
            .assertions
            .iter()
            .any(|a| a.check_id == "verifier:outcome" && a.status == AssertionStatus::Pass)
    );
    #[cfg(windows)]
    assert_eq!(
        fs::read_to_string(dir.path().join("accepted.txt")).unwrap(),
        "correct\nalternate"
    );
}

#[test]
fn verifier_summaries_are_masked_in_json_junit_and_terminal_output() {
    let (dir, file) = setup("good", "check");
    add_qualification(&file);
    let secret = "verifier-summary-private-value";
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace(
            "suite_id='agent-integration'",
            "suite_id='agent-integration'\nredact_values_env=['AGENT_VERIFIER_TEST_SECRET']",
        )
        .replace(
            "'{{dependency.accepted}}']",
            &format!("'{{{{dependency.accepted}}}}','{secret}']"),
        );
    fs::write(&file, text).unwrap();
    let json = dir.path().join("result.json");
    let junit = dir.path().join("result.xml");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("AGENT_VERIFIER_TEST_SECRET", secret)
        .env("SPANFORGE_VERIFY_WORK_ROOT", dir.path())
        .args(["run", "--file"])
        .arg(&file)
        .arg("--json")
        .arg(&json)
        .arg("--junit")
        .arg(&junit)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    for bytes in [
        &output.stdout,
        &output.stderr,
        &fs::read(&json).unwrap(),
        &fs::read(&junit).unwrap(),
    ] {
        assert!(!String::from_utf8_lossy(bytes).contains(secret));
    }
    let report: RunResult = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    let check = report.cases[0]
        .assertions
        .iter()
        .find(|a| a.check_id == "verifier:outcome")
        .unwrap();
    assert!(!check.observed_summary.as_ref().unwrap().contains(secret));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("AGENT_VERIFIER_TEST_SECRET", secret)
        .env("SPANFORGE_VERIFY_WORK_ROOT", dir.path())
        .args(["qualify", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    let health: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        health["verifiers"][0]["controls"].as_array().unwrap().len(),
        8
    );
}

#[test]
fn incomplete_target_does_not_launch_verifiers_or_duplicate_missing_checks() {
    let (_dir, file) = setup("good", "check");
    let text = fs::read_to_string(&file).unwrap().replace(
        "args=['agent-task','good']",
        "args=['sleep','60000']\ntimeout_ms=100",
    );
    fs::write(&file, text).unwrap();
    let result = execute(&file);
    let case = &result.cases[0];
    assert_eq!(case.status, Status::Fail);
    assert_eq!(case.reason_code.as_deref(), Some("timeout"));
    for id in ["agent_protocol", "verifier:outcome"] {
        assert_eq!(case.unchecked.iter().filter(|s| *s == id).count(), 1);
        let check = case.assertions.iter().find(|a| a.check_id == id).unwrap();
        assert_eq!(check.status, AssertionStatus::NotEvaluated);
        assert_eq!(check.reason_code.as_deref(), Some("process_terminated"));
    }
}

fn add_qualification(file: &Path) {
    let text = fs::read_to_string(file).unwrap().replace(
        "[[cases]]",
        r#"
[verifiers.qualification]
schema_version=1
repeat=2
[[verifiers.qualification.controls]]
id='reference'
kind='reference'
files={'answer.txt'='correct'}
[[verifiers.qualification.controls]]
id='no-op'
kind='no_op'
[[verifiers.qualification.controls]]
id='seeded-defect'
kind='defect'
files={'answer.txt'='wrong'}
[[verifiers.qualification.controls]]
id='alternative'
kind='alternate'
files={'answer.txt'='alternate'}
[[cases]]"#,
    );
    fs::write(file, text).unwrap();
}

#[test]
fn qualified_oracle_passes_controls_and_still_rejects_false_completion() {
    for agent in ["good", "alternate", "noop"] {
        let (_dir, file) = setup(agent, "check");
        add_qualification(&file);
        let result = execute(&file);
        assert_eq!(
            result.status,
            if agent == "noop" {
                Status::Fail
            } else {
                Status::Pass
            }
        );
        let checks = &result.cases[0].assertions;
        let health: Vec<_> = checks
            .iter()
            .filter(|a| a.check_id.starts_with("qualification:"))
            .collect();
        assert_eq!(health.len(), 8);
        assert!(health.iter().all(|a| a.status == AssertionStatus::Pass));
        assert!(
            checks
                .iter()
                .find(|a| a.check_id == "verifier:outcome")
                .unwrap()
                .expected_summary
                .as_ref()
                .unwrap()
                .contains("qualified_controls")
        );
    }
}

#[test]
fn broken_oracles_are_quarantined_without_fabricating_agent_failures() {
    for verifier in ["always-pass", "always-fail", "crash", "mutate", "unknown"] {
        let (_dir, file) = setup("good", verifier);
        add_qualification(&file);
        let result = execute(&file);
        let case = &result.cases[0];
        assert_eq!(case.status, Status::Inconclusive, "{verifier}");
        assert_eq!(case.reason_code.as_deref(), Some("evaluator_unqualified"));
        let check = case
            .assertions
            .iter()
            .find(|a| a.check_id == "verifier:outcome")
            .unwrap();
        assert_eq!(check.status, AssertionStatus::NotEvaluated);
        let claim = case
            .assertions
            .iter()
            .find(|a| a.check_id == "claim:completed")
            .unwrap();
        assert_eq!(claim.status, AssertionStatus::NotEvaluated);
        assert!(
            !case
                .assertions
                .iter()
                .any(|a| a.reason_code.as_deref() == Some("false_completion_claim"))
        );
        if verifier == "always-pass" {
            assert_eq!(
                case.assertions
                    .iter()
                    .filter(|a| a.reason_code.as_deref() == Some("qualification_defect_escaped"))
                    .count(),
                4
            );
        }
    }
}

#[test]
fn missing_required_qualification_is_inconclusive() {
    let (_dir, file) = setup("good", "check");
    let text = fs::read_to_string(&file).unwrap().replace(
        "max_output_bytes=4096",
        "max_output_bytes=4096\nrequire_qualification=true",
    );
    fs::write(&file, text).unwrap();
    let result = execute(&file);
    assert_eq!(result.status, Status::Inconclusive);
    assert_eq!(
        result.cases[0].reason_code.as_deref(),
        Some("evaluator_unqualified")
    );
}

#[test]
fn qualification_validates_controls_before_selection() {
    for (from, to) in [
        ("repeat=2", "repeat=1"),
        ("repeat=2", "repeat=6"),
        ("kind='alternate'", "kind='reference'"),
        ("id='alternative'", "id='reference'"),
        ("'answer.txt'='wrong'", "'answer.txt'='correct'"),
        ("'answer.txt'='wrong'", "'../escape'='wrong'"),
        (
            "'answer.txt'='wrong'",
            "'answer.txt'='wrong','answer.txt/nested'='x'",
        ),
        ("kind='defect'", "kind='unknown'"),
    ] {
        let (_dir, file) = setup("good", "check");
        add_qualification(&file);
        let text = fs::read_to_string(&file).unwrap().replace(from, to);
        fs::write(&file, text).unwrap();
        assert!(
            schema::validate(&file, Some("task")).is_err(),
            "{from} -> {to}"
        );
    }
}

#[test]
fn oracle_health_cli_runs_controls_without_launching_agent() {
    for verifier in ["check", "always-pass", "unknown"] {
        let (dir, file) = setup("good", verifier);
        add_qualification(&file);
        let marker = dir
            .path()
            .join("agent-launched")
            .to_str()
            .unwrap()
            .replace('\\', "/");
        let text = fs::read_to_string(&file).unwrap().replace(
            "args=['agent-task','good']",
            &format!("args=['touch','{marker}']"),
        );
        fs::write(&file, text).unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
            .env("SPANFORGE_VERIFY_WORK_ROOT", dir.path())
            .args(["qualify", "--file"])
            .arg(&file)
            .args(["--verifier", "outcome"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(match verifier {
                "check" => 0,
                "always-pass" => 1,
                _ => 4,
            }),
            "{:?}",
            output.stderr
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            json["verifiers"][0]["controls"].as_array().unwrap().len(),
            8
        );
        assert_eq!(
            json["verifiers"][0]["executable_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(
            json["verifiers"][0]["declaration_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        assert!(!dir.path().join("agent-launched").exists());
    }
}

#[test]
fn qualification_cache_is_per_run_and_repeated_tasks_keep_receipts() {
    let (dir, file) = setup("good", "count-check");
    add_qualification(&file);
    let counter = dir
        .path()
        .join("count")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace(
            "'{{dependency.accepted}}']",
            &format!("'{{{{dependency.accepted}}}}','{counter}']"),
        )
        .replace("id='task'", "id='task'\nrepeat=2");
    fs::write(&file, text).unwrap();
    let plan = spanforge_verify::inputs::prepare(&file, None).unwrap();
    for expected in [10, 20] {
        let report = runner::execute_plan_started(
            &plan,
            Arc::new(AtomicBool::new(false)),
            std::time::Instant::now(),
            spanforge_verify::reports::utc_now(),
            None,
        )
        .unwrap();
        assert_eq!(report.status, Status::Pass);
        assert_eq!(fs::read_to_string(&counter).unwrap(), expected.to_string());
        assert!(report.cases.iter().all(|c| {
            c.assertions
                .iter()
                .filter(|a| a.check_id.starts_with("qualification:"))
                .count()
                == 8
        }));
    }
}

#[test]
fn inconsistent_verdicts_cannot_qualify() {
    let (dir, file) = setup("good", "flaky");
    add_qualification(&file);
    let counter = dir
        .path()
        .join("count")
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let text = fs::read_to_string(&file).unwrap().replace(
        "'{{dependency.accepted}}']",
        &format!("'{{{{dependency.accepted}}}}','{counter}']"),
    );
    fs::write(&file, text).unwrap();
    let report = execute(&file);
    assert_eq!(report.status, Status::Inconclusive);
    assert_eq!(fs::read_to_string(counter).unwrap(), "8");
    let checks = &report.cases[0].assertions;
    assert_eq!(
        checks
            .iter()
            .find(|a| a.check_id == "qualification:outcome:reference:001")
            .unwrap()
            .status,
        AssertionStatus::Pass
    );
    assert_eq!(
        checks
            .iter()
            .find(|a| a.check_id == "qualification:outcome:reference:002")
            .unwrap()
            .status,
        AssertionStatus::Fail
    );
}

fn use_jsonl(file: &Path) {
    let text = fs::read_to_string(file).unwrap().replace(
        "schema_version=1\ninput=",
        "schema_version=1\nprotocol='jsonl'\ninput=",
    );
    fs::write(file, text).unwrap();
}

#[test]
fn jsonl_preserves_independent_outcomes_and_explicit_self_report_provenance() {
    for agent in ["good", "alternate", "noop"] {
        let (_dir, file) = setup(agent, "check");
        use_jsonl(&file);
        let report = execute(&file);
        assert_eq!(
            report.status,
            if agent == "noop" {
                Status::Fail
            } else {
                Status::Pass
            }
        );
        let case = &report.cases[0];
        assert_eq!(
            case.assertions
                .iter()
                .filter(|a| a.check_id.starts_with("event:"))
                .count(),
            3
        );
        assert!(
            case.assertions
                .iter()
                .filter(|a| a.check_id.starts_with("event:"))
                .all(|a| a
                    .expected_summary
                    .as_deref()
                    .unwrap()
                    .contains("agent_self_reported"))
        );
        if agent == "noop" {
            assert!(
                case.assertions
                    .iter()
                    .any(|a| a.reason_code.as_deref() == Some("false_completion_claim"))
            );
        }
    }
}

#[test]
fn invalid_jsonl_cannot_pass_or_launder_a_gateway_event() {
    for agent in [
        "event-gap",
        "event-duplicate",
        "event-spoof",
        "event-no-final",
        "event-after-final",
    ] {
        let (_dir, file) = setup(agent, "check");
        use_jsonl(&file);
        let report = execute(&file);
        assert_eq!(report.status, Status::Fail);
        let protocol = report.cases[0]
            .assertions
            .iter()
            .find(|a| a.check_id == "agent_protocol")
            .unwrap();
        assert_eq!(protocol.status, AssertionStatus::Fail, "{agent}");
        assert_eq!(
            protocol.reason_code.as_deref(),
            Some(if agent == "event-no-final" {
                "agent_final_missing"
            } else {
                "invalid_agent_event"
            })
        );
        assert!(
            !report.cases[0]
                .assertions
                .iter()
                .any(|a| a.check_id.starts_with("claim:"))
        );
    }
}

#[test]
fn interrupted_jsonl_retains_only_valid_framed_prefix() {
    let (_dir, file) = setup("event-timeout", "check");
    use_jsonl(&file);
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("id='task'", "id='task'\ntimeout_ms=300");
    fs::write(&file, text).unwrap();
    let report = execute(&file);
    let case = &report.cases[0];
    assert_eq!(case.status, Status::Fail);
    assert_eq!(case.reason_code.as_deref(), Some("timeout"));
    let protocol = case
        .assertions
        .iter()
        .find(|a| a.check_id == "agent_protocol")
        .unwrap();
    assert_eq!(protocol.status, AssertionStatus::NotEvaluated);
    assert_eq!(
        case.assertions
            .iter()
            .filter(|a| a.check_id.starts_with("event:"))
            .count(),
        1
    );
    assert_eq!(
        case.unchecked
            .iter()
            .filter(|id| *id == "agent_protocol")
            .count(),
        1
    );
}

#[test]
fn jsonl_limits_and_versions_validate_before_launch() {
    for config in [
        "protocol='unknown'",
        "protocol='jsonl'\nmax_events=1",
        "protocol='jsonl'\nmax_events=1025",
        "max_events=2",
    ] {
        let (_dir, file) = setup("good", "check");
        let text = fs::read_to_string(&file).unwrap().replace(
            "schema_version=1\ninput=",
            &format!("schema_version=1\n{config}\ninput="),
        );
        fs::write(&file, text).unwrap();
        assert!(schema::validate(&file, None).is_err(), "{config}");
    }
    let (_dir, file) = setup("good", "check");
    use_jsonl(&file);
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("protocol='jsonl'", "protocol='jsonl'\nmax_events=2");
    fs::write(&file, text).unwrap();
    let report = execute(&file);
    assert!(
        report.cases[0]
            .assertions
            .iter()
            .any(|a| a.reason_code.as_deref() == Some("agent_event_limit"))
    );
}

#[test]
fn cancelled_jsonl_preserves_framed_events_and_inconclusive_status() {
    let (dir, file) = setup("event-cancel", "check");
    use_jsonl(&file);
    let ready = dir.path().join("ready");
    let path = ready.to_str().unwrap().replace('\\', "/");
    let text = fs::read_to_string(&file).unwrap().replace(
        "args=['agent-task','event-cancel']",
        &format!("args=['agent-task','event-cancel','{path}']"),
    );
    fs::write(&file, text).unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let signal = flag.clone();
    let thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        while !ready.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready.exists(), "Fixture did not become ready");
        signal.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let result = runner::execute(&file, None, flag).unwrap();
    thread.join().unwrap();
    let case = &result.cases[0];
    assert_eq!(case.status, Status::Inconclusive);
    assert_eq!(case.reason_code.as_deref(), Some("cancelled"));
    assert_eq!(
        case.assertions
            .iter()
            .filter(|a| a.check_id.starts_with("event:"))
            .count(),
        1
    );
    assert_eq!(
        case.unchecked
            .iter()
            .filter(|s| *s == "agent_protocol")
            .count(),
        1
    );
    assert!(!case.checked.iter().any(|s| s == "verifier:outcome"));
}

#[test]
fn jsonl_event_summaries_are_masked_in_publication() {
    let (dir, file) = setup("good", "check");
    use_jsonl(&file);
    let secret = "private-event-summary";
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace(
            "suite_id='agent-integration'",
            "suite_id='agent-integration'\nredact_values_env=['JSONL_TEST_SECRET']",
        )
        .replace(
            "args=['agent-task','good']",
            &format!("args=['agent-task','good','{secret}']"),
        );
    fs::write(&file, text).unwrap();
    let json = dir.path().join("result.json");
    let xml = dir.path().join("result.xml");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("JSONL_TEST_SECRET", secret)
        .env("SPANFORGE_VERIFY_WORK_ROOT", dir.path())
        .args(["run", "--file"])
        .arg(&file)
        .arg("--json")
        .arg(&json)
        .arg("--junit")
        .arg(&xml)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    for bytes in [
        &output.stdout,
        &output.stderr,
        &fs::read(&json).unwrap(),
        &fs::read(&xml).unwrap(),
    ] {
        assert!(!String::from_utf8_lossy(bytes).contains(secret));
    }
    let report: RunResult = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    let event = report.cases[0]
        .assertions
        .iter()
        .find(|a| a.check_id == "event:1:tool-call")
        .unwrap();
    assert!(
        event
            .observed_summary
            .as_deref()
            .unwrap()
            .contains("summary=")
    );
    assert!(!event.observed_summary.as_deref().unwrap().contains(secret));
}
