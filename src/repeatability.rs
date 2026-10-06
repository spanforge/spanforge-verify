//! All-attempt evidence and explicit variability classification.
use crate::{
    model::{RunResult, Status},
    workflow::Attempt,
    workspace::Snapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub group: String,
    pub number: u32,
    pub total: u32,
    pub stdout_sha256: Option<String>,
    pub stderr_sha256: Option<String>,
    pub workspace_sha256: Option<String>,
}
impl Evidence {
    pub fn new(a: &Attempt) -> Self {
        Self {
            group: a.group.clone(),
            number: a.number,
            total: a.total,
            stdout_sha256: None,
            stderr_sha256: None,
            workspace_sha256: None,
        }
    }
    pub fn capture(&mut self, stdout: &[u8], stderr: &[u8], workspace: &Snapshot) {
        self.stdout_sha256 = Some(format!("{:x}", Sha256::digest(stdout)));
        self.stderr_sha256 = Some(format!("{:x}", Sha256::digest(stderr)));
        let mut hash = Sha256::new();
        for (path, entry) in workspace {
            hash.update((path.len() as u64).to_le_bytes());
            hash.update(path.as_bytes());
            match entry {
                crate::workspace::Entry::Directory => hash.update(b"directory"),
                crate::workspace::Entry::File { sha256, bytes } => {
                    hash.update(b"file");
                    hash.update(bytes.to_le_bytes());
                    hash.update(sha256.as_bytes());
                }
            }
        }
        self.workspace_sha256 = Some(format!("{:x}", hash.finalize()));
    }
}
#[derive(Serialize)]
pub struct Group {
    pub id: String,
    pub expected_attempts: u32,
    pub observed_attempts: usize,
    pub passes: usize,
    pub failures: usize,
    pub incomplete: usize,
    pub distinct_observations: usize,
    pub classification: &'static str,
    pub case_ids: Vec<String>,
}
#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub source_status: Status,
    pub source_exit_code: u8,
    pub groups: Vec<Group>,
    pub note: &'static str,
}
pub fn summarize(run: &RunResult) -> Result<Report, String> {
    if run.schema_version != 1 || run.cases.len() > 1000 || run.exit_code != run.status.exit_code()
    {
        return Err("Unsupported repeatability report".into());
    }
    let mut groups = BTreeMap::<String, Vec<_>>::new();
    let mut all_ids = BTreeSet::new();
    for case in &run.cases {
        if !all_ids.insert(&case.case_id) {
            return Err("Duplicate case IDs".into());
        }
        if let Some(a) = &case.attempt {
            groups.entry(a.group.clone()).or_default().push((case, a));
        }
    }
    let mut result = Vec::new();
    for (id, mut attempts) in groups {
        attempts.sort_by_key(|(_, a)| a.number);
        let total = attempts[0].1.total;
        let mut numbers = BTreeSet::new();
        let mut observations = BTreeSet::new();
        let (mut passes, mut failures, mut incomplete) = (0, 0, 0);
        for (case, a) in &attempts {
            if !(2..=100).contains(&total)
                || a.total != total
                || a.number == 0
                || a.number > total
                || !numbers.insert(a.number)
            {
                return Err("Invalid repeated-attempt identities".into());
            }
            for hash in [&a.stdout_sha256, &a.stderr_sha256, &a.workspace_sha256]
                .into_iter()
                .flatten()
            {
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Invalid observation digest".into());
                }
            }
            if case.status == Status::Pass {
                passes += 1;
            } else if case.status == Status::Fail {
                failures += 1;
            } else {
                incomplete += 1;
            }
            if let (Some(stdout), Some(stderr), Some(workspace)) =
                (&a.stdout_sha256, &a.stderr_sha256, &a.workspace_sha256)
            {
                observations.insert(format!(
                    "{:?}:{:?}:{stdout}:{stderr}:{workspace}:{:?}",
                    case.status, case.raw_exit_code, case.termination_reason
                ));
            } else {
                incomplete += usize::from(matches!(case.status, Status::Pass | Status::Fail));
            }
        }
        let classification = if attempts.len() != total as usize || incomplete > 0 {
            "incomplete"
        } else if passes > 0 && failures > 0 {
            "flaky"
        } else if observations.len() > 1 {
            "variable"
        } else if failures > 0 {
            "consistent_failure"
        } else {
            "consistent_pass"
        };
        result.push(Group {
            id,
            expected_attempts: total,
            observed_attempts: attempts.len(),
            passes,
            failures,
            incomplete,
            distinct_observations: observations.len(),
            classification,
            case_ids: attempts.iter().map(|(c, _)| c.case_id.clone()).collect(),
        });
    }
    Ok(Report {
        schema_version: 1,
        source_status: run.status,
        source_exit_code: run.exit_code,
        groups: result,
        note: "Every attempt remains in the source report. A passing attempt never erases a failure. Observations use exact stream and final-workspace hashes, exits and termination; timing is excluded. Variability is evidence, not a causal diagnosis.",
    })
}
