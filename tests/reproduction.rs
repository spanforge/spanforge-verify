use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Pilot {
    dir: tempfile::TempDir,
    suite: PathBuf,
    out: PathBuf,
}
impl Pilot {
    fn new(cases: &str) -> Self {
        let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
        let suite = dir.path().join("original.toml");
        fs::write(
            &suite,
            format!(
                "schema_version=1\nsuite_id='reproduction'\nprogram='{}'\n{cases}",
                env!("CARGO_BIN_EXE_spanforge-verify-fixture")
            ),
        )
        .unwrap();
        let out = dir.path().join("bundle");
        Self { dir, suite, out }
    }
    fn bundle(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"));
        command
            .args(["bundle", "--file"])
            .arg(&self.suite)
            .args(["--case", "one", "--out"])
            .arg(&self.out);
        command
    }
    fn replay(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"));
        command
            .env("CLIVERIFYR_WORK_ROOT", self.dir.path())
            .args(["replay", "--bundle"])
            .arg(&self.out)
            .arg("--program")
            .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"));
        command
    }
}
fn success(output: Output) {
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn bundle_replays_immutable_inputs_after_sources_are_deleted() {
    let pilot = Pilot::new(
        "[[cases]]\nid='one'\nargs=['stdin']\nstdin_file='input.bin'\nexpect={exit_code=0}\nstdout={mode='exact_file',expected_file='expected.bin'}\n",
    );
    fs::write(pilot.dir.path().join("input.bin"), [0, 255, 13, 10]).unwrap();
    fs::write(pilot.dir.path().join("expected.bin"), [0, 255, 13, 10]).unwrap();
    success(pilot.bundle().output().unwrap());
    fs::remove_file(&pilot.suite).unwrap();
    fs::remove_file(pilot.dir.path().join("input.bin")).unwrap();
    fs::remove_file(pilot.dir.path().join("expected.bin")).unwrap();
    let output = pilot.replay().output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["cases"][0]["status"], "PASS");
    assert_eq!(report["cases"][0]["stdin"]["written_bytes"], 4);
}

#[test]
fn exports_fixture_directories_and_file_expectations_as_valid_portable_toml() {
    let pilot = Pilot::new(
        "[[cases]]\nid='one'\nargs=['write','out.json','{\"n\":1}']\nfixture_dir='source'\nexpect={exit_code=0}\nfiles=[{path='out.json',kind='file',mode='json_equals_file',expected_file='expected.json'}]\n",
    );
    fs::create_dir_all(pilot.dir.path().join("source/empty/subdir")).unwrap();
    fs::write(pilot.dir.path().join("source/input.txt"), "fixture").unwrap();
    fs::write(pilot.dir.path().join("expected.json"), "{\"n\":1.0}").unwrap();
    success(pilot.bundle().output().unwrap());
    assert!(pilot.out.join("fixture/empty/subdir").is_dir());
    success(pilot.replay().output().unwrap());
    assert_eq!(
        fs::read(pilot.out.join("fixture/input.txt")).unwrap(),
        b"fixture"
    );
}

