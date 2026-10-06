use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Pilot {
    dir: tempfile::TempDir,
    suite: PathBuf,
    baseline: PathBuf,
    candidate: PathBuf,
}
impl Pilot {
    fn new(old: Value, new: Value, extra: &str) -> Self {
        let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
        let baseline = dir.path().join("baseline.exe");
        let candidate = dir.path().join("candidate.exe");
        for (path, profile) in [(&baseline, old), (&candidate, new)] {
            fs::copy(env!("CARGO_BIN_EXE_spanforge-verify-fixture"), path).unwrap();
            fs::write(
                path.with_extension("profile.json"),
                serde_json::to_vec(&profile).unwrap(),
            )
            .unwrap();
        }
        let suite = dir.path().join("suite.toml");
        fs::write(&suite, format!("schema_version=1\nsuite_id='compare'\nprogram='unused-original.exe'\n{extra}\n[[cases]]\nid='release'\nargs=['comparison-profile']\nexpect={{exit_code=0}}\n")).unwrap();
        Self {
            dir,
            suite,
            baseline,
            candidate,
        }
    }
    fn command(&self, flags: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"));
        command
            .env("CLIVERIFYR_WORK_ROOT", self.dir.path())
            .args(["compare", "--file"])
            .arg(&self.suite)
            .arg("--baseline")
            .arg(&self.baseline)
            .arg("--candidate")
            .arg(&self.candidate)
            .args(flags);
        command
    }
    fn run(&self, flags: &[&str]) -> (Output, Value) {
        let output = self.command(flags).output().unwrap();
        let report =
            serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{:?}", output));
        (output, report)
    }
}
fn difference(report: &Value, field: &str) -> bool {
    report["differences"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["field"] == field)
}

#[test]
fn detects_equal_length_successful_output_and_changed_final_files() {
    let pilot = Pilot::new(
        json!({"stdout":"old", "file":"aaa"}),
        json!({"stdout":"new", "file":"bbb"}),
        "",
    );
    // A native PE/ELF overlay changes binary identity while retaining a runnable
    // test executable; release profiles then provide the changed behavior.
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&pilot.candidate)
        .unwrap()
        .write_all(b"cliverifyr candidate test revision")
        .unwrap();
    let (output, report) = pilot.run(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report["comparison_complete"], true);
    assert_ne!(
        report["baseline"]["target_hash"],
        report["candidate"]["target_hash"]
    );
    assert!(difference(&report, "stdout"));
    assert!(difference(&report, "workspace"));
    assert_ne!(
        report["baseline_evidence"][0]["stdout_sha256"],
        report["candidate_evidence"][0]["stdout_sha256"]
    );
    assert_ne!(
        report["baseline_evidence"][0]["workspace_sha256"],
        report["candidate_evidence"][0]["workspace_sha256"]
    );
    assert!(!pilot.dir.path().join("result.txt").exists());
    // Workspace setup/case scratch roots are removed even when contracts fail.
    assert!(
        !fs::read_dir(pilot.dir.path())
            .unwrap()
            .any(|e| e.unwrap().file_type().unwrap().is_dir())
    );
}

