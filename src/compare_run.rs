//! Two native targets, shared immutable inputs, private bounded content capture.
use crate::{
    inputs,
    model::{CaseResult, RunError, RunResult, Status, TerminationReason},
    privacy::Masker,
    runner,
    semantic_diff::{ContentDifference, Differences, Options},
    workspace::{Entry, Snapshot},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const CAPTURE_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Default)]
struct Captures {
    bytes: usize,
    cases: BTreeMap<usize, Capture>,
    files: Vec<String>,
}
struct Capture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    workspace: Snapshot,
    files: BTreeMap<String, Vec<u8>>,
}
impl runner::CaseObserver for Captures {
    fn observe(
        &mut self,
        index: usize,
        stdout: &[u8],
        stderr: &[u8],
        workspace: &Snapshot,
        root: &Path,
    ) -> Result<(), String> {
        let cost =
            stdout.len() + stderr.len() + workspace.keys().map(|p| p.len() + 256).sum::<usize>();
        if cost > CAPTURE_LIMIT.saturating_sub(self.bytes) {
            return Err("Private comparison capture exceeds 64 MiB per target".into());
        }
        self.bytes += cost;
        let mut files = BTreeMap::new();
        for path in &self.files {
            if let Some((actual, Entry::File { bytes, sha256 })) = workspace
                .iter()
                .find(|(p, _)| inputs::ordinal_cmp(p, path).is_eq())
            {
                if *bytes > 16 * 1024 * 1024
                    || *bytes as usize > CAPTURE_LIMIT.saturating_sub(self.bytes)
                {
                    return Err(
                        "Selected file content exceeds private comparison capture limits".into(),
                    );
                }
                let data = inputs::read_bounded(&root.join(actual), *bytes)?;
                if stream_sha256(&data) != *sha256 {
                    return Err("Selected comparison file changed after snapshot".into());
                }
                self.bytes += data.len();
                files.insert(actual.clone(), data);
            }
        }
        self.cases.insert(
            index,
            Capture {
                stdout: stdout.into(),
                stderr: stderr.into(),
                workspace: workspace.clone(),
                files,
            },
        );
        Ok(())
    }
}

#[derive(Serialize)]
pub struct CaseOutcome {
    case_id: String,
    status: Status,
    reason_code: Option<String>,
    raw_exit_code: Option<u32>,
    termination_reason: Option<TerminationReason>,
}
#[derive(Serialize)]
pub struct Execution {
    run_id: String,
    target_hash: Option<String>,
    status: Status,
    cases: Vec<CaseOutcome>,
    errors: Vec<RunError>,
}
impl From<RunResult> for Execution {
    fn from(run: RunResult) -> Self {
        Self {
            run_id: run.run_id,
            target_hash: run.target_hash,
            status: run.status,
            errors: run.errors,
            cases: run
                .cases
                .into_iter()
                .map(|case| CaseOutcome {
                    case_id: case.case_id,
                    status: case.status,
                    reason_code: case.reason_code,
                    raw_exit_code: case.raw_exit_code,
                    termination_reason: case.termination_reason,
                })
                .collect(),
        }
    }
}
#[derive(Serialize)]
pub struct ContentEvidence {
    case_id: String,
    stdout_sha256: String,
    stderr_sha256: String,
    workspace_sha256: String,
}
fn evidence(plan: &inputs::RunPlan, captures: &Captures, masker: &Masker) -> Vec<ContentEvidence> {
    captures
        .cases
        .iter()
        .map(|(index, capture)| {
            let mut hash = Sha256::new();
            for (path, value) in &capture.workspace {
                hash.update((path.len() as u64).to_le_bytes());
                hash.update(path.as_bytes());
                let value = entry(value);
                hash.update((value.len() as u64).to_le_bytes());
                hash.update(value.as_bytes());
            }
            ContentEvidence {
                case_id: masker.diagnostic(&plan.suite().cases[*index].id).text,
                stdout_sha256: stream_sha256(&capture.stdout),
                stderr_sha256: stream_sha256(&capture.stderr),
                workspace_sha256: format!("{:x}", hash.finalize()),
            }
        })
        .collect()
}
#[derive(Serialize)]
pub struct ResultReport {
    pub schema_version: u32,
    pub suite_hash: String,
    pub baseline_target_hash: String,
    pub candidate_target_hash: String,
    pub runner_version: String,
    pub os: String,
    pub arch: String,
    pub status: Status,
    pub exit_code: u8,
    pub comparison_complete: bool,
    pub options: Options,
    pub baseline: Execution,
    pub candidate: Option<Execution>,
    pub baseline_evidence: Vec<ContentEvidence>,
    pub candidate_evidence: Vec<ContentEvidence>,
    pub differences: Vec<ContentDifference>,
    pub differences_omitted: u64,
    pub reason_code: Option<String>,
    pub policy_name: Option<String>,
    pub policy_hash: Option<String>,
    pub blocking_differences: u64,
    pub allowed_differences: u64,
    pub breaking_differences: u64,
}
fn complete(run: &RunResult, captures: &Captures) -> bool {
    matches!(run.status, Status::Pass | Status::Fail)
        && run.cases.len() == captures.cases.len()
        && run.cases.iter().all(|c| {
            c.termination_reason.is_none() && matches!(c.status, Status::Pass | Status::Fail)
        })
}
fn interrupted(run: &RunResult) -> Status {
    if run.status == Status::InfraError {
        Status::InfraError
    } else {
        Status::Inconclusive
    }
}