#[test]
fn rejects_changed_bundle_bytes_wrong_binary_and_overwrite_before_launch() {
    let pilot = Pilot::new("[[cases]]\nid='one'\nargs=['exit','0']\nexpect={exit_code=0}\n");
    success(pilot.bundle().output().unwrap());
    assert_eq!(pilot.bundle().output().unwrap().status.code(), Some(2));
    let original = fs::read(pilot.out.join("stdin.bin")).unwrap();
    fs::write(pilot.out.join("stdin.bin"), b"tampered").unwrap();
    assert_eq!(pilot.replay().output().unwrap().status.code(), Some(2));
    fs::write(pilot.out.join("stdin.bin"), original).unwrap();
    fs::create_dir(pilot.out.join("unexpected-empty-directory")).unwrap();
    assert_eq!(pilot.replay().output().unwrap().status.code(), Some(2));
    fs::remove_dir(pilot.out.join("unexpected-empty-directory")).unwrap();
    let changed = pilot.dir.path().join("changed.exe");
    fs::copy(env!("CARGO_BIN_EXE_spanforge-verify-fixture"), &changed).unwrap();
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&changed)
        .unwrap()
        .write_all(b"other version")
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["replay", "--bundle"])
        .arg(&pilot.out)
        .arg("--program")
        .arg(changed)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn secret_values_are_required_by_name_and_hash_and_never_exported() {
    let pilot = Pilot::new(
        "redact_values_env=['CLIVERIFYR_BUNDLE_SECRET']\n[[cases]]\nid='one'\nargs=['env','CLIVERIFYR_BUNDLE_SECRET']\ninherit_env=['CLIVERIFYR_BUNDLE_SECRET']\nexpect={exit_code=0}\n",
    );
    let secret = "synthetic-bundle-secret-9342";
    success(
        pilot
            .bundle()
            .env("CLIVERIFYR_BUNDLE_SECRET", secret)
            .output()
            .unwrap(),
    );
    for entry in spanforge_verify::inputs::tree(&pilot.out).unwrap() {
        if !entry.directory {
            assert!(!String::from_utf8_lossy(&fs::read(entry.source).unwrap()).contains(secret));
        }
    }
    assert_eq!(
        pilot
            .replay()
            .env_remove("CLIVERIFYR_BUNDLE_SECRET")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        pilot
            .replay()
            .env("CLIVERIFYR_BUNDLE_SECRET", "different")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let output = pilot
        .replay()
        .env("CLIVERIFYR_BUNDLE_SECRET", secret)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
}

#[test]
fn rejects_plain_and_json_escaped_secrets_unless_explicitly_included() {
    let pilot = Pilot::new(
        "redact_values_env=['CLIVERIFYR_BUNDLE_SECRET']\n[[cases]]\nid='one'\nargs=['stdin']\nstdin_file='secret.json'\nexpect={exit_code=0}\n",
    );
    let secret = "token-\"\\\nsecret";
    fs::write(
        pilot.dir.path().join("secret.json"),
        serde_json::to_vec(&json!({"value":secret})).unwrap(),
    )
    .unwrap();
    let output = pilot
        .bundle()
        .env("CLIVERIFYR_BUNDLE_SECRET", secret)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!pilot.out.exists());
    success(
        pilot
            .bundle()
            .env("CLIVERIFYR_BUNDLE_SECRET", secret)
            .arg("--include-sensitive-inputs")
            .output()
            .unwrap(),
    );
    let manifest: Value =
        serde_json::from_slice(&fs::read(pilot.out.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["sensitive_inputs_included"], true);
}

#[test]
fn bundle_does_not_execute_target_and_replay_preserves_failure_status() {
    let pilot = Pilot::new("[[cases]]\nid='one'\nargs=['exit','37']\nexpect={exit_code=0}\n");
    success(pilot.bundle().output().unwrap());
    let output = pilot.replay().output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["cases"][0]["raw_exit_code"], 37);
    assert_eq!(report["cases"][0]["status"], "FAIL");
}

#[test]
fn automatic_failure_bundle_preserves_the_actual_run_snapshot_and_witness() {
    let pilot = Pilot::new(
        "[[cases]]\nid='one'\nargs=['comparison-profile']\nstdin_file='input.txt'\nexpect={exit_code=0}\nstdout={mode='text_equals',text='expected'}\n",
    );
    let target = pilot.dir.path().join("release.exe");
    fs::copy(env!("CARGO_BIN_EXE_spanforge-verify-fixture"), &target).unwrap();
    let input = pilot.dir.path().join("input.txt");
    fs::write(&input, b"original failed input").unwrap();
    fs::write(
        target.with_extension("profile.json"),
        serde_json::to_vec(&json!({"echo_stdin":true,"mutate_source":input})).unwrap(),
    )
    .unwrap();
    let text = fs::read_to_string(&pilot.suite).unwrap().replace(
        env!("CARGO_BIN_EXE_spanforge-verify-fixture"),
        target.to_str().unwrap(),
    );
    fs::write(&pilot.suite, text).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_WORK_ROOT", pilot.dir.path())
        .args(["run", "--file"])
        .arg(&pilot.suite)
        .arg("--bundle-on-failure")
        .arg(&pilot.out)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(fs::read(&input).unwrap(), b"mutated");
    assert_eq!(
        fs::read(pilot.out.join("stdin.bin")).unwrap(),
        b"original failed input"
    );
    let witness: Value =
        serde_json::from_slice(&fs::read(pilot.out.join("failure.json")).unwrap()).unwrap();
    assert_eq!(witness["status"], "FAIL");
    let replay = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_WORK_ROOT", pilot.dir.path())
        .args(["replay", "--bundle"])
        .arg(&pilot.out)
        .arg("--program")
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(replay.status.code(), Some(1));
    let result: Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(
        result["cases"][0]["stdout"]["captured_bytes"],
        "original failed input".len()
    );
}