#[test]
fn semantic_json_is_opt_in_and_ignored_fields_retain_raw_hash_evidence() {
    let pilot = Pilot::new(
        json!({"stdout":"{\"n\":1.0,\"time\":\"old\"}"}),
        json!({"stdout":"{\"time\":\"new\",\"n\":1}"}),
        "",
    );
    assert_eq!(pilot.run(&[]).0.status.code(), Some(1));
    let (output, report) = pilot.run(&["--json-stdout", "--ignore-json-pointer", "/time"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report["differences"], json!([]));
    assert_ne!(
        report["baseline_evidence"][0]["stdout_sha256"],
        report["candidate_evidence"][0]["stdout_sha256"]
    );
    fs::write(
        pilot.candidate.with_extension("profile.json"),
        serde_json::to_vec(&json!({"stdout":"{\"n\":2,\"time\":\"new\"}"})).unwrap(),
    )
    .unwrap();
    let (output, report) = pilot.run(&["--json-stdout", "--ignore-json-pointer", "/time"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report["differences"][0]["path"], "/n");
}

#[test]
fn both_targets_and_options_validate_before_any_launch() {
    let pilot = Pilot::new(json!({}), json!({}), "");
    let marker = pilot.dir.path().join("launched");
    fs::write(
        pilot.baseline.with_extension("profile.json"),
        serde_json::to_vec(&json!({"launch_marker": marker})).unwrap(),
    )
    .unwrap();
    let invalid = pilot
        .command(&["--ignore-json-pointer", "/time"])
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(!marker.exists());
    fs::remove_file(&pilot.candidate).unwrap();
    let invalid = pilot.command(&[]).output().unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(!marker.exists());
}

#[test]
fn shares_immutable_stdin_even_when_baseline_changes_original_input() {
    let pilot = Pilot::new(json!({"echo_stdin":true}), json!({"echo_stdin":true}), "");
    let input = pilot.dir.path().join("input.txt");
    fs::write(&input, b"original").unwrap();
    fs::write(
        pilot.baseline.with_extension("profile.json"),
        serde_json::to_vec(&json!({"echo_stdin":true,"mutate_source":input})).unwrap(),
    )
    .unwrap();
    let text = fs::read_to_string(&pilot.suite).unwrap();
    fs::write(
        &pilot.suite,
        text.replace(
            "expect={exit_code=0}",
            "stdin_file='input.txt'\nexpect={exit_code=0}",
        ),
    )
    .unwrap();
    let (output, report) = pilot.run(&[]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report["comparison_complete"], true);
    assert_eq!(fs::read(input).unwrap(), b"mutated");
}

#[test]
fn incomplete_baseline_skips_candidate_and_exit_regressions_fail() {
    let pilot = Pilot::new(
        json!({"sleep_ms":1000}),
        json!({}),
        "[defaults]\ntimeout_ms=50",
    );
    let marker = pilot.dir.path().join("candidate-launched");
    fs::write(
        pilot.candidate.with_extension("profile.json"),
        serde_json::to_vec(&json!({"launch_marker":marker})).unwrap(),
    )
    .unwrap();
    let (output, report) = pilot.run(&[]);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(report["comparison_complete"], false);
    assert!(report["candidate"].is_null());
    assert!(!marker.exists());
    let pilot = Pilot::new(json!({}), json!({"exit":7}), "");
    let (output, report) = pilot.run(&[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(difference(&report, "exit_code"));
    assert!(
        report["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["kind"] == "contract_regression")
    );
}

#[test]
fn masks_secret_json_strings_before_escaping_and_compares_before_masking() {
    let secret = "token-\"\\\nvalue";
    let a = serde_json::to_string(&json!({"secret":secret})).unwrap();
    let b = serde_json::to_string(&json!({"secret":format!("{secret}-changed")})).unwrap();
    let pilot = Pilot::new(
        json!({"stdout":a}),
        json!({"stdout":b}),
        "redact_values_env=['CLIVERIFYR_COMPARE_SECRET']",
    );
    let output = pilot
        .command(&["--json-stdout"])
        .env("CLIVERIFYR_COMPARE_SECRET", secret)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["differences"][0]["baseline"]
            .as_str()
            .unwrap()
            .contains("[REDACTED]")
    );
    assert!(!String::from_utf8(output.stdout).unwrap().contains("token-"));
}

#[test]
fn comparison_hash_excludes_target_path_and_ordinary_suite_hash_is_stable() {
    let pilot = Pilot::new(json!({}), json!({}), "");
    let (baseline, candidate) = spanforge_verify::inputs::prepare_comparison(
        &pilot.suite,
        None,
        &pilot.baseline,
        &pilot.candidate,
    )
    .unwrap();
    assert_eq!(baseline.suite_hash(), candidate.suite_hash());
    assert_eq!(baseline.cases()[0].stdin(), candidate.cases()[0].stdin());
    let text = fs::read_to_string(&pilot.suite).unwrap();
    fs::write(
        &pilot.suite,
        text.replace("unused-original.exe", pilot.baseline.to_str().unwrap()),
    )
    .unwrap();
    let plan = spanforge_verify::inputs::prepare(&pilot.suite, None).unwrap();
    use sha2::{Digest, Sha256};
    let expected = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(plan.suite()).unwrap())
    );
    assert_eq!(plan.suite_hash(), expected);
    // Absolute target overrides resolve against the invocation, not fixture cwd.
    assert_eq!(
        baseline.executable().canonicalize().unwrap(),
        pilot.baseline.canonicalize().unwrap()
    );
}

#[test]
fn shares_one_run_deadline_across_both_executables() {
    let pilot = Pilot::new(
        json!({}),
        json!({"sleep_ms":10000}),
        "[limits]\nrun_timeout_ms=2500",
    );
    let (output, report) = pilot.run(&[]);
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(report["comparison_complete"], false);
    assert_eq!(report["baseline"]["status"], "PASS");
    assert_eq!(
        report["candidate"]["cases"][0]["termination_reason"],
        "run_deadline"
    );
}

#[test]
fn failed_private_capture_cleans_workspace_and_aborts_remaining_cases() {
    // Configure the work root in a child test process; never mutate the shared
    // environment of parallel Rust tests.
    if std::env::var_os("CLIVERIFYR_CAPTURE_FAILURE_WORKER").is_none() {
        let root = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "failed_private_capture_cleans_workspace_and_aborts_remaining_cases",
                "--nocapture",
            ])
            .env("CLIVERIFYR_CAPTURE_FAILURE_WORKER", "1")
            .env("CLIVERIFYR_WORK_ROOT", root.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
        return;
    }
    struct Reject;
    impl spanforge_verify::runner::CaseObserver for Reject {
        fn observe(
            &mut self,
            _: usize,
            _: &[u8],
            _: &[u8],
            _: &spanforge_verify::workspace::Snapshot,
            _: &std::path::Path,
        ) -> Result<(), String> {
            Err("injected capture budget failure".into())
        }
    }
    let pilot = Pilot::new(json!({"file":"private"}), json!({}), "");
    let text = fs::read_to_string(&pilot.suite).unwrap();
    fs::write(
        &pilot.suite,
        format!(
            "{}\n[[cases]]\nid='remaining'\nargs=['comparison-profile']\nexpect={{exit_code=0}}\n",
            text.replace("unused-original.exe", pilot.baseline.to_str().unwrap())
        ),
    )
    .unwrap();
    let plan = spanforge_verify::inputs::prepare(&pilot.suite, None).unwrap();
    let result = spanforge_verify::runner::execute_plan_started(
        &plan,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        std::time::Instant::now(),
        spanforge_verify::reports::utc_now(),
        Some(&mut Reject),
    )
    .unwrap();
    assert_eq!(result.exit_code, 3);
    assert_eq!(
        result.cases[0].reason_code.as_deref(),
        Some("comparison_capture_failed")
    );
    assert_eq!(
        result.cases[1].reason_code.as_deref(),
        Some("not_run_after_abort")
    );
    assert!(!pilot.dir.path().join("result.txt").exists());
}

#[test]
fn reviewed_policy_allows_a_change_but_never_weakens_assertions() {
    let pilot = Pilot::new(json!({"stdout":"old"}), json!({"stdout":"new"}), "");
    let policy = pilot.dir.path().join("policy.toml");
    fs::write(&policy,"schema_version=1\nname='release-policy'\n[[rules]]\nid='human-output'\ncase='release'\nfield='stdout'\nimpact='allowed'\nreason='Human-readable wording may change'\n").unwrap();
    let (output, report) = pilot.run(&["--policy", policy.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(report["allowed_differences"], 1);
    assert_eq!(report["blocking_differences"], 0);
    assert_eq!(report["differences"][0]["impact"], "allowed");
    assert_eq!(report["differences"][0]["rule_id"], "human-output");
    assert_eq!(report["policy_name"], "release-policy");
    fs::write(
        &pilot.suite,
        fs::read_to_string(&pilot.suite).unwrap() + "stdout={mode='text_equals',text='old'}\n",
    )
    .unwrap();
    let (output, report) = pilot.run(&["--policy", policy.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(report["breaking_differences"].as_u64().unwrap() > 0);
}

#[test]
fn selected_generated_json_files_have_semantic_pointer_diffs_and_raw_hashes() {
    let pilot = Pilot::new(
        json!({"file":"{\"n\":1.0,\"a\":true}"}),
        json!({"file":"{\"a\":true,\"n\":1}"}),
        "",
    );
    fs::write(
        pilot.dir.path().join("expected.json"),
        "{\"n\":1,\"a\":true}",
    )
    .unwrap();
    fs::write(&pilot.suite,fs::read_to_string(&pilot.suite).unwrap()+"files=[{path='result.txt',kind='file',mode='json_equals_file',expected_file='expected.json'}]\n").unwrap();
    assert_eq!(pilot.run(&[]).0.status.code(), Some(1));
    let (output, report) = pilot.run(&["--json-file", "result.txt"]);
    assert_eq!(output.status.code(), Some(0));
    assert_ne!(
        report["baseline_evidence"][0]["workspace_sha256"],
        report["candidate_evidence"][0]["workspace_sha256"]
    );
    fs::write(
        pilot.candidate.with_extension("profile.json"),
        serde_json::to_vec(&json!({"file":"{\"a\":true,\"n\":2}"})).unwrap(),
    )
    .unwrap();
    let (output, report) = pilot.run(&["--json-file", "result.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        report["differences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["path"] == "result.txt#/n")
    );
    let (output, report) = pilot.run(&["--text-file", "result.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(report["differences"].as_array().unwrap().iter().any(|d| {
        d["field"] == "workspace"
            && d["baseline"]
                .as_str()
                .is_some_and(|s| s.contains("excerpt"))
    }));
}

#[test]
fn invalid_or_unknown_case_policy_is_rejected_before_launch() {
    let pilot = Pilot::new(json!({}), json!({}), "");
    let marker = pilot.dir.path().join("launched");
    fs::write(
        pilot.baseline.with_extension("profile.json"),
        serde_json::to_vec(&json!({"launch_marker":marker})).unwrap(),
    )
    .unwrap();
    let policy = pilot.dir.path().join("policy.toml");
    fs::write(&policy,"schema_version=1\nname='policy'\n[[rules]]\nid='bad'\nfield='assertion'\nimpact='allowed'\nreason='May change'\n").unwrap();
    assert_eq!(
        pilot
            .command(&["--policy", policy.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert!(!marker.exists());
    fs::write(&policy,"schema_version=1\nname='policy'\n[[rules]]\nid='bad'\nfield='stdout'\ncase='missing'\nimpact='breaking'\nreason='Must remain stable'\n").unwrap();
    assert_eq!(
        pilot
            .command(&["--policy", policy.to_str().unwrap()])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert!(!marker.exists());
}

#[test]
fn malformed_candidate_json_is_a_format_regression_not_a_config_error() {
    let pilot = Pilot::new(
        json!({"stdout":"{\"status\":200}"}),
        json!({"stdout":"malformed JSON"}),
        "",
    );
    let policy = pilot.dir.path().join("policy.toml");
    fs::write(&policy,"schema_version=1\nname='policy'\n[comparison]\njson_stdout=true\n[[rules]]\nid='wording'\nfield='stdout'\nimpact='allowed'\nreason='May change wording'\n").unwrap();
    let (output, report) = pilot.run(&["--policy", policy.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(report["comparison_complete"], true);
    assert_eq!(report["differences"][0]["kind"], "format_regression");
    assert_eq!(report["differences"][0]["impact"], "breaking");
    assert_eq!(report["breaking_differences"], 1);
}
