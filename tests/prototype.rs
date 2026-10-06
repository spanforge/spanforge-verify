#[cfg(windows)]
#[test]
fn windows_lifecycle_scenarios() {
    let fixture = std::path::Path::new(env!("CARGO_BIN_EXE_spanforge-verify-fixture"));
    let evidence = spanforge_verify::prototype_gate::run(fixture, 1).unwrap();
    let failures: Vec<_> = evidence.iter().filter(|record| !record.passed).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}