#[test]
fn bundle_destination_inside_absolute_fixture_input_is_rejected() {
    let pilot = Pilot::new("[[cases]]\nid='one'\nargs=['exit','0']\nexpect={exit_code=0}\n");
    let fixture = pilot.dir.path().join("source");
    fs::create_dir(&fixture).unwrap();
    fs::write(
        &pilot.suite,
        fs::read_to_string(&pilot.suite).unwrap()
            + &format!("fixture_dir='{}'\n", fixture.display()),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["bundle", "--file"])
        .arg(&pilot.suite)
        .args(["--case", "one", "--out"])
        .arg(fixture.join("nested-bundle"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.join("nested-bundle").exists());
}

#[test]
fn bundling_never_launches_target_and_invalid_auto_destination_stops_launch() {
    let pilot = Pilot::new("[[cases]]\nid='one'\nargs=['exit','0']\nexpect={exit_code=0}\n");
    let marker = pilot.dir.path().join("launch-marker");
    let text = fs::read_to_string(&pilot.suite).unwrap().replace(
        "args=['exit','0']",
        &format!("args=['touch','{}']", marker.display()),
    );
    fs::write(&pilot.suite, text).unwrap();
    success(pilot.bundle().output().unwrap());
    assert!(!marker.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_WORK_ROOT", pilot.dir.path())
        .args(["run", "--file"])
        .arg(&pilot.suite)
        .arg("--bundle-on-failure")
        .arg(&pilot.out)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!marker.exists());
}

#[test]
fn doctor_reports_missing_prerequisites_and_never_executes_target() {
    let pilot = Pilot::new(
        "redact_values_env=['CLIVERIFYR_BUNDLE_SECRET']\n[[cases]]\nid='one'\nargs=['exit','0']\ninherit_env=['CLIVERIFYR_BUNDLE_SECRET']\nexpect={exit_code=0}\n",
    );
    let marker = pilot.dir.path().join("must-not-launch");
    fs::write(
        &pilot.suite,
        fs::read_to_string(&pilot.suite).unwrap().replace(
            "args=['exit','0']",
            &format!("args=['touch','{}']", marker.display()),
        ),
    )
    .unwrap();
    success(
        pilot
            .bundle()
            .env("CLIVERIFYR_BUNDLE_SECRET", "doctor-secret-321")
            .output()
            .unwrap(),
    );
    let run_doctor = |secret: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"));
        command
            .args(["doctor", "--bundle"])
            .arg(&pilot.out)
            .arg("--program")
            .arg(env!("CARGO_BIN_EXE_spanforge-verify-fixture"));
        if let Some(secret) = secret {
            command.env("CLIVERIFYR_BUNDLE_SECRET", secret);
        } else {
            command.env_remove("CLIVERIFYR_BUNDLE_SECRET");
        }
        command.output().unwrap()
    };
    let output = run_doctor(None);
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ready"], false);
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "environment:CLIVERIFYR_BUNDLE_SECRET" && c["status"] == "fail")
    );
    assert!(!marker.exists());
    let output = run_doctor(Some("doctor-secret-321"));
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["ready"],
        true
    );
    assert!(!marker.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("doctor-secret-321"));
}
