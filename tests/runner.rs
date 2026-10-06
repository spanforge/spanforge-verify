use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap()
}
fn suite(dir: &Path, cases: &str) -> std::path::PathBuf {
    let file = dir.join("suite.toml");
    fs::write(
        &file,
        format!(
            "schema_version=1\nsuite_id='integration'\nprogram='{}'\n{cases}",
            env!("CARGO_BIN_EXE_spanforge-verify-fixture")
        ),
    )
    .unwrap();
    file
}
fn run(file: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_WORK_ROOT", file.parent().unwrap())
        .args(["run", "--file"])
        .arg(file)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn real_reports_cover_pass_fail_stdin_and_files() {
    let dir = scratch();
    fs::write(dir.path().join("expected"), b"resumed").unwrap();
    let file = suite(
        dir.path(),
        "[[cases]]\nid='stdin'\nargs=['stdin']\nstdin_text='hello'\nstdout={mode='text_equals',text='hello'}\nexpect={exit_code=0}\n[[cases]]\nid='fail'\nargs=['exit','37']\nexpect={exit_code=0}\n[[cases]]\nid='file'\nargs=['touch','created']\nfiles=[{path='created',kind='file',mode='exact_file',expected_file='expected'}]\nexpect={exit_code=0}\n",
    );
    let json = dir.path().join("report.json");
    let xml = dir.path().join("report.xml");
    let output = run(
        &file,
        &[
            "--json",
            json.to_str().unwrap(),
            "--junit",
            xml.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(1), "{:?}", output.stderr);
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(
        result.cases.iter().map(|c| c.status).collect::<Vec<_>>(),
        vec![
            spanforge_verify::model::Status::Pass,
            spanforge_verify::model::Status::Fail,
            spanforge_verify::model::Status::Pass
        ]
    );
    assert_eq!(result.cases[1].raw_exit_code, Some(37));
    assert!(uuid::Uuid::parse_str(&result.run_id).is_ok());
    assert!(chrono::DateTime::parse_from_rfc3339(&result.started_at).is_ok());
    assert!(!dir.path().join("created").exists());
    let xml_text = fs::read_to_string(xml).unwrap();
    let mut reader = quick_xml::Reader::from_str(&xml_text);
    while !matches!(reader.read_event().unwrap(), quick_xml::events::Event::Eof) {}
}
#[test]
fn timeout_is_failure_and_run_deadline_aborts_remaining_cases() {
    let dir = scratch();
    let file = suite(
        dir.path(),
        "[limits]\nrun_timeout_ms=1200\n[[cases]]\nid='timeout'\nargs=['sleep','60000']\ntimeout_ms=100\nexpect={exit_code=0}\n[[cases]]\nid='deadline'\nargs=['sleep','60000']\nexpect={exit_code=0}\n[[cases]]\nid='skipped'\nargs=['exit','0']\nexpect={exit_code=0}",
    );
    let json = dir.path().join("result.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(4), "{:?}", output.stderr);
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(
        result.cases[0].status,
        spanforge_verify::model::Status::Fail
    );
    assert!(matches!(
        result.cases[0].termination_reason,
        Some(spanforge_verify::model::TerminationReason::Timeout)
    ));
    assert_eq!(
        result.cases[2].reason_code.as_deref(),
        Some("not_run_after_abort")
    );
}
#[test]
fn report_preflight_rejects_input_aliases_and_existing_files_before_launch() {
    let dir = scratch();
    let marker = dir.path().join("marker");
    let file = suite(
        dir.path(),
        &format!(
            "[[cases]]\nid='touch'\nargs=['touch','{}']\nexpect={{exit_code=0}}",
            marker.display()
        ),
    );
    let bytes = fs::read(&file).unwrap();
    let output = run(
        &file,
        &["--json", file.to_str().unwrap(), "--overwrite-reports"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(&file).unwrap(), bytes);
    assert!(!marker.exists());
    let report = dir.path().join("existing.json");
    fs::write(&report, b"keep").unwrap();
    let output = run(&file, &["--json", report.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(&report).unwrap(), b"keep");
    assert!(!marker.exists());
}
#[test]
fn output_overflow_and_undeclared_files_fail() {
    let dir = scratch();
    let file = suite(
        dir.path(),
        "[[cases]]\nid='flood'\nargs=['flood','1025']\nmax_output_bytes=1024\nstdout={mode='not_contains',text='secret'}\nexpect={exit_code=0}\n[[cases]]\nid='undeclared'\nargs=['touch','unexpected']\nexpect={exit_code=0}",
    );
    let json = dir.path().join("report.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{:?}", output.stderr);
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert!(matches!(
        result.cases[0].termination_reason,
        Some(spanforge_verify::model::TerminationReason::OutputLimit)
    ));
    assert_eq!(
        result.cases[0]
            .assertions
            .iter()
            .find(|a| a.check_id == "stdout")
            .unwrap()
            .status,
        spanforge_verify::model::AssertionStatus::NotEvaluated
    );
    assert_eq!(
        result.cases[1].status,
        spanforge_verify::model::Status::Fail
    );
}
#[test]
fn masking_is_applied_to_real_diagnostics() {
    let dir = scratch();
    let file = suite(
        dir.path(),
        "redact_values_env=['CLIVERIFYR_TEST_SECRET']\n[[cases]]\nid='masked'\nargs=['stdin']\nstdin_text='private-value'\nstdout={mode='text_equals',text='private-value-other'}\nexpect={exit_code=0}",
    );
    let json = dir.path().join("report.json");
    let output = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_TEST_SECRET", "private-value")
        .env("CLIVERIFYR_WORK_ROOT", dir.path())
        .args(["run", "--file"])
        .arg(file)
        .arg("--json")
        .arg(&json)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = fs::read_to_string(json).unwrap();
    assert!(!text.contains("private-value"));
    assert!(text.contains("[REDACTED]"));
}
#[test]
fn report_race_fails_without_clobber_and_pair_publication_is_nontransactional() {
    let dir = scratch();
    let first = dir.path().join("first.json");
    let raced = dir.path().join("raced.xml");
    let file = suite(
        dir.path(),
        &format!(
            "[[cases]]\nid='race'\nargs=['touch','{}']\nexpect={{exit_code=0}}",
            raced.display()
        ),
    );
    let output = run(
        &file,
        &[
            "--json",
            first.to_str().unwrap(),
            "--junit",
            raced.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(fs::read(&raced).unwrap(), b"resumed");
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(first).unwrap()).unwrap();
    assert_eq!(result.status, spanforge_verify::model::Status::Pass);
}
#[test]
fn overwrite_rechecks_destination_type_and_hardlink_aliases() {
    let dir = scratch();
    let report = dir.path().join("report.json");
    fs::write(&report, b"old").unwrap();
    let file = suite(
        dir.path(),
        &format!(
            "[[cases]]\nid='race'\nargs=['replace-directory','{}']\nexpect={{exit_code=0}}",
            report.display()
        ),
    );
    let output = run(
        &file,
        &["--json", report.to_str().unwrap(), "--overwrite-reports"],
    );
    assert_eq!(output.status.code(), Some(3));
    assert!(report.is_dir());
    let alias = dir.path().join("alias.json");
    fs::hard_link(&file, &alias).unwrap();
    let output = run(
        &file,
        &["--json", alias.to_str().unwrap(), "--overwrite-reports"],
    );
    assert_eq!(output.status.code(), Some(2));
}
#[test]
fn invalid_config_emits_synthetic_junit_and_infrastructure_setup_errors_exit_three() {
    let dir = scratch();
    let file = suite(
        dir.path(),
        "[[cases]]\nid='one'\nargs=['exit','0']\nexpect={exit_code=0}",
    );
    let json = dir.path().join("config.json");
    let xml = dir.path().join("config.xml");
    let output = run(
        &file,
        &[
            "--case",
            "missing",
            "--json",
            json.to_str().unwrap(),
            "--junit",
            xml.to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert!(result.cases.is_empty());
    assert_eq!(result.status, spanforge_verify::model::Status::ConfigError);
    assert!(
        fs::read_to_string(xml)
            .unwrap()
            .contains("name=\"__run__\"")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_WORK_ROOT", &file)
        .args(["run", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
}
#[test]
fn malformed_toml_still_emits_new_configuration_reports() {
    let dir = scratch();
    let file = dir.path().join("broken.toml");
    fs::write(&file, "schema_version = [\n").unwrap();
    let json = dir.path().join("broken.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(result.status, spanforge_verify::model::Status::ConfigError);
    assert!(result.cases.is_empty());
}
#[test]
fn created_modified_deleted_directory_and_json_file_assertions_use_final_state() {
    let dir = scratch();
    let fixtures = dir.path().join("fixtures");
    fs::create_dir(&fixtures).unwrap();
    fs::write(fixtures.join("modified"), b"old").unwrap();
    fs::write(fixtures.join("deleted"), b"old").unwrap();
    fs::write(fixtures.join("unchanged"), b"keep").unwrap();
    fs::write(dir.path().join("expected.json"), b"1.0").unwrap();
    let file = suite(
        dir.path(),
        "[[cases]]\nid='modified'\nfixture_dir='fixtures'\nargs=['write','modified','1e0']\nfiles=[{path='modified',kind='file',mode='json_equals_file',expected_file='expected.json'}]\nexpect={exit_code=0}\n[[cases]]\nid='deleted'\nfixture_dir='fixtures'\nargs=['remove','deleted']\nfiles=[{path='deleted',kind='absent'}]\nexpect={exit_code=0}\n[[cases]]\nid='directory'\nargs=['mkdir','new']\nfiles=[{path='new',kind='directory'}]\nexpect={exit_code=0}",
    );
    let output = run(&file, &[]);
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    assert_eq!(fs::read(fixtures.join("modified")).unwrap(), b"old");
    assert_eq!(fs::read(fixtures.join("deleted")).unwrap(), b"old");
}
#[test]
fn automatic_parent_allowance_requires_a_directory() {
    let dir = scratch();
    fs::write(dir.path().join("expected"), b"resumed").unwrap();
    let file = suite(
        dir.path(),
        "[[cases]]\nid='file-parent'\nargs=['touch','out']\nfiles=[{path='out/result',kind='file',mode='exact_file',expected_file='expected'}]\nexpect={exit_code=0}\n[[cases]]\nid='directory-parent'\nargs=['mkdir','out/result']\nfiles=[{path='out/result',kind='directory'}]\nexpect={exit_code=0}",
    );
    let json = dir.path().join("parent.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(
        result.cases[0]
            .assertions
            .iter()
            .find(|a| a.check_id == "workspace_changes")
            .unwrap()
            .status,
        spanforge_verify::model::AssertionStatus::Fail
    );
    assert_eq!(
        result.cases[1].status,
        spanforge_verify::model::Status::Pass
    );
}
#[cfg(target_os = "linux")]
#[test]
fn unix_signal_and_escaped_descendants_are_reported_without_fabricated_exit_codes() {
    let dir = scratch();
    let file = suite(
        dir.path(),
        "[[cases]]\nid='signal'\nargs=['signal']\nexpect={exit_code=0}\n[[cases]]\nid='escape'\nargs=['escape']\nexpect={exit_code=0}",
    );
    let json = dir.path().join("linux.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert!(result.cases[0].raw_exit_code.is_none());
    assert!(matches!(
        result.cases[0].termination_reason,
        Some(spanforge_verify::model::TerminationReason::Signal)
    ));
    assert!(matches!(
        result.cases[1].termination_reason,
        Some(spanforge_verify::model::TerminationReason::LingeringDescendants)
    ));
}
#[cfg(target_os = "linux")]
#[test]
fn output_symlinks_abort_and_cleanup_does_not_follow_links() {
    let dir = scratch();
    let sentinel = dir.path().join("sentinel");
    fs::write(&sentinel, b"keep").unwrap();
    let file = suite(
        dir.path(),
        &format!(
            "[[cases]]\nid='link'\nargs=['symlink','{}','link']\nexpect={{exit_code=0}}\n[[cases]]\nid='skipped'\nargs=['exit','0']\nexpect={{exit_code=0}}",
            sentinel.display()
        ),
    );
    let json = dir.path().join("linux.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3));
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(
        result.cases[0].reason_code.as_deref(),
        Some("unsupported_entry")
    );
    assert_eq!(
        result.cases[1].reason_code.as_deref(),
        Some("not_run_after_abort")
    );
    assert_eq!(fs::read(sentinel).unwrap(), b"keep");
}
#[cfg(windows)]
#[test]
fn output_junctions_abort_and_cleanup_preserves_the_external_tree() {
    let dir = scratch();
    let external = dir.path().join("external");
    fs::create_dir(&external).unwrap();
    let sentinel = external.join("sentinel");
    fs::write(&sentinel, b"keep").unwrap();
    let file = suite(
        dir.path(),
        &format!(
            "[[cases]]\nid='junction'\nargs=['junction','{}','link']\nexpect={{exit_code=0}}\n[[cases]]\nid='skipped'\nargs=['exit','0']\nexpect={{exit_code=0}}",
            external.display()
        ),
    );
    let json = dir.path().join("report.json");
    let output = run(&file, &["--json", json.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(3), "{:?}", output.stderr);
    let result: spanforge_verify::model::RunResult =
        serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
    assert_eq!(
        result.cases[0].reason_code.as_deref(),
        Some("unsupported_entry")
    );
    assert_eq!(
        result.cases[1].reason_code.as_deref(),
        Some("not_run_after_abort")
    );
    assert_eq!(fs::read(sentinel).unwrap(), b"keep");
}
#[cfg(target_os = "linux")]
#[test]
fn real_sigint_and_sigterm_cancel_the_cli_and_reap_the_tree() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let dir = scratch();
        let marker = dir.path().join("ready");
        let report = dir.path().join("run.json");
        let file = suite(
            dir.path(),
            &format!(
                "[[cases]]\nid='tree'\nargs=['tree','2','{}']\nexpect={{exit_code=0}}\n[[cases]]\nid='skipped'\nargs=['exit','0']\nexpect={{exit_code=0}}",
                marker.display()
            ),
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
            .env("CLIVERIFYR_WORK_ROOT", dir.path())
            .args(["run", "--file"])
            .arg(file)
            .arg("--json")
            .arg(&report)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(marker.exists(), "Fixture never became ready");
        assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "Cancellation cleanup timed out");
            thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(status.code(), Some(4));
        let result: spanforge_verify::model::RunResult =
            serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert!(matches!(
            result.cases[0].termination_reason,
            Some(spanforge_verify::model::TerminationReason::Cancelled)
        ));
        assert_eq!(
            result.cases[1].reason_code.as_deref(),
            Some("not_run_after_abort")
        );
        for depth in 0..=2 {
            let pid: i32 = fs::read_to_string(marker.with_extension(format!("pid-{depth}")))
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
}
