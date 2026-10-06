//! Repeatable lifecycle experiments, independent of the suite runner/reporters.
use crate::{
    model::TerminationReason,
    process_windows::{self as process, Observation, Request},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub iteration: usize,
    pub scenario: String,
    pub passed: bool,
    pub diagnostic: Option<String>,
    pub active_members_after_cleanup: Option<u32>,
    pub root_pid: Option<u32>,
    pub residual_processes: Vec<u32>,
    pub member_pids_at_shutdown: Vec<u32>,
    pub handle_count_before: u32,
    pub handle_count_after: u32,
    pub elapsed_ms: u64,
    pub cleanup_ms: Option<u64>,
}
fn descendants(observation: &Observation) -> Vec<u32> {
    std::str::from_utf8(&observation.stdout.bytes)
        .unwrap_or("")
        .lines()
        .filter_map(|line| line.strip_prefix("pid:").and_then(|id| id.parse().ok()))
        .collect()
}
fn experiment(fixture: &Path, iteration: usize, scenario: &str) -> Result<Evidence, String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut env = BTreeMap::new();
    for name in ["SystemRoot", "WINDIR", "COMSPEC"] {
        if let Ok(value) = std::env::var(name) {
            env.insert(name.into(), value);
        }
    }
    env.insert("PATH".into(), String::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut stdin = Vec::new();
    let mut limit = 4096;
    let mut timeout = Duration::from_secs(5);
    let payloads = [
        "",
        "a b",
        "a\"b",
        "\\",
        "trailing\\",
        "back\\\"quote",
        "தமிழ் 😀",
        "&|<>^%!;$()",
    ];
    let marker = directory.path().join("resumed.txt");
    let args: Vec<String> = match scenario {
        "argv" => std::iter::once("argv")
            .chain(payloads)
            .map(str::to_owned)
            .collect(),
        "exit-zero" => vec!["exit".into(), "0".into()],
        "exit-nonzero" => vec!["exit".into(), "37".into()],
        "exit-u32" => vec!["exit".into(), "-1".into()],
        "stdin-eof" => {
            stdin = b"input\0\xff\r\n".to_vec();
            vec!["stdin".into()]
        }
        "binary-streams" => vec!["binary".into()],
        "flood-boundary" => vec!["flood".into(), limit.to_string()],
        "flood-overflow" => vec!["flood".into(), (limit + 1).to_string()],
        "pipe-pressure" => {
            stdin = vec![b'x'; 1024 * 1024];
            limit = 1024;
            vec!["flood".into(), "1048576".into()]
        }
        "early-stdin" => {
            stdin = vec![b'x'; 1024 * 1024];
            vec!["early-stdin".into()]
        }
        "blocked-stdin" => {
            stdin = vec![b'x'; 1024 * 1024];
            timeout = Duration::from_millis(80);
            vec!["sleep".into(), "60000".into()]
        }
        "timeout-tree" => {
            timeout = Duration::from_millis(500);
            vec!["tree".into(), "2".into()]
        }
        "cancel-tree" => {
            let flag = cancelled.clone();
            let ready = directory.path().join("grandchild-ready");
            let marker = ready.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !marker.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(2));
                }
                thread::sleep(Duration::from_millis(30));
                flag.store(true, Ordering::Release);
            });
            vec!["tree".into(), "2".into(), ready.to_string_lossy().into()]
        }
        "run-deadline" => vec!["sleep".into(), "60000".into()],
        "retained-pipe" => vec!["linger".into()],
        "assignment-rejected" => vec!["touch".into(), marker.to_string_lossy().into()],
        _ => return Err("Unknown scenario".into()),
    };
    let before = process::handle_count().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let request = Request {
        executable: fixture,
        args: &args,
        cwd: directory.path(),
        env: &env,
        stdin: &stdin,
        max_output_bytes: limit,
        timeout,
        run_deadline: started
            + if scenario == "run-deadline" {
                Duration::from_millis(80)
            } else {
                Duration::from_secs(10)
            },
        cancelled,
    };
    let observation = if scenario == "assignment-rejected" {
        process::prototype_rejected_assignment(&request)
    } else {
        process::run(&request)
    };
    let mut evidence = Evidence {
        iteration,
        scenario: scenario.into(),
        passed: false,
        diagnostic: None,
        active_members_after_cleanup: None,
        root_pid: None,
        residual_processes: vec![],
        member_pids_at_shutdown: vec![],
        handle_count_before: before,
        handle_count_after: 0,
        elapsed_ms: started.elapsed().as_millis() as u64,
        cleanup_ms: None,
    };
    match observation {
        Err(error)
            if scenario == "assignment-rejected"
                && error.reason_code == "job_assignment_failed" =>
        {
            evidence.passed = !marker.exists();
        }
        Err(error) => evidence.diagnostic = Some(error.to_string()),
        Ok(observation) => {
            evidence.active_members_after_cleanup = Some(observation.active_members_after_cleanup);
            evidence.root_pid = Some(observation.pid);
            evidence.cleanup_ms = Some(observation.cleanup_duration.as_millis() as u64);
            evidence.residual_processes = observation.residual_processes.clone();
            evidence.member_pids_at_shutdown = observation.member_pids_at_shutdown.clone();
            evidence.passed = match scenario {
                "argv" => {
                    observation.termination.is_none()
                        && observation.stdout.bytes
                            == payloads
                                .iter()
                                .map(|a| format!("{}:{a}\n", a.len()))
                                .collect::<String>()
                                .as_bytes()
                }
                "exit-zero" => observation.raw_exit_code == Some(0),
                "exit-nonzero" => observation.raw_exit_code == Some(37),
                "exit-u32" => observation.raw_exit_code == Some(u32::MAX),
                "stdin-eof" => {
                    observation.stdout.bytes == stdin && observation.written_bytes == stdin.len()
                }
                "binary-streams" => {
                    observation.stdout.bytes == [0, 255, 13, 10]
                        && observation.stderr.bytes == b"stderr\n"
                }
                "flood-boundary" => {
                    observation.termination.is_none()
                        && observation.stdout.bytes.len() == limit
                        && observation.stderr.bytes.len() == limit
                        && !observation.stdout.truncated
                        && !observation.stderr.truncated
                }
                "flood-overflow" | "pipe-pressure" => {
                    matches!(
                        observation.termination,
                        Some(TerminationReason::OutputLimit)
                    ) && (observation.stdout.truncated || observation.stderr.truncated)
                        && observation.stdout.bytes.len() <= limit
                        && observation.stderr.bytes.len() <= limit
                }
                "early-stdin" => observation.termination.is_none() && observation.stdin_incomplete,
                "blocked-stdin" => {
                    matches!(observation.termination, Some(TerminationReason::Timeout))
                        && observation.stdin_incomplete
                }
                "timeout-tree" => {
                    matches!(observation.termination, Some(TerminationReason::Timeout))
                        && !descendants(&observation).is_empty()
                }
                "cancel-tree" => {
                    matches!(observation.termination, Some(TerminationReason::Cancelled))
                        && !descendants(&observation).is_empty()
                }
                "run-deadline" => matches!(
                    observation.termination,
                    Some(TerminationReason::RunDeadline)
                ),
                "retained-pipe" => matches!(
                    observation.termination,
                    Some(TerminationReason::LingeringDescendants)
                ),
                _ => false,
            } && evidence.residual_processes.is_empty()
                && evidence.diagnostic.is_none()
                && observation.active_members_after_cleanup == 0;
            if !evidence.passed && evidence.diagnostic.is_none() {
                evidence.diagnostic = Some(format!(
                    "Unexpected observation: status={:?}, termination={:?}, stdout={} stderr={}, stdin={}/{}",
                    observation.raw_exit_code,
                    observation.termination,
                    observation.stdout.bytes.len(),
                    observation.stderr.bytes.len(),
                    observation.written_bytes,
                    observation.supplied_bytes
                ));
            }
        }
    }
    evidence.handle_count_after = process::handle_count().map_err(|e| e.to_string())?;
    // Iteration zero records one-time standard library/OS initialization.
    // Every measured repetition requires an unchanged count.
    if iteration != 0 && evidence.handle_count_after != before {
        evidence.passed = false;
        evidence.diagnostic = Some(format!(
            "Handle count changed: {before} -> {}",
            evidence.handle_count_after
        ));
    }
    Ok(evidence)
}
pub fn run(fixture: &Path, repeat: usize) -> Result<Vec<Evidence>, String> {
    run_with_cancellation(fixture, repeat, &AtomicBool::new(false))
}
pub fn run_with_cancellation(
    fixture: &Path,
    repeat: usize,
    cancelled: &AtomicBool,
) -> Result<Vec<Evidence>, String> {
    if !(1..=1000).contains(&repeat) {
        return Err("Repeat count must be 1..1000".into());
    }
    let mut evidence = Vec::new();
    'repetitions: for iteration in 0..=repeat {
        for scenario in [
            "argv",
            "exit-zero",
            "exit-nonzero",
            "exit-u32",
            "stdin-eof",
            "binary-streams",
            "flood-boundary",
            "flood-overflow",
            "pipe-pressure",
            "early-stdin",
            "blocked-stdin",
            "timeout-tree",
            "cancel-tree",
            "run-deadline",
            "retained-pipe",
            "assignment-rejected",
        ] {
            if cancelled.load(Ordering::Acquire) {
                break 'repetitions;
            }
            let record = experiment(fixture, iteration, scenario)?;
            if !record.passed {
                eprintln!("Prototype failed: {scenario}: {:?}", record.diagnostic);
            }
            evidence.push(record);
        }
        if iteration != 0 && iteration % 10 == 0 {
            eprintln!("Prototype: {iteration}/{repeat} repetitions complete");
        }
    }
    Ok(evidence)
}
