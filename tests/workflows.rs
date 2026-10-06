use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn cli(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"));
    cmd.env("CLIVERIFYR_WORK_ROOT", dir);
    cmd
}
fn setup(cases: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let suite = dir.path().join("suite.toml");
    fs::write(
        &suite,
        format!(
            "schema_version=1\nsuite_id='workflows'\nprogram='{}'\n{cases}",
            env!("CARGO_BIN_EXE_spanforge-verify-fixture")
        ),
    )
    .unwrap();
    (dir, suite)
}
fn run(dir: &Path, suite: &Path) -> (Output, Value) {
    let path = dir.join("run.json");
    let out = cli(dir)
        .args(["run", "--file"])
        .arg(suite)
        .arg("--json")
        .arg(&path)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    (out, value)
}
const SCENARIO: &str = "[[cases]]\nid='workflow'\nargs=[]\nexpect={exit_code=0}\n[[cases.steps]]\nid='create'\nargs=['write','shared.txt','hello']\nexpect={exit_code=0}\nfiles=[{path='shared.txt',kind='file',mode='exact_file',expected_file='expected.txt'}]\n[[cases.steps]]\nid='extract'\nargs=['stdin']\nstdin_text='{\"path\":\"shared.txt\"}'\nexpect={exit_code=0}\nextract={path={pointer='/path',kind='string'}}\n[[cases.steps]]\nid='consume'\nargs=['read','{{steps.path}}']\nexpect={exit_code=0}\nstdout={mode='text_equals',text='hello'}\n";
#[test]
fn scenario_shares_workspace_extracts_typed_values_and_bundles_all_steps() {
    let (dir, suite) = setup(SCENARIO);
    fs::write(dir.path().join("expected.txt"), "hello").unwrap();
    let (out, report) = run(dir.path(), &suite);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(report["cases"][0]["steps"].as_array().unwrap().len(), 3);
    assert!(
        report["cases"][0]["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["check_id"] == "step:consume/stdout" && a["status"] == "PASS")
    );
    let bundle = dir.path().join("bundle");
    let out = cli(dir.path())
        .args(["bundle", "--file"])
        .arg(&suite)
        .args(["--case", "workflow", "--out"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    fs::remove_file(suite).unwrap();
    fs::remove_file(dir.path().join("expected.txt")).unwrap();
    let out = cli(dir.path())
        .args(["replay", "--bundle"])
        .arg(bundle)
        .arg("--program")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}
#[test]
fn step_failure_stops_dependents_and_continue_policy_preserves_failure() {
    for continue_on_failure in [false, true] {
        let cases = format!(
            "[[cases]]\nid='workflow'\nargs=[]\nexpect={{exit_code=0}}\ncontinue_on_failure={continue_on_failure}\n[[cases.steps]]\nid='fail'\nargs=['exit','7']\nexpect={{exit_code=0}}\n[[cases.steps]]\nid='next'\nargs=['exit','0']\nexpect={{exit_code=0}}\n"
        );
        let (dir, suite) = setup(&cases);
        let (out, report) = run(dir.path(), &suite);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            report["cases"][0]["steps"][1]["status"],
            if continue_on_failure {
                "PASS"
            } else {
                "INCONCLUSIVE"
            }
        );
        assert_eq!(report["cases"][0]["steps"][0]["raw_exit_code"], 7);
    }
}
#[test]
fn extraction_type_failure_stops_even_continue_policy() {
    let cases = SCENARIO
        .replace("args=[]", "args=[]\ncontinue_on_failure=true")
        .replace("kind='string'", "kind='number'");
    let (dir, suite) = setup(&cases);
    fs::write(dir.path().join("expected.txt"), "hello").unwrap();
    let (out, report) = run(dir.path(), &suite);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        report["cases"][0]["steps"][2]["reason_code"],
        "not_run_after_step_failure"
    );
}
#[test]
fn matrix_and_repeats_expand_stably_and_keep_every_attempt() {
    let (dir, suite) = setup(
        "[[contracts]]\nid='unicode'\ndescription='Unicode input'\ncases=['echo']\n[[cases]]\nid='echo'\nargs=['stdin']\nstdin_text='{{matrix.text}}'\nstdout={mode='text_equals',text='{{matrix.text}}'}\nexpect={exit_code=0}\nrepeat=2\n[[cases.matrix]]\nid='ascii'\nvalues={text='hello'}\n[[cases.matrix]]\nid='unicode'\nvalues={text='\u{0ba4}\u{0bae}\u{0bbf}\u{0bb4}\u{0bcd}'}\n",
    );
    let (out, report) = run(dir.path(), &suite);
    assert!(out.status.success(), "{out:?}");
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 4);
    assert_eq!(cases[3]["case_id"], "echo--matrix-unicode--repeat-002");
    let out = cli(dir.path())
        .args(["repeatability", "--report"])
        .arg(dir.path().join("run.json"))
        .arg("--fail-on-problems")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let summary: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(summary["groups"].as_array().unwrap().len(), 2);
    assert_eq!(summary["groups"][0]["classification"], "consistent_pass");
    let out = cli(dir.path())
        .args(["coverage", "--file"])
        .arg(&suite)
        .arg("--fail-on-unmapped")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let out = cli(dir.path())
        .args(["run", "--file"])
        .arg(&suite)
        .args(["--case", "echo--matrix-unicode"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("matrix-ascii"));
}
#[test]
fn passing_retry_never_hides_failure_and_successful_output_variability_is_visible() {
    for (mode, classification, exit) in [("alternating", "flaky", 1), ("pid", "variable", 0)] {
        let (dir, suite) =
            setup("[[cases]]\nid='repeat'\nargs=['pid']\nexpect={exit_code=0}\nrepeat=4\n");
        if mode == "alternating" {
            let text = fs::read_to_string(&suite).unwrap().replace(
                "args=['pid']",
                &format!(
                    "args=['alternating','{}']",
                    dir.path().join("counter").display()
                ),
            );
            fs::write(&suite, text).unwrap();
        }
        let (out, report) = run(dir.path(), &suite);
        assert_eq!(out.status.code(), Some(exit), "{out:?}");
        assert_eq!(report["cases"].as_array().unwrap().len(), 4);
        let out = cli(dir.path())
            .args(["repeatability", "--report"])
            .arg(dir.path().join("run.json"))
            .arg("--fail-on-problems")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let summary: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(summary["groups"][0]["classification"], classification);
    }
}
#[test]
fn invalid_expansion_and_forward_binding_never_launch_target() {
    for cases in [
        "[[cases]]\nid='bad'\nargs=['exit','0']\nexpect={exit_code=0}\nrepeat=101\n",
        "[[cases]]\nid='bad'\nargs=[]\nexpect={exit_code=0}\n[[cases.steps]]\nid='forward'\nargs=['argv','{{steps.missing}}']\nexpect={exit_code=0}\n",
        "[[cases]]\nid='bad'\nargs=['argv','{{matrix.missing}}']\nexpect={exit_code=0}\n[[cases.matrix]]\nid='one'\nvalues={}\n",
    ] {
        let (dir, suite) = setup(cases);
        let out = cli(dir.path())
            .args(["validate", "--file"])
            .arg(suite)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{out:?}");
    }
}

#[test]
fn nested_failure_diagnostics_and_attempt_group_names_are_masked() {
    let secret = "private-workflow-token";
    let cases = format!(
        "redact_values_env=['WF_SECRET']\n[[cases]]\nid='{secret}'\nargs=[]\nexpect={{exit_code=0}}\nrepeat=2\n[[cases.steps]]\nid='env'\nargs=['env','WF_SECRET']\nenv={{WF_SECRET='{secret}'}}\nexpect={{exit_code=0}}\nstdout={{mode='text_equals',text='different'}}\n"
    );
    let (dir, suite) = setup(&cases);
    let report = dir.path().join("run.json");
    let out = cli(dir.path())
        .env("WF_SECRET", secret)
        .args(["run", "--file"])
        .arg(&suite)
        .arg("--json")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&out.stderr).contains(secret));
    assert!(!fs::read_to_string(&report).unwrap().contains(secret));
    let out = cli(dir.path())
        .args(["repeatability", "--report"])
        .arg(report)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains(secret));
}
#[test]
fn scenario_secret_environment_is_required_but_not_stored_in_bundle() {
    let secret = "bundle-step-secret";
    let (dir, suite) = setup(&format!(
        "redact_values_env=['WF_SECRET']\n[[cases]]\nid='workflow'\nargs=[]\nexpect={{exit_code=0}}\n[[cases.steps]]\nid='env'\nargs=['env','WF_SECRET']\nenv={{WF_SECRET='{secret}'}}\nexpect={{exit_code=0}}\n"
    ));
    let bundle = dir.path().join("bundle");
    let out = cli(dir.path())
        .env("WF_SECRET", secret)
        .args(["bundle", "--file"])
        .arg(&suite)
        .args(["--case", "workflow", "--out"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        !fs::read_to_string(bundle.join("suite.toml"))
            .unwrap()
            .contains(secret)
    );
    let out = cli(dir.path())
        .env("WF_SECRET", secret)
        .args(["replay", "--bundle"])
        .arg(bundle)
        .arg("--program")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}
#[test]
fn global_deadline_preserves_unrun_attempts_and_classifies_incomplete() {
    let (dir, suite) = setup(
        "[limits]\nrun_timeout_ms=40\n[[cases]]\nid='deadline'\nargs=['sleep','1000']\nexpect={exit_code=0}\nrepeat=3\n",
    );
    let (out, report) = run(dir.path(), &suite);
    assert_eq!(out.status.code(), Some(4), "{out:?}");
    assert_eq!(report["cases"].as_array().unwrap().len(), 3);
    let out = cli(dir.path())
        .args(["repeatability", "--report"])
        .arg(dir.path().join("run.json"))
        .arg("--fail-on-problems")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let summary: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(summary["groups"][0]["classification"], "incomplete");
}
#[test]
fn exact_scenario_comparison_works_and_semantic_stream_mode_is_rejected() {
    let (dir, suite) = setup(SCENARIO);
    fs::write(dir.path().join("expected.txt"), "hello").unwrap();
    for semantic in [false, true] {
        let mut cmd = cli(dir.path());
        cmd.args(["compare", "--file"])
            .arg(&suite)
            .arg("--baseline")
            .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
            .arg("--candidate")
            .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"));
        if semantic {
            cmd.arg("--json-stdout");
        }
        let out = cmd.output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(if semantic { 2 } else { 0 }),
            "{out:?}"
        );
    }
}
#[test]
fn matrix_validates_every_row_before_selected_execution() {
    let (dir, suite) = setup(
        "[[cases]]\nid='matrix'\nargs=['exit','0']\nexpect={exit_code=0}\n[[cases.matrix]]\nid='good'\n[[cases.matrix]]\nid='bad'\nstdin_file='missing.bin'\n",
    );
    let out = cli(dir.path())
        .args(["run", "--file"])
        .arg(&suite)
        .args(["--case", "matrix--matrix-good"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn matrix_inputs_and_unicode_output_paths_are_validated_and_asserted() {
    let (dir, suite) = setup(
        "[[cases]]\nid='files'\nargs=['write','{{matrix.path}}','{{matrix.text}}']\nexpect={exit_code=0}\nfiles=[{path='{{matrix.path}}',kind='file',mode='exact_file',expected_file='expected.txt'}]\n[[cases.matrix]]\nid='unicode'\nvalues={path='\u{0ba4}\u{0bb0}\u{0bb5}\u{0bc1}.txt',text='hello'}\n",
    );
    fs::write(dir.path().join("expected.txt"), "hello").unwrap();
    let (out, report) = run(dir.path(), &suite);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        report["cases"][0]["workspace_deltas"][0]["path"],
        "\u{0ba4}\u{0bb0}\u{0bb5}\u{0bc1}.txt"
    );
}
#[test]
fn scenario_deadline_keeps_all_steps_and_prevents_later_execution() {
    let (dir, suite) = setup(
        "[limits]\nrun_timeout_ms=1000\n[[cases]]\nid='scenario'\nargs=[]\nexpect={exit_code=0}\n[[cases.steps]]\nid='sleep'\nargs=['sleep','5000']\nexpect={exit_code=0}\n[[cases.steps]]\nid='later'\nargs=['exit','0']\nexpect={exit_code=0}\n",
    );
    let (out, report) = run(dir.path(), &suite);
    assert_eq!(out.status.code(), Some(4), "{out:?}");
    assert_eq!(report["cases"][0]["steps"].as_array().unwrap().len(), 2);
    assert_eq!(
        report["cases"][0]["steps"][1]["reason_code"],
        "not_run_after_abort"
    );
}
#[test]
fn duplicated_attempt_metadata_is_rejected_instead_of_reported_as_stable() {
    let (dir, suite) =
        setup("[[cases]]\nid='repeat'\nargs=['exit','0']\nexpect={exit_code=0}\nrepeat=2\n");
    let (out, mut report) = run(dir.path(), &suite);
    assert!(out.status.success(), "{out:?}");
    report["cases"][1]["attempt"]["number"] = serde_json::json!(1);
    fs::write(
        dir.path().join("tampered.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    let out = cli(dir.path())
        .args(["repeatability", "--report"])
        .arg(dir.path().join("tampered.json"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn selection_does_not_match_unrelated_cases_with_similar_names() {
    let (dir, suite) = setup(
        "[[cases]]\nid='one'\nargs=['exit','0']\nexpect={exit_code=0}\n[[cases]]\nid='one--matrix-unrelated'\nargs=['exit','9']\nexpect={exit_code=0}\n",
    );
    let out = cli(dir.path())
        .args(["run", "--file"])
        .arg(&suite)
        .args(["--case", "one"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("unrelated"));
}
#[test]
fn binary_stdin_datasets_expand_and_use_original_expected_bytes() {
    let (dir, suite) = setup(
        "[[cases]]\nid='datasets'\nargs=['stdin']\nexpect={exit_code=0}\nstdout={mode='exact_file',expected_file='{{matrix.file}}'}\n[[cases.matrix]]\nid='one'\nstdin_file='one.bin'\nvalues={file='one.bin'}\n[[cases.matrix]]\nid='two'\nstdin_file='two.bin'\nvalues={file='two.bin'}\n",
    );
    fs::write(dir.path().join("one.bin"), [0, 255, 13, 10]).unwrap();
    fs::write(dir.path().join("two.bin"), [254, 0, 128]).unwrap();
    let (out, report) = run(dir.path(), &suite);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(report["cases"][0]["stdin"]["written_bytes"], 4);
    assert_eq!(report["cases"][1]["stdin"]["written_bytes"], 3);
}
