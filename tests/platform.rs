use spanforge_verify::inputs::{ordinal_cmp, unique_names};
#[test]
fn platform_names_use_native_case_rules() {
    assert_eq!(ordinal_cmp("PATH", "path").is_eq(), cfg!(windows));
    assert_eq!(unique_names(["PATH", "path"]), !cfg!(windows));
}
#[cfg(unix)]
#[test]
fn linux_targets_need_executable_permission_not_an_exe_suffix() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let dir = tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    let target = dir.path().join("native-target");
    fs::copy(env!("CARGO_BIN_EXE_spanforge-verify-fixture"), &target).unwrap();
    let suite = dir.path().join("suite.toml");
    fs::write(&suite,"schema_version=1\nsuite_id='native'\nprogram='native-target'\n[[cases]]\nid='one'\nargs=[]\n[cases.expect]\nexit_code=0\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(spanforge_verify::schema::validate(&suite, None).is_err());
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(spanforge_verify::schema::validate(&suite, None).is_ok());
    fs::write(&target, b"#!/bin/sh\nexit 0\n").unwrap();
    assert!(spanforge_verify::schema::validate(&suite, None).is_err());
}