pub struct Request<'a> {
    pub file: &'a Path,
    pub selected: Option<&'a str>,
    pub baseline: &'a Path,
    pub candidate: &'a Path,
    pub options: Options,
    pub cancelled: Arc<AtomicBool>,
    pub started: Instant,
    pub started_at: String,
    pub policy: Option<crate::compatibility::Policy>,
}
pub fn execute(request: Request<'_>) -> Result<ResultReport, (u8, String)> {
    request.options.validate().map_err(|e| (2, e))?;
    if let Some(policy) = &request.policy {
        policy.validate().map_err(|e| (2, e))?;
    }
    let (old_plan, new_plan) = inputs::prepare_comparison(
        request.file,
        request.selected,
        request.baseline,
        request.candidate,
    )
    .map_err(|e| (if e == "input_changed" { 3 } else { 2 }, e))?;
    if (request.options.json_stdout || request.options.json_stderr || request.options.crlf_to_lf)
        && old_plan
            .cases()
            .iter()
            .any(|input| !old_plan.suite().cases[input.index()].steps.is_empty())
    {
        return Err((2,"Scenario stream comparison uses exact framed step bytes; JSON/CRLF stream modes are unsupported".into()));
    }
    if request.policy.as_ref().is_some_and(|p| {
        p.rules.iter().any(|r| {
            r.case
                .as_ref()
                .is_some_and(|id| !old_plan.suite().cases.iter().any(|c| &c.id == id))
        })
    }) {
        return Err((2, "Policy references an unknown suite case".into()));
    }
    let values = old_plan
        .suite()
        .redact_values_env
        .iter()
        .map(|name| {
            std::env::var(name).map_err(|_| (2, "Missing masking environment variable".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let masker = Masker::new(values).map_err(|e| (2, e))?;
    let deadline = request.started + Duration::from_millis(old_plan.suite().limits.run_timeout_ms);
    let selected_files = request
        .options
        .json_files
        .iter()
        .chain(&request.options.text_files)
        .cloned()
        .collect::<Vec<_>>();
    let mut old_capture = Captures {
        files: selected_files.clone(),
        ..Default::default()
    };
    eprintln!("Running baseline executable");
    let old = runner::execute_plan_started(
        &old_plan,
        request.cancelled.clone(),
        request.started,
        request.started_at.clone(),
        Some(&mut old_capture),
    )?;
    let mut report = ResultReport {
        schema_version: 1,
        suite_hash: old_plan.suite_hash().into(),
        baseline_target_hash: old_plan.target_hash().into(),
        candidate_target_hash: new_plan.target_hash().into(),
        runner_version: env!("CARGO_PKG_VERSION").into(),
        os: crate::reports::os_build(),
        arch: std::env::consts::ARCH.into(),
        status: interrupted(&old),
        exit_code: interrupted(&old).exit_code(),
        comparison_complete: false,
        options: request.options,
        baseline: Execution {
            run_id: String::new(),
            target_hash: None,
            status: Status::Inconclusive,
            cases: vec![],
            errors: vec![],
        },
        candidate: None,
        baseline_evidence: evidence(&old_plan, &old_capture, &masker),
        candidate_evidence: vec![],
        differences: vec![],
        differences_omitted: 0,
        reason_code: Some("baseline_incomplete".into()),
        policy_name: request
            .policy
            .as_ref()
            .map(|p| masker.diagnostic(&p.name).text),
        policy_hash: request.policy.as_ref().map(|p| p.hash()),
        blocking_differences: 0,
        allowed_differences: 0,
        breaking_differences: 0,
    };
    if !complete(&old, &old_capture) {
        report.baseline = old.into();
        mask_options(&mut report.options, &masker);
        return Ok(report);
    }
    let mut new_capture = Captures {
        files: selected_files,
        ..Default::default()
    };
    eprintln!("Running candidate executable");
    let new = runner::execute_plan_started(
        &new_plan,
        request.cancelled.clone(),
        request.started,
        request.started_at,
        Some(&mut new_capture),
    )?;
    report.candidate_evidence = evidence(&new_plan, &new_capture, &masker);
    if !complete(&new, &new_capture) {
        report.status = interrupted(&new);
        report.exit_code = report.status.exit_code();
        report.reason_code = Some("candidate_incomplete".into());
    } else {
        let mut differences = Differences::with_policy(&masker, request.policy.as_ref());
        for (position, input) in old_plan.cases().iter().enumerate() {
            if Instant::now() >= deadline
                || request.cancelled.load(std::sync::atomic::Ordering::SeqCst)
            {
                report.status = Status::Inconclusive;
                report.exit_code = 4;
                report.reason_code = Some("comparison_interrupted".into());
                break;
            }
            let id = &old_plan.suite().cases[input.index()].id;
            let old_case = &old.cases[position];
            let new_case = &new.cases[position];
            outcomes(&mut differences, id, old_case, new_case);
            let a = &old_capture.cases[&input.index()];
            let b = &new_capture.cases[&input.index()];
            for (field, before, after) in [
                ("stdout", a.stdout.as_slice(), b.stdout.as_slice()),
                ("stderr", a.stderr.as_slice(), b.stderr.as_slice()),
            ] {
                differences
                    .stream(id, field, before, after, &report.options)
                    .map_err(|e| (2, e))?;
            }
            let paths = a
                .workspace
                .keys()
                .chain(b.workspace.keys())
                .collect::<BTreeSet<_>>();
            for path in paths {
                if let (Some(before), Some(after)) = (a.files.get(path), b.files.get(path)) {
                    let json = report
                        .options
                        .json_files
                        .iter()
                        .any(|p| inputs::ordinal_cmp(p, path).is_eq());
                    differences
                        .file(id, path, (before, after), json, &report.options)
                        .map_err(|e| (2, e))?;
                    continue;
                }
                if a.workspace.get(path) != b.workspace.get(path) {
                    differences.add(
                        id,
                        "workspace",
                        path,
                        "content_changed",
                        a.workspace.get(path).map(entry),
                        b.workspace.get(path).map(entry),
                    );
                }
            }
            if position + 1 == old_plan.cases().len() {
                report.comparison_complete = true;
            }
        }
        report.differences = differences.items;
        report.differences_omitted = differences.omitted;
        report.blocking_differences = differences.blocking;
        report.allowed_differences = differences.allowed;
        report.breaking_differences = differences.breaking;
        if report.comparison_complete {
            if Instant::now() >= deadline
                || request.cancelled.load(std::sync::atomic::Ordering::SeqCst)
            {
                report.comparison_complete = false;
                report.status = Status::Inconclusive;
                report.reason_code = Some("comparison_interrupted".into());
            } else {
                report.status = if old.status == Status::Fail
                    || new.status == Status::Fail
                    || report.blocking_differences > 0
                {
                    Status::Fail
                } else {
                    Status::Pass
                };
                report.reason_code = None;
            }
            report.exit_code = report.status.exit_code();
        }
    }
    report.baseline = old.into();
    report.candidate = Some(new.into());
    mask_options(&mut report.options, &masker);
    Ok(report)
}
fn mask_options(options: &mut Options, masker: &Masker) {
    for pointer in &mut options.ignore_json_pointers {
        *pointer = crate::semantic_diff::masked_pointer(masker, pointer);
    }
    for path in options.json_files.iter_mut().chain(&mut options.text_files) {
        *path = masker.diagnostic(path).text;
    }
}
fn outcomes(diff: &mut Differences<'_>, id: &str, old: &CaseResult, new: &CaseResult) {
    if old.raw_exit_code != new.raw_exit_code {
        diff.add(
            id,
            "exit_code",
            "",
            "changed",
            old.raw_exit_code.map(|n| n.to_string()),
            new.raw_exit_code.map(|n| n.to_string()),
        );
    }
    if old.status != new.status {
        diff.add(
            id,
            "contract_status",
            "",
            if old.status == Status::Pass && new.status == Status::Fail {
                "contract_regression"
            } else {
                "changed"
            },
            Some(old.status.to_string()),
            Some(new.status.to_string()),
        );
    }
    for (a, b) in old.assertions.iter().zip(&new.assertions) {
        if a.status != b.status {
            diff.add(
                id,
                "assertion",
                &a.check_id,
                "changed",
                Some(format!("{:?}", a.status)),
                Some(format!("{:?}", b.status)),
            );
        }
    }
}
fn entry(value: &Entry) -> String {
    match value {
        Entry::Directory => "directory".into(),
        Entry::File { sha256, bytes } => format!("file: {bytes} bytes; sha256={sha256}"),
    }
}

pub fn stream_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::CaseObserver;
    #[test]
    fn capture_budget_rejects_before_copying_and_accepts_exact_boundary() {
        let mut captures = Captures {
            bytes: CAPTURE_LIMIT - 2,
            ..Default::default()
        };
        captures
            .observe(0, b"ab", b"", &Snapshot::new(), Path::new("."))
            .unwrap();
        assert_eq!(captures.bytes, CAPTURE_LIMIT);
        assert!(
            captures
                .observe(1, b"x", b"", &Snapshot::new(), Path::new("."))
                .is_err()
        );
        assert_eq!(captures.cases.len(), 1);
    }
}
