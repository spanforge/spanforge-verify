//! Structural coverage of explicitly declared contracts, not inferred behavior.
use crate::schema::Suite;
use serde::Serialize;
#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub suite_id: String,
    pub declared_contracts: usize,
    pub mapped_contracts: usize,
    pub unmapped_contracts: Vec<String>,
    pub cases_without_contracts: Vec<String>,
    pub stdout_unasserted: Vec<String>,
    pub stderr_unasserted: Vec<String>,
    pub note: &'static str,
}
pub fn report(suite: &Suite) -> Report {
    let stream_cases: Vec<_> = suite
        .cases
        .iter()
        .flat_map(|case| {
            if case.steps.is_empty() {
                vec![(case.id.clone(), case)]
            } else {
                case.steps
                    .iter()
                    .map(|step| (format!("{}/{}", case.id, step.id), step))
                    .collect()
            }
        })
        .collect();
    Report {
        schema_version: 1,
        suite_id: suite.suite_id.clone(),
        declared_contracts: suite.contracts.len(),
        mapped_contracts: suite
            .contracts
            .iter()
            .filter(|c| !c.cases.is_empty())
            .count(),
        unmapped_contracts: suite
            .contracts
            .iter()
            .filter(|c| c.cases.is_empty())
            .map(|c| c.id.clone())
            .collect(),
        cases_without_contracts: suite
            .cases
            .iter()
            .filter(|c| !suite.contracts.iter().any(|b| b.cases.contains(&c.id)))
            .map(|c| c.id.clone())
            .collect(),
        stdout_unasserted: stream_cases
            .iter()
            .filter(|(_, c)| c.stdout.is_none())
            .map(|(id, _)| id.clone())
            .collect(),
        stderr_unasserted: stream_cases
            .iter()
            .filter(|(_, c)| c.stderr.is_none())
            .map(|(id, _)| id.clone())
            .collect(),
        note: "Mappings identify tests, not proof of behavior. No target was executed. Unasserted streams may be intentional. Undeclared behaviors are unknown.",
    }
}
