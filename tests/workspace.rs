use spanforge_verify::{
    inputs::prepare,
    schema::{FileKind, FileRule},
    workspace::{CaseWorkspace, deltas, snapshot, undeclared_changes},
};
use std::{
    fs,
    time::{Duration, Instant},
};
#[test]
fn workspace_changes_are_checked_and_sources_are_preserved() {
    let dir = tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    fs::write(dir.path().join("target.exe"), b"\x7fELFnot launched").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("target.exe"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    fs::create_dir(dir.path().join("fixtures")).unwrap();
    fs::write(dir.path().join("fixtures/input"), b"approved").unwrap();
    let file = dir.path().join("suite.toml");
    fs::write(&file,"schema_version=1\nsuite_id='workspace'\nprogram='target.exe'\n[[cases]]\nid='one'\nargs=[]\nfixture_dir='fixtures'\n[cases.expect]\nexit_code=0\n").unwrap();
    let plan = prepare(&file, None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let workspace = CaseWorkspace::create(dir.path(), &plan.cases()[0], deadline).unwrap();
    fs::write(workspace.path().join("input"), b"changed").unwrap();
    fs::create_dir(workspace.path().join("out")).unwrap();
    fs::write(workspace.path().join("out/result"), b"output").unwrap();
    fs::write(workspace.path().join("out/sibling"), b"unexpected").unwrap();
    let after = snapshot(workspace.path(), deadline).unwrap();
    let delta = deltas(workspace.baseline(), &after);
    let unexpected = undeclared_changes(
        workspace.baseline(),
        &delta,
        &[FileRule {
            path: "out/result".into(),
            rule: FileKind::File {
                mode: "exact_file".into(),
                expected_file: "expected".into(),
            },
        }],
    );
    assert_eq!(unexpected, vec!["input", "out/sibling"]);
    assert_eq!(
        fs::read(dir.path().join("fixtures/input")).unwrap(),
        b"approved"
    );
    let private = workspace.private_environment();
    assert!(
        private
            .values()
            .all(|value| !std::path::Path::new(value).starts_with(workspace.path()))
    );
    let path = workspace.path().to_path_buf();
    workspace.cleanup(deadline).unwrap();
    assert!(!path.exists());
}
