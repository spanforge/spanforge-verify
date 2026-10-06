use spanforge_verify::schema::{validate, workspace_path};
use std::fs;

fn suite(extra: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    fs::write(
        dir.path().join("target.exe"),
        b"\x7fELFvalidation never launches this file",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("target.exe"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    let path = dir.path().join("suite.toml");
    fs::write(&path, format!("schema_version=1\nsuite_id='test'\nprogram='target.exe'\n[[cases]]\nid='one'\nargs=[]\n{extra}\n[cases.expect]\nexit_code=0\n")).unwrap();
    (dir, path)
}
#[test]
fn valid_suite_and_exact_filter() {
    let (_dir, path) = suite("");
    assert!(validate(&path, Some("one")).is_ok());
    assert!(validate(&path, Some("ONE")).is_err());
}
#[test]
fn unknown_keys_and_conflicting_stdin_fail() {
    for invalid in [
        "typo=true",
        "stdin_text='x'\nstdin_file='target.exe'",
        "timeout_ms=60001",
        "id='duplicate'",
    ] {
        let (_dir, path) = suite(invalid);
        assert!(validate(&path, None).is_err(), "{invalid}");
    }
}
#[test]
fn unsafe_windows_paths_fail() {
    for path in [
        "../out", "C:out", "/out", "a//b", "a/./b", "nul.txt", "COM1", "a:b", "out.", "out ",
        "a\\..\\b",
    ] {
        assert!(workspace_path(path).is_err(), "{path}");
    }
    assert!(workspace_path("out/result.json").is_ok());
}
#[test]
fn unknown_assertion_payload_fails() {
    let (_dir, path) = suite("");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n[cases.stdout]\nmode='contains'\ntext='ok'\npattern='extra'\n");
    fs::write(&path, text).unwrap();
    assert!(validate(&path, None).is_err());
}
#[test]
fn invalid_unselected_case_is_rejected() {
    let (_dir, path) = suite("");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("\n[[cases]]\nid='two'\nargs=[]\ntimeout_ms=0\n[cases.expect]\nexit_code=0\n");
    fs::write(&path, text).unwrap();
    assert!(validate(&path, Some("one")).is_err());
}

#[test]
fn file_declaration_accepts_only_kind_payload() {
    let (_dir, path) = suite("");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("{text}\n[[cases.files]]\npath='out'\nkind='directory'\n"),
    )
    .unwrap();
    assert!(validate(&path, None).is_ok());
    fs::write(
        &path,
        format!("{text}\n[[cases.files]]\npath='out'\nkind='directory'\nmode='exact_file'\n"),
    )
    .unwrap();
    assert!(validate(&path, None).is_err());
}

#[test]
fn configuration_contract_rejects_environment_regex_and_missing_fields() {
    let (_dir, path) = suite("");
    let original = fs::read_to_string(&path).unwrap();
    for text in [
        original.replace("args=[]\n", ""),
        original.replace("exit_code=0", "exit_code=4294967296"),
        original.replace("program='target.exe'", "program='missing.exe'"),
        format!("{original}\n[cases.stdout]\nmode='regex'\npattern='['\n"),
        format!(
            "{original}\n[cases.stdout]\nmode='contains'\ntext=''\nnormalize=['crlf_to_lf','crlf_to_lf']\n"
        ),
        format!("{original}\n[[cases]]\nid='one'\nargs=[]\n[cases.expect]\nexit_code=0\n"),
    ] {
        fs::write(&path, &text).unwrap();
        assert!(
            validate(&path, None).is_err(),
            "Unexpected valid configuration: {text}"
        );
    }
    #[cfg(windows)]
    for environment in ["PATH='one'\npath='two'", "home='reserved'"] {
        fs::write(&path, format!("{original}\n[cases.env]\n{environment}\n")).unwrap();
        assert!(validate(&path, None).is_err());
    }
    #[cfg(unix)]
    {
        fs::write(
            &path,
            format!("{original}\n[cases.env]\nPATH='one'\npath='two'\n"),
        )
        .unwrap();
        assert!(validate(&path, None).is_ok());
        fs::write(
            &path,
            format!("{original}\n[cases.env]\nTMPDIR='reserved'\n"),
        )
        .unwrap();
        assert!(validate(&path, None).is_err());
    }
}
