//! Sequential execution of immutable suite plans.
use crate::{
    inputs::{self, CaseInputs},
    model::*,
    privacy::Masker,
    schema::{Case, FileKind, StreamRule},
    workspace::{self, CaseWorkspace, Entry},
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub fn execute(
    file: &Path,
    selected: Option<&str>,
    cancelled: Arc<AtomicBool>,
) -> Result<RunResult, (u8, String)> {
    execute_started(
        file,
        selected,
        cancelled,
        Instant::now(),
        crate::reports::utc_now(),
    )
}
pub fn execute_started(
    file: &Path,
    selected: Option<&str>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
    started_at: String,
) -> Result<RunResult, (u8, String)> {
    let plan = inputs::prepare(file, selected)
        .map_err(|e| (if e == "input_changed" { 3 } else { 2 }, e))?;
    execute_plan_started(&plan, cancelled, started, started_at, None)
}

/// Private raw observations are consumed before cleanup and never added to v1 reports.
pub trait CaseObserver {
    fn observe(
        &mut self,
        index: usize,
        stdout: &[u8],
        stderr: &[u8],
        workspace: &workspace::Snapshot,
        root: &Path,
    ) -> Result<(), String>;
}
pub fn execute_plan_started(
    plan: &inputs::RunPlan,
    cancelled: Arc<AtomicBool>,
    started: Instant,
    started_at: String,
    mut observer: Option<&mut dyn CaseObserver>,
) -> Result<RunResult, (u8, String)> {
    let suite = plan.suite();
    let deadline = started + Duration::from_millis(suite.limits.run_timeout_ms);
    let values = suite
        .redact_values_env
        .iter()
        .map(|name| {
            std::env::var(name)
                .map_err(|_| (2, format!("Missing masking environment variable: {name}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let masker = Masker::new(values).map_err(|e| (2, e))?;
    let scratch_parent = std::env::var_os("SPANFORGE_VERIFY_WORK_ROOT")
        .or_else(|| std::env::var_os("CLIVERIFYR_WORK_ROOT"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let scratch = tempfile::tempdir_in(scratch_parent).map_err(|e| (3, e.to_string()))?;
    let mut result = RunResult {
        publication_deadline: None,
        schema_version: 1,
        run_id: crate::reports::run_id(),
        suite_id: Some(suite.suite_id.clone()),
        suite_hash: Some(plan.suite_hash().into()),
        target_hash: Some(plan.target_hash().into()),
        runner_version: env!("CARGO_PKG_VERSION").into(),
        os: crate::reports::os_build(),
        arch: std::env::consts::ARCH.into(),
        started_at,
        duration_ms: 0,
        status: Status::Pass,
        exit_code: 0,
        limits: RunLimits {
            run_timeout_ms: suite.limits.run_timeout_ms,
            max_cases: suite.limits.max_cases,
        },
        errors: vec![],
        cases: vec![],
        details_omitted: 0,
    };
    let mut abort = false;
    for inputs in plan.cases() {
        let case = &suite.cases[inputs.index()];
        let mut output = empty(
            case,
            case.timeout_ms.unwrap_or(suite.defaults.timeout_ms),
            case.max_output_bytes
                .unwrap_or(suite.defaults.max_output_bytes),
        );
        if abort || cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            output.status = Status::Inconclusive;
            output.reason_code = Some(
                if abort {
                    "not_run_after_abort"
                } else if cancelled.load(Ordering::SeqCst) {
                    "cancelled"
                } else {
                    "run_deadline"
                }
                .into(),
            );
            abort = true;
        } else {
            eprintln!(
                "Running {}",
                crate::privacy::terminal_escape(&masker.mask(&case.id))
            );
            match CaseWorkspace::create(scratch.path(), inputs, deadline) {
                Err(error) => {
                    output.status = if error.reason_code == "run_deadline" {
                        Status::Inconclusive
                    } else {
                        Status::InfraError
                    };
                    output.reason_code = Some(error.reason_code.into());
                    result.errors.push(RunError {
                        reason_code: error.reason_code.into(),
                        message: error.to_string(),
                    });
                }
                Ok(workspace) => {
                    let before = Instant::now();
                    let evaluate = if case.steps.is_empty() {
                        run_case
                    } else {
                        run_scenario
                    };
                    let evaluated = evaluate(
                        plan,
                        inputs,
                        case,
                        &workspace,
                        deadline,
                        cancelled.clone(),
                        CaseEvaluation {
                            output: &mut output,
                            observer: &mut observer,
                            baseline: None,
                            bindings: None,
                        },
                    );
                    if let Err((code, message)) = evaluated {
                        output.status = if code == "run_deadline" {
                            Status::Inconclusive
                        } else {
                            Status::InfraError
                        };
                        output.reason_code = Some(code.clone());
                        if output.status == Status::InfraError {
                            output.termination_reason = Some(TerminationReason::Infrastructure);
                        }
                        result.errors.push(RunError {
                            reason_code: code,
                            message,
                        });
                    }
                    let cleanup_deadline = Instant::now() + Duration::from_secs(5);
                    if matches!(output.status, Status::InfraError | Status::Inconclusive) {
                        result.publication_deadline = Some(cleanup_deadline);
                    }
                    if let Err(error) = workspace.cleanup(cleanup_deadline) {
                        output.status = Status::InfraError;
                        output.reason_code = Some(error.reason_code.into());
                        result.errors.push(RunError {
                            reason_code: error.reason_code.into(),
                            message: error.to_string(),
                        });
                    }
                    output.duration_ms = before.elapsed().as_millis() as u64;
                }
            }
            abort = matches!(output.status, Status::InfraError | Status::Inconclusive);
        }
        if abort && result.publication_deadline.is_none() {
            result.publication_deadline = Some(Instant::now() + Duration::from_secs(5));
        }
        if !case.steps.is_empty() && output.steps.is_empty() {
            for step in &case.steps {
                let mut unrun = empty(
                    step,
                    step.timeout_ms.unwrap_or(output.limits.timeout_ms),
                    step.max_output_bytes
                        .unwrap_or(output.limits.max_output_bytes),
                );
                unrun.status = Status::Inconclusive;
                unrun.reason_code = Some("not_run_parent".into());
                output.unchecked.extend(
                    unrun
                        .unchecked
                        .iter()
                        .map(|check| format!("step:{}/{check}", step.id)),
                );
                output.steps.push(unrun);
            }
        }
        output.checked = output
            .assertions
            .iter()
            .filter(|a| a.status != AssertionStatus::NotEvaluated)
            .map(|a| a.check_id.clone())
            .collect();
        output.unchecked.retain(|id| !output.checked.contains(id));
        result.cases.push(output);
    }
    result.status = Status::aggregate(result.cases.iter().map(|c| c.status));
    result.exit_code = result.status.exit_code();
    result.duration_ms = started.elapsed().as_millis() as u64;
    if result
        .errors
        .iter()
        .any(|e| e.reason_code == "cleanup_failed")
    {
        let _ = scratch.keep();
    }
    // Bound each diagnostic after literal masking, before serialization/escaping.
    for error in &mut result.errors {
        error.message = masker.diagnostic(&error.message).text;
    }
    for case in &mut result.cases {
        mask_case(case, &masker, &mut result.details_omitted);
    }
    if let Some(id) = &mut result.suite_id {
        *id = masker.mask(id);
    }
    Ok(result)
}
fn mask_case(case: &mut CaseResult, masker: &Masker, details_omitted: &mut u64) {
    if let Some(attempt) = &mut case.attempt {
        attempt.group = masker.mask(&attempt.group);
    }
    case.case_id = masker.mask(&case.case_id);
    for check in &mut case.assertions {
        if check.status == AssertionStatus::Pass
            && (matches!(check.check_id.as_str(), "stdout" | "stderr")
                || check.check_id.ends_with("/stdout")
                || check.check_id.ends_with("/stderr"))
        {
            check.observed_summary = None;
        }
        for summary in [&mut check.expected_summary, &mut check.observed_summary]
            .into_iter()
            .flatten()
        {
            let diagnostic = masker.diagnostic(summary);
            *details_omitted += u64::from(diagnostic.truncated);
            *summary = diagnostic.text;
        }
        check.check_id = masker.mask(&check.check_id);
    }
    for delta in &mut case.workspace_deltas {
        delta.path = masker.mask(&delta.path);
    }
    for name in case.checked.iter_mut().chain(case.unchecked.iter_mut()) {
        *name = masker.mask(name);
    }
    for step in &mut case.steps {
        mask_case(step, masker, details_omitted);
    }
}
fn empty(case: &Case, timeout_ms: u64, max_output_bytes: u64) -> CaseResult {
    CaseResult {
        steps: vec![],
        attempt: case
            .attempt
            .as_ref()
            .map(crate::repeatability::Evidence::new),
        case_id: case.id.clone(),
        status: Status::Pass,
        reason_code: None,
        raw_exit_code: None,
        termination_reason: None,
        duration_ms: 0,
        limits: CaseLimits {
            timeout_ms,
            max_output_bytes,
        },
        stdin: StdinResult::default(),
        stdout: StreamResult::default(),
        stderr: StreamResult::default(),
        assertions: vec![],
        workspace_deltas: vec![],
        checked: vec![],
        unchecked: [
            "exit_code",
            "stdin_delivery",
            "lifecycle",
            "workspace_changes",
            "stdout",
            "stderr",
        ]
        .into_iter()
        .map(String::from)
        .chain(case.files.iter().map(|f| format!("file:{}", f.path)))
        .chain(case.http.as_ref().map(|_| "http_requests".to_owned()))
        .chain((!case.extract.is_empty()).then(|| "extraction".to_owned()))
        .collect(),
    }
}
fn environment(
    inputs: &CaseInputs,
    workspace: &CaseWorkspace,
) -> Result<BTreeMap<String, String>, (String, String)> {
    let mut env = inputs.environment().clone();
    env.extend(workspace.private_environment());
    Ok(env)
}
#[cfg(target_os = "linux")]
use crate::process_linux as process;
#[cfg(windows)]
use crate::process_windows as process;

struct CaseEvaluation<'a, 'b> {
    output: &'a mut CaseResult,
    observer: &'a mut Option<&'b mut dyn CaseObserver>,
    baseline: Option<&'a workspace::Snapshot>,
    bindings: Option<&'a mut BTreeMap<String, String>>,
}
fn run_case(
    plan: &inputs::RunPlan,
    inputs: &CaseInputs,
    case: &Case,
    workspace: &CaseWorkspace,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    evaluation: CaseEvaluation<'_, '_>,
) -> Result<(), (String, String)> {
    let output = evaluation.output;
    let mut env = environment(inputs, workspace)?;
    for (name, value) in &case.env {
        env.retain(|key, _| !inputs::ordinal_cmp(key, name).is_eq());
        env.insert(name.clone(), value.clone());
    }
    let http = case
        .http
        .as_ref()
        .map(|fixture| {
            crate::http_fixture::Server::start(fixture, deadline)
                .map_err(|e| ("http_fixture_failed".into(), e))
        })
        .transpose()?;
    if let (Some(server), Some(fixture)) = (&http, &case.http) {
        env.insert(fixture.url_env.clone(), server.url.clone());
    }
    let args: Vec<String> = case
        .args
        .iter()
        .map(|arg| {
            http.as_ref()
                .map(|server| arg.replace("{{http.url}}", &server.url))
                .unwrap_or_else(|| arg.clone())
        })
        .collect();
    #[cfg(target_os = "linux")]
    plan.verify_target(deadline).map_err(|e| {
        (
            if e == "run_deadline" {
                e.clone()
            } else {
                "input_changed".into()
            },
            e,
        )
    })?;
    let cwd = case
        .cwd
        .as_ref()
        .map(|p| workspace.path().join(p))
        .unwrap_or_else(|| workspace.path().into());
    let observation = process::run(&process::Request {
        executable: plan.executable(),
        #[cfg(target_os = "linux")]
        executable_file: plan.target_file(),
        args: &args,
        cwd: &cwd,
        env: &env,
        stdin: case
            .stdin_text
            .as_ref()
            .map(|s| s.as_bytes())
            .or_else(|| case.stdin_file.as_ref().and_then(|p| inputs.expected(p)))
            .unwrap_or_else(|| inputs.stdin()),
        max_output_bytes: output.limits.max_output_bytes as usize,
        timeout: Duration::from_millis(output.limits.timeout_ms),
        run_deadline: deadline,
        cancelled,
    })
    .map_err(|e| (e.reason_code.into(), e.to_string()))?;
    #[cfg(target_os = "linux")]
    if !matches!(
        observation.termination,
        Some(TerminationReason::RunDeadline | TerminationReason::Cancelled)
    ) {
        plan.verify_target(deadline).map_err(|e| {
            (
                if e == "run_deadline" {
                    e.clone()
                } else {
                    "input_changed".into()
                },
                e,
            )
        })?;
    }
    output.raw_exit_code = observation.raw_exit_code;
    let http_result = http.map(crate::http_fixture::Server::finish);
    output.termination_reason = observation.termination;
    output.stdin = StdinResult {
        supplied_bytes: observation.supplied_bytes as u64,
        written_bytes: observation.written_bytes as u64,
    };
    output.stdout = StreamResult {
        captured_bytes: observation.stdout.bytes.len() as u64,
        truncated: observation.stdout.truncated,
    };
    output.stderr = StreamResult {
        captured_bytes: observation.stderr.bytes.len() as u64,
        truncated: observation.stderr.truncated,
    };
    if matches!(
        output.termination_reason,
        Some(TerminationReason::Cancelled | TerminationReason::RunDeadline)
    ) {
        output.status = Status::Inconclusive;
        output.reason_code = Some(termination_code(
            output.termination_reason.as_ref().unwrap(),
        ));
        output.raw_exit_code = None;
        output.unchecked = vec![
            "exit_code".into(),
            "stdin_delivery".into(),
            "lifecycle".into(),
            "workspace_changes".into(),
            "stdout".into(),
            "stderr".into(),
        ];
        output
            .unchecked
            .extend(case.files.iter().map(|f| format!("file:{}", f.path)));
        if case.http.is_some() {
            output.unchecked.push("http_requests".into());
        }
        if !case.extract.is_empty() {
            output.unchecked.push("extraction".into());
        }
        return Ok(());
    }
    let complete = output.termination_reason.is_none();
    if !case.extract.is_empty() {
        let extracted = if complete && !observation.stdout.truncated {
            crate::workflow::extract(&observation.stdout.bytes, &case.extract)
        } else {
            Err("Extraction requires complete stdout".into())
        };
        let mut assertion = if complete {
            check("extraction", extracted.is_ok(), "assertion_mismatch")
        } else {
            unevaluated("extraction", "process_terminated")
        };
        match extracted {
            Ok(values) => {
                if let Some(bindings) = evaluation.bindings {
                    bindings.extend(values);
                }
            }
            Err(message) => assertion.observed_summary = Some(message),
        }
        output.assertions.push(assertion);
    }
    if let Some(result) = http_result {
        let mut assertion = if complete {
            check("http_requests", result.is_ok(), "assertion_mismatch")
        } else {
            unevaluated("http_requests", "process_terminated")
        };
        assertion.observed_summary = result.err();
        output.assertions.push(assertion);
    }
    let mut exit = if complete {
        check(
            "exit_code",
            output.raw_exit_code == Some(case.expect.exit_code),
            "assertion_mismatch",
        )
    } else {
        unevaluated("exit_code", "process_terminated")
    };
    exit.expected_summary = Some(case.expect.exit_code.to_string());
    exit.observed_summary = output.raw_exit_code.map(|c| c.to_string());
    output.assertions.push(exit);
    output.assertions.push(if complete {
        check(
            "stdin_delivery",
            !observation.stdin_incomplete,
            "stdin_incomplete",
        )
    } else {
        unevaluated("stdin_delivery", "process_terminated")
    });
    output.assertions.push(check(
        "lifecycle",
        complete,
        &output
            .termination_reason
            .as_ref()
            .map(termination_code)
            .unwrap_or_default(),
    ));
    for (name, rule, captured) in [
        ("stdout", &case.stdout, &observation.stdout),
        ("stderr", &case.stderr, &observation.stderr),
    ] {
        if let Some(rule) = rule {
            output.assertions.push(crate::assertions::evaluate_stream(
                name,
                rule,
                &captured.bytes,
                complete && !captured.truncated,
                |p| inputs.expected(p).map(Vec::from),
            ));
        }
    }
    // The tree is quiescent and accounted before any persistent-state assertion.
    let after = workspace::snapshot(workspace.path(), deadline)
        .map_err(|e| (e.reason_code.into(), e.to_string()))?;
    if let Some(attempt) = &mut output.attempt {
        attempt.capture(&observation.stdout.bytes, &observation.stderr.bytes, &after);
    }
    if let Some(observer) = evaluation.observer.as_deref_mut() {
        observer
            .observe(
                inputs.index(),
                &observation.stdout.bytes,
                &observation.stderr.bytes,
                &after,
                workspace.path(),
            )
            .map_err(|e| ("comparison_capture_failed".into(), e))?;
    }
    let baseline = evaluation.baseline.unwrap_or_else(|| workspace.baseline());
    output.workspace_deltas = workspace::deltas(baseline, &after);
    let undeclared = workspace::undeclared_changes_with_types(
        baseline,
        &after,
        &output.workspace_deltas,
        &case.files,
    );
    output.assertions.push(check(
        "workspace_changes",
        undeclared.is_empty(),
        "assertion_mismatch",
    ));
    if !undeclared.is_empty() {
        output.assertions.last_mut().unwrap().observed_summary =
            Some(format!("Undeclared changes: {}", undeclared.join(", ")));
    }
    for rule in &case.files {
        let entry = after
            .iter()
            .find(|(p, _)| {
                inputs::ordinal_cmp(&p.replace('\\', "/"), &rule.path.replace('\\', "/")).is_eq()
            })
            .map(|(_, e)| e);
        let passed = match &rule.rule {
            FileKind::Absent => entry.is_none(),
            FileKind::Directory => matches!(entry, Some(Entry::Directory)),
            FileKind::File {
                mode,
                expected_file,
            } => {
                if matches!(entry, Some(Entry::File { .. })) {
                    let observed = inputs::read_bounded(
                        &workspace.path().join(&rule.path),
                        1024 * 1024 * 1024,
                    )
                    .map_err(|e| ("workspace_failed".into(), e))?;
                    let stream = if mode == "exact_file" {
                        StreamRule::ExactFile {
                            expected_file: expected_file.clone(),
                        }
                    } else {
                        StreamRule::JsonEqualsFile {
                            expected_file: expected_file.clone(),
                        }
                    };
                    let evaluation = crate::assertions::evaluate_stream(
                        &format!("file:{}", rule.path),
                        &stream,
                        &observed,
                        true,
                        |p| inputs.expected(p).map(Vec::from),
                    );
                    output.assertions.push(evaluation);
                    continue;
                } else {
                    false
                }
            }
        };
        output.assertions.push(check(
            &format!("file:{}", rule.path),
            passed,
            "assertion_mismatch",
        ));
    }
    output.status = match output.termination_reason {
        Some(TerminationReason::Cancelled | TerminationReason::RunDeadline) => Status::Inconclusive,
        Some(TerminationReason::Infrastructure) => Status::InfraError,
        _ => {
            if output
                .assertions
                .iter()
                .any(|a| a.status == AssertionStatus::Fail)
            {
                Status::Fail
            } else {
                Status::Pass
            }
        }
    };
    output.reason_code = output
        .termination_reason
        .as_ref()
        .map(termination_code)
        .or_else(|| {
            output
                .assertions
                .iter()
                .find(|a| a.status == AssertionStatus::Fail)
                .and_then(|a| a.reason_code.clone())
        });
    Ok(())
}
fn termination_code(reason: &TerminationReason) -> String {
    serde_json::to_value(reason)
        .unwrap()
        .as_str()
        .unwrap()
        .into()
}
fn unevaluated(id: &str, reason: &str) -> AssertionResult {
    AssertionResult {
        check_id: id.into(),
        status: AssertionStatus::NotEvaluated,
        reason_code: Some(reason.into()),
        expected_summary: None,
        observed_summary: None,
    }
}
fn check(id: &str, passed: bool, reason: &str) -> AssertionResult {
    AssertionResult {
        check_id: id.into(),
        status: if passed {
            AssertionStatus::Pass
        } else {
            AssertionStatus::Fail
        },
        reason_code: if passed { None } else { Some(reason.into()) },
        expected_summary: None,
        observed_summary: None,
    }
}

// Step boundaries are framed so two different output partitions cannot compare equal.
#[derive(Default)]
struct ScenarioCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    last: Option<workspace::Snapshot>,
}
impl CaseObserver for ScenarioCapture {
    fn observe(
        &mut self,
        _index: usize,
        stdout: &[u8],
        stderr: &[u8],
        snapshot: &workspace::Snapshot,
        _root: &Path,
    ) -> Result<(), String> {
        if self.stdout.len() + self.stderr.len() + stdout.len() + stderr.len() + 16
            > 64 * 1024 * 1024
        {
            return Err("Scenario private stream capture exceeds 64 MiB".into());
        }
        self.stdout
            .extend_from_slice(&(stdout.len() as u64).to_le_bytes());
        self.stdout.extend_from_slice(stdout);
        self.stderr
            .extend_from_slice(&(stderr.len() as u64).to_le_bytes());
        self.stderr.extend_from_slice(stderr);
        self.last = Some(snapshot.clone());
        Ok(())
    }
}
fn run_scenario(
    plan: &inputs::RunPlan,
    inputs: &CaseInputs,
    case: &Case,
    workspace: &CaseWorkspace,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    evaluation: CaseEvaluation<'_, '_>,
) -> Result<(), (String, String)> {
    let output = evaluation.output;
    let mut bindings = BTreeMap::new();
    let mut capture = ScenarioCapture::default();
    let mut baseline = workspace.baseline().clone();
    let mut stop = false;
    let mut status = Status::Pass;
    let mut pending_error = None;
    for step in &case.steps {
        let mut result = empty(
            step,
            step.timeout_ms.unwrap_or(output.limits.timeout_ms),
            step.max_output_bytes
                .unwrap_or(output.limits.max_output_bytes),
        );
        if stop {
            result.status = Status::Inconclusive;
            result.reason_code = Some(
                if matches!(status, Status::InfraError | Status::Inconclusive) {
                    "not_run_after_abort"
                } else {
                    "not_run_after_step_failure"
                }
                .into(),
            );
        } else {
            let before = Instant::now();
            let mut step_observer: Option<&mut dyn CaseObserver> = Some(&mut capture);
            let evaluated = match crate::workflow::bind(step, &bindings) {
                Ok(mut bound) => {
                    if bound.cwd.is_none() {
                        bound.cwd = case.cwd.clone();
                    }
                    run_case(
                        plan,
                        inputs,
                        &bound,
                        workspace,
                        deadline,
                        cancelled.clone(),
                        CaseEvaluation {
                            output: &mut result,
                            observer: &mut step_observer,
                            baseline: Some(&baseline),
                            bindings: Some(&mut bindings),
                        },
                    )
                }
                Err(message) => {
                    let mut assertion = check("bindings", false, "invalid_binding");
                    assertion.observed_summary = Some(message);
                    result.assertions.push(assertion);
                    result.status = Status::Fail;
                    result.reason_code = Some("invalid_binding".into());
                    Ok(())
                }
            };
            result.duration_ms = before.elapsed().as_millis() as u64;
            if let Err(error) = evaluated {
                result.status = if error.0 == "run_deadline" {
                    Status::Inconclusive
                } else {
                    Status::InfraError
                };
                result.reason_code = Some(error.0.clone());
                if result.status == Status::InfraError {
                    result.termination_reason = Some(TerminationReason::Infrastructure);
                }
                pending_error = Some(error);
            }
            status = Status::aggregate([status, result.status]);
            stop = matches!(result.status, Status::InfraError | Status::Inconclusive)
                || result.assertions.iter().any(|a| {
                    matches!(a.check_id.as_str(), "extraction" | "bindings")
                        && a.status != AssertionStatus::Pass
                })
                || result.status == Status::Fail && !case.continue_on_failure;
            if let Some(snapshot) = &capture.last {
                baseline = snapshot.clone();
            }
            output.raw_exit_code = result.raw_exit_code;
            output.termination_reason = result.termination_reason.clone();
            output.stdin.supplied_bytes += result.stdin.supplied_bytes;
            output.stdin.written_bytes += result.stdin.written_bytes;
            output.stdout.captured_bytes += result.stdout.captured_bytes;
            output.stdout.truncated |= result.stdout.truncated;
            output.stderr.captured_bytes += result.stderr.captured_bytes;
            output.stderr.truncated |= result.stderr.truncated;
        }
        result.checked = result
            .assertions
            .iter()
            .filter(|a| a.status != AssertionStatus::NotEvaluated)
            .map(|a| a.check_id.clone())
            .collect();
        result.unchecked.retain(|id| !result.checked.contains(id));
        for assertion in &result.assertions {
            let mut assertion = assertion.clone();
            assertion.check_id = format!("step:{}/{}", step.id, assertion.check_id);
            output.assertions.push(assertion);
        }
        output.unchecked.extend(
            result
                .unchecked
                .iter()
                .map(|id| format!("step:{}/{id}", step.id)),
        );
        output.steps.push(result);
    }
    output.status = status;
    output.reason_code = output
        .steps
        .iter()
        .find(|s| s.status == status && status != Status::Pass)
        .and_then(|s| s.reason_code.clone());
    output.unchecked.retain(|id| id.starts_with("step:"));
    if let Some(error) = pending_error {
        return Err(error);
    }
    if let Some(snapshot) = &capture.last {
        output.workspace_deltas = workspace::deltas(workspace.baseline(), snapshot);
        if let Some(attempt) = &mut output.attempt {
            attempt.capture(&capture.stdout, &capture.stderr, snapshot);
        }
        if let Some(observer) = evaluation.observer.as_deref_mut() {
            observer
                .observe(
                    inputs.index(),
                    &capture.stdout,
                    &capture.stderr,
                    snapshot,
                    workspace.path(),
                )
                .map_err(|e| ("comparison_capture_failed".into(), e))?;
        }
    }
    Ok(())
}
