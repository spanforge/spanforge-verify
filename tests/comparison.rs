use spanforge_verify::{
    comparison::compare,
    model::{RunResult, Status},
};

fn report() -> RunResult {
    serde_json::from_str(include_str!("fixtures/reports/pass.json")).unwrap()
}

#[test]
fn compares_outcomes_but_ignores_execution_metadata() {
    let baseline = report();
    let mut candidate = report();
    candidate.run_id = "other".into();
    candidate.target_hash = Some("new executable".into());
    candidate.duration_ms += 100;
    candidate.cases[0].duration_ms += 100;
    assert!(
        compare(&baseline, &candidate)
            .unwrap()
            .differences
            .is_empty()
    );
    candidate.cases[0].raw_exit_code = Some(37);
    let result = compare(&baseline, &candidate).unwrap();
    assert_eq!(result.differences.len(), 1);
    assert_eq!(result.differences[0].field, "raw_exit_code");
    assert_eq!(result.differences[0].baseline, 0);
    assert_eq!(result.differences[0].candidate, 37);
}

#[test]
fn rejects_different_suites_partial_reports_and_duplicate_cases() {
    let baseline = report();
    let mut candidate = report();
    candidate.suite_hash = Some("different".into());
    assert!(compare(&baseline, &candidate).is_err());
    candidate = report();
    candidate.details_omitted = 1;
    assert!(compare(&baseline, &candidate).is_err());
    candidate = report();
    candidate.status = Status::Inconclusive;
    assert!(compare(&baseline, &candidate).is_err());
    candidate = report();
    candidate.cases.push(candidate.cases[0].clone());
    assert!(compare(&baseline, &candidate).is_err());
}

#[test]
fn command_emits_json_and_uses_difference_exit_status() {
    let dir = tempfile::tempdir_in(concat!(env!("CARGO_MANIFEST_DIR"), "/target")).unwrap();
    let baseline = dir.path().join("baseline.json");
    let candidate = dir.path().join("candidate.json");
    std::fs::write(&baseline, serde_json::to_vec(&report()).unwrap()).unwrap();
    let mut changed = report();
    changed.cases[0].raw_exit_code = Some(7);
    std::fs::write(&candidate, serde_json::to_vec(&changed).unwrap()).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .arg("compare-reports")
        .arg("--baseline")
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["differences"][0]["field"], "raw_exit_code");
    let equal = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["compare-reports", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&baseline)
        .output()
        .unwrap();
    assert_eq!(equal.status.code(), Some(0));
    std::fs::write(&candidate, b"{\"schema_version\":1,\"schema_version\":1}").unwrap();
    assert!(spanforge_verify::comparison::read_report(&candidate).is_err());
    let invalid = std::process::Command::new(env!("CARGO_BIN_EXE_spanforge-verify"))
        .args(["compare-reports", "--baseline"])
        .arg(&baseline)
        .arg("--candidate")
        .arg(&candidate)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
}
