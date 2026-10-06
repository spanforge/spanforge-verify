use spanforge_verify::{
    inputs::{INPUT_BUDGET, prepare},
    schema::validate,
};
use std::fs;
fn suite() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    fs::write(dir.path().join("target.exe"), b"\x7fELFno launch").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            dir.path().join("target.exe"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
    }
    fs::create_dir(dir.path().join("fixture")).unwrap();
    fs::write(dir.path().join("fixture/input.txt"), b"original").unwrap();
    fs::write(dir.path().join("expected.json"), b"1.0").unwrap();
    let file = dir.path().join("suite.toml");
    fs::write(&file,"schema_version=1\nsuite_id='inputs'\nprogram='target.exe'\n[[cases]]\nid='one'\nargs=[]\nfixture_dir='fixture'\nstdin_file='fixture/input.txt'\n[cases.expect]\nexit_code=0\n[cases.stdout]\nmode='json_equals_file'\nexpected_file='expected.json'\n").unwrap();
    (dir, file)
}
#[test]
fn snapshot_remains_immutable_after_source_changes() {
    let (dir, file) = suite();
    let plan = prepare(&file, None).unwrap();
    fs::write(dir.path().join("fixture/input.txt"), b"changed").unwrap();
    fs::write(dir.path().join("expected.json"), b"2").unwrap();
    assert_eq!(plan.cases()[0].stdin(), b"original");
    assert_eq!(
        plan.cases()[0].expected("expected.json"),
        Some(b"1.0".as_slice())
    );
    assert_eq!(
        plan.cases()[0].fixture()["input.txt"].as_deref(),
        Some(b"original".as_slice())
    );
    assert_eq!(plan.suite_hash().len(), 64);
    assert_eq!(plan.target_hash().len(), 64);
}
#[test]
fn invalid_expected_json_is_rejected_before_filter() {
    let (dir, file) = suite();
    fs::write(dir.path().join("expected.json"), br#"{"key":1,"key":2}"#).unwrap();
    assert!(validate(&file, Some("one")).is_err());
}
#[test]
fn total_input_budget_is_checked_without_loading_oversized_input() {
    let (dir, file) = suite();
    let input = fs::File::create(dir.path().join("fixture/huge.bin")).unwrap();
    input.set_len(INPUT_BUDGET + 1).unwrap();
    drop(input);
    assert!(validate(&file, None).unwrap_err().contains("budget"));
}
#[test]
fn json_pointer_literals_are_strict() {
    let (_dir, file) = suite();
    let text = fs::read_to_string(&file).unwrap();
    let text = text.replace(
        "mode='json_equals_file'\nexpected_file='expected.json'",
        "mode='json_pointers'\nvalues = { '/value' = 'null', '' = '{\"a\":1}' }",
    );
    fs::write(&file, &text).unwrap();
    assert!(validate(&file, None).is_ok());
    fs::write(&file, text.replace("'/value' = 'null'", "'/value' = 'NaN'")).unwrap();
    assert!(validate(&file, None).is_err());
}
#[test]
fn equivalent_separators_collide_and_file_parent_cannot_have_children() {
    let (_dir, file) = suite();
    let text = fs::read_to_string(&file).unwrap();
    for declarations in [
        "[[cases.files]]\npath='out/file'\nkind='absent'\n[[cases.files]]\npath='OUT\\file'\nkind='absent'",
        "[[cases.files]]\npath='out'\nkind='file'\nmode='exact_file'\nexpected_file='expected.json'\n[[cases.files]]\npath='out/child'\nkind='absent'",
    ] {
        fs::write(&file, format!("{text}\n{declarations}\n")).unwrap();
        assert!(validate(&file, None).is_err());
    }
}
#[test]
fn suite_hash_ignores_toml_layout() {
    let (_dir, file) = suite();
    let hash = prepare(&file, None).unwrap().suite_hash().to_owned();
    let text = fs::read_to_string(&file).unwrap();
    fs::write(&file, format!("# new comment\n{text}\n")).unwrap();
    assert_eq!(prepare(&file, None).unwrap().suite_hash(), hash);
}

#[test]
fn case_selection_does_not_change_suite_identity() {
    let (_dir, file) = suite();
    let text = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        format!("{text}\n[[cases]]\nid='two'\nargs=[]\n[cases.expect]\nexit_code=0\n"),
    )
    .unwrap();
    let all = prepare(&file, None).unwrap();
    let selected = prepare(&file, Some("one")).unwrap();
    assert_eq!(all.suite_hash(), selected.suite_hash());
    assert_eq!(all.cases().len(), 2);
    assert_eq!(selected.cases().len(), 1);
}
