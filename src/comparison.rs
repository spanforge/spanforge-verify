//! Compare recorded contract outcomes, not raw output or performance.
use crate::model::{RunResult, Status};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Serialize)]
pub struct Difference {
    pub case_id: String,
    pub field: String,
    pub baseline: Value,
    pub candidate: Value,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub schema_version: u32,
    pub baseline_target_hash: Option<String>,
    pub candidate_target_hash: Option<String>,
    pub differences: Vec<Difference>,
}

pub fn read_report(path: &Path) -> Result<RunResult, String> {
    let bytes = crate::inputs::read_bounded(path, 16 * 1024 * 1024)?;
    // Reject duplicate JSON keys before deserializing the versioned wire model.
    crate::strict_json::parse(&bytes).map_err(|_| "Invalid report JSON".to_owned())?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid report schema".into())
}

pub fn compare(baseline: &RunResult, candidate: &RunResult) -> Result<Comparison, String> {
    for report in [baseline, candidate] {
        if report.schema_version != 1
            || !matches!(report.status, Status::Pass | Status::Fail)
            || report.exit_code != report.status.exit_code()
            || !report.errors.is_empty()
            || report.details_omitted != 0
            || report.cases.is_empty()
            || report.cases.iter().any(|case| {
                !matches!(case.status, Status::Pass | Status::Fail)
                    || case.assertions.iter().any(|assertion| {
                        assertion.status == crate::model::AssertionStatus::NotEvaluated
                    })
            })
        {
            return Err("Comparison requires complete schema-v1 PASS/FAIL reports without omitted details or unevaluated assertions".into());
        }
    }
    if baseline.suite_hash.is_none()
        || baseline.suite_hash != candidate.suite_hash
        || baseline.suite_id != candidate.suite_id
    {
        return Err("Reports must have the same suite identity and suite hash".into());
    }
    // Owned sets avoid retaining references across report lifetimes.
    let before = baseline
        .cases
        .iter()
        .map(|c| c.case_id.clone())
        .collect::<BTreeSet<_>>();
    let after = candidate
        .cases
        .iter()
        .map(|c| c.case_id.clone())
        .collect::<BTreeSet<_>>();
    if before.len() != baseline.cases.len()
        || after.len() != candidate.cases.len()
        || before != after
    {
        return Err("Reports must contain the same unique case IDs".into());
    }
    let mut differences = Vec::new();
    for old in &baseline.cases {
        let new = candidate
            .cases
            .iter()
            .find(|c| c.case_id == old.case_id)
            .unwrap();
        let old = serde_json::to_value(old).map_err(|_| "Cannot compare report")?;
        let new = serde_json::to_value(new).map_err(|_| "Cannot compare report")?;
        for field in [
            "status",
            "reason_code",
            "raw_exit_code",
            "termination_reason",
            "stdin",
            "stdout",
            "stderr",
            "assertions",
            "workspace_deltas",
            "checked",
            "unchecked",
        ] {
            if old[field] != new[field] {
                differences.push(Difference {
                    case_id: old["case_id"].as_str().unwrap().into(),
                    field: field.into(),
                    baseline: old[field].clone(),
                    candidate: new[field].clone(),
                });
            }
        }
    }
    Ok(Comparison {
        schema_version: 1,
        baseline_target_hash: baseline.target_hash.clone(),
        candidate_target_hash: candidate.target_hash.clone(),
        differences,
    })
}
