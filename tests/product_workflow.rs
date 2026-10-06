use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};
fn cli(dir: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"));
    cmd.env("CLIVERIFYR_WORK_ROOT", dir);
    cmd
}
fn setup(args: &str, http: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let file = dir.path().join("suite.toml");
    fs::write(&file, format!("schema_version=1\nsuite_id='http'\nprogram='{}'\n[[cases]]\nid='one'\nargs={args}\nexpect={{exit_code=0}}\n{http}", env!("CARGO_BIN_EXE_spanforge-verify-fixture"))).unwrap();
    (dir, file)
}
const HTTP: &str = "[cases.http]\nurl_env='CLIVERIFYR_HTTP_URL'\n[[cases.http.responses]]\nmethod='GET'\npath='/health'\nstatus=503\nbody='offline'\ndelay_ms=10\n";
#[test]
fn new_http_environment_prefix_runs_and_replays() {
    let http = HTTP.replace("CLIVERIFYR_HTTP_URL", "SPANFORGE_VERIFY_HTTP_URL");
    let (dir, file) = setup("['http-get']", &http);
    let (output, _) = run(dir.path(), &file);
    assert!(output.status.success(), "{output:?}");
    let bundle = dir.path().join("new-brand-bundle");
    let output = cli(dir.path())
        .args(["bundle", "--file"])
        .arg(&file)
        .args(["--case", "one", "--out"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let output = cli(dir.path())
        .args(["replay", "--bundle"])
        .arg(&bundle)
        .arg("--program")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}
fn run(dir: &std::path::Path, file: &std::path::Path) -> (Output, Value) {
    let report = dir.join("report.json");
    let output = cli(dir)
        .args(["run", "--file"])
        .arg(file)
        .arg("--json")
        .arg(&report)
        .output()
        .unwrap();
    let value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    (output, value)
}
#[test]
fn native_http_fault_and_replay_preserve_request_assertions() {
    let (dir, file) = setup("['http-get']", HTTP);
    let text = fs::read_to_string(&file).unwrap().replace(
        "[cases.http]",
        "stdout={mode='contains',text='offline'}\n[cases.http]",
    );
    fs::write(&file, text).unwrap();
    let (out, report) = run(dir.path(), &file);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        report["cases"][0]["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["check_id"] == "http_requests" && a["status"] == "PASS")
    );
    let bundle = dir.path().join("bundle");
    let out = cli(dir.path())
        .args(["bundle", "--file"])
        .arg(&file)
        .args(["--case", "one", "--out"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    fs::remove_file(file).unwrap();
    let out = cli(dir.path())
        .args(["replay", "--bundle"])
        .arg(&bundle)
        .arg("--program")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}
#[test]
fn wrong_and_missing_requests_fail_even_with_successful_exit() {
    for args in ["['http-get','/wrong']", "['exit','0']"] {
        let (dir, file) = setup(args, HTTP);
        let (out, report) = run(dir.path(), &file);
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(
            report["cases"][0]["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["check_id"] == "http_requests" && a["status"] == "FAIL")
        );
    }
}
#[test]
fn invalid_http_is_rejected_before_target_launch() {
    let (dir, file) = setup(
        "['touch','marker']",
        &HTTP.replace("status=503", "status=99"),
    );
    let out = cli(dir.path())
        .args(["run", "--file"])
        .arg(file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(!dir.path().join("marker").exists());
}
#[test]
fn onboarding_and_coverage_do_not_hide_unmapped_behaviors() {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let file = dir.path().join("suite.toml");
    let out = cli(dir.path())
        .args(["init", "--file"])
        .arg(&file)
        .arg("--program")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let out = cli(dir.path())
        .args(["coverage", "--file"])
        .arg(&file)
        .arg("--fail-on-unmapped")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["mapped_contracts"], 1);
    assert_eq!(report["unmapped_contracts"][0], "invalid-input");
    let invalid = fs::read_to_string(&file)
        .unwrap()
        .replace("cases = []", "cases = ['missing']");
    fs::write(&file, invalid).unwrap();
    let out = cli(dir.path())
        .args(["coverage", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn comparison_uses_fresh_http_fixtures_without_port_noise() {
    let (dir, file) = setup("['http-get']", HTTP);
    let out = cli(dir.path())
        .args(["compare", "--file"])
        .arg(&file)
        .arg("--baseline")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .arg("--candidate")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["status"], "PASS");
}
#[test]
fn bundle_restricts_declared_contracts_to_selected_case() {
    let (dir, file) = setup("['exit','0']", "");
    let text = fs::read_to_string(&file).unwrap();
    fs::write(&file,format!("{text}\n[[cases]]\nid='two'\nargs=['exit','0']\nexpect={{exit_code=0}}\n[[contracts]]\nid='exits'\ndescription='Successful exits'\ncases=['one','two']\n")).unwrap();
    let bundle = dir.path().join("bundle");
    let out = cli(dir.path())
        .args(["bundle", "--file"])
        .arg(&file)
        .args(["--case", "one", "--out"])
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let suite: toml::Value =
        toml::from_str(&fs::read_to_string(bundle.join("suite.toml")).unwrap()).unwrap();
    assert_eq!(suite["contracts"][0]["cases"].as_array().unwrap().len(), 1);
    let out = cli(dir.path())
        .args(["replay", "--bundle"])
        .arg(&bundle)
        .arg("--program")
        .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn http_url_argument_is_expanded_as_an_argument_without_shell_execution() {
    let (dir, file) = setup("['http-url','{{http.url}}']", HTTP);
    let (out, _) = run(dir.path(), &file);
    assert!(out.status.success(), "{out:?}");
}
