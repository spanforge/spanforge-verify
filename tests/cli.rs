use std::{fs, process::Command};
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target")).unwrap()
}
#[test]
fn product_name_and_legacy_entry_point_share_version() {
    for binary in [
        env!("CARGO_BIN_EXE_spanforge-verify"),
        env!("CARGO_BIN_EXE_cliverifyr"),
    ] {
        let output = Command::new(binary).arg("--version").output().unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("spanforge-verify 0.1.0")
        );
    }
}
#[test]
fn default_config_migration_preserves_explicit_paths() {
    let dir = scratch();
    let binary = env!("CARGO_BIN_EXE_spanforge-verify");
    let init = Command::new(binary)
        .current_dir(dir.path())
        .arg("init")
        .output()
        .unwrap();
    assert!(init.status.success());
    let current = dir.path().join("spanforge-verify.toml");
    let legacy = dir.path().join("cliverifyr.toml");
    assert!(current.exists());
    let suite = format!(
        "schema_version=1\nsuite_id='migration'\nprogram='{}'\n[[cases]]\nid='one'\nargs=['exit','0']\n[cases.expect]\nexit_code=0\n",
        env!("CARGO_BIN_EXE_spanforge-verify-fixture")
    );
    fs::write(&current, &suite).unwrap();
    fs::write(&legacy, "invalid TOML").unwrap();
    let validate = |args: &[&str]| {
        Command::new(binary)
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(validate(&["validate"]).status.success());
    fs::remove_file(&current).unwrap();
    fs::write(&legacy, suite).unwrap();
    assert!(validate(&["validate"]).status.success());
    assert_eq!(
        validate(&["validate", "--file", "spanforge-verify.toml"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        validate(&["validate", "--file", "missing.toml"])
            .status
            .code(),
        Some(2)
    );
    let run = Command::new(binary)
        .current_dir(dir.path())
        .arg("run")
        .env("SPANFORGE_VERIFY_WORK_ROOT", dir.path())
        .env("CLIVERIFYR_WORK_ROOT", dir.path().join("does-not-exist"))
        .output()
        .unwrap();
    assert!(run.status.success(), "{:?}", run.stderr);
}
#[test]
fn init_refuses_overwrite_without_changing_bytes() {
    let dir = scratch();
    let file = dir.path().join("suite.toml");
    let first = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["init", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(first.status.success());
    let original = fs::read(&file).unwrap();
    let second = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["init", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(2));
    assert_eq!(fs::read(&file).unwrap(), original);
}
#[test]
fn validate_never_launches_the_target_and_run_executes_selected_cases() {
    let dir = scratch();
    let file = dir.path().join("suite.toml");
    let marker = dir.path().join("resumed.txt");
    let program = env!("CARGO_BIN_EXE_spanforge-verify-fixture");
    let text = format!(
        "schema_version=1\nsuite_id='no-launch'\nprogram='{program}'\n[[cases]]\nid='one'\nargs=['touch','{}']\n[cases.expect]\nexit_code=0\n",
        marker.display()
    );
    fs::write(&file, text).unwrap();
    let validated = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["validate", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(validated.status.success(), "{:?}", validated.stderr);
    assert!(!marker.exists());
    let run = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .env("CLIVERIFYR_WORK_ROOT", dir.path())
        .args(["run", "--file"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(0), "{:?}", run.stderr);
    assert!(marker.exists());
    fs::remove_file(&marker).unwrap();
    let missing = Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["run", "--file"])
        .arg(&file)
        .args(["--case", "missing"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(!marker.exists());
}
