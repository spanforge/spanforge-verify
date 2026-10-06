//! Version 1 wire types shared by the runner and its reporters.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Pass,
    Fail,
    ConfigError,
    InfraError,
    Inconclusive,
}
impl Status {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Pass => 0,
            Self::Fail => 1,
            Self::ConfigError => 2,
            Self::InfraError => 3,
            Self::Inconclusive => 4,
        }
    }
    pub fn aggregate(cases: impl IntoIterator<Item = Self>) -> Self {
        cases
            .into_iter()
            .max_by_key(|s| match s {
                Self::Pass => 0,
                Self::Fail => 1,
                Self::Inconclusive => 2,
                Self::ConfigError => 3,
                Self::InfraError => 4,
            })
            .unwrap_or(Self::ConfigError)
    }
}
impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::ConfigError => "CONFIG_ERROR",
            Self::InfraError => "INFRA_ERROR",
            Self::Inconclusive => "INCONCLUSIVE",
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssertionStatus {
    Pass,
    Fail,
    NotEvaluated,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertionResult {
    pub check_id: String,
    pub status: AssertionStatus,
    pub reason_code: Option<String>,
    pub expected_summary: Option<String>,
    pub observed_summary: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLimits {
    pub run_timeout_ms: u64,
    pub max_cases: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseLimits {
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StdinResult {
    pub supplied_bytes: u64,
    pub written_bytes: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamResult {
    pub captured_bytes: u64,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    Signal,
    Timeout,
    OutputLimit,
    LingeringDescendants,
    Cancelled,
    RunDeadline,
    Infrastructure,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Created,
    Removed,
    Modified,
    TypeChanged,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDelta {
    pub path: String,
    pub change: Change,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunError {
    pub reason_code: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<CaseResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<crate::repeatability::Evidence>,
    pub case_id: String,
    pub status: Status,
    pub reason_code: Option<String>,
    pub raw_exit_code: Option<u32>,
    pub termination_reason: Option<TerminationReason>,
    pub duration_ms: u64,
    pub limits: CaseLimits,
    pub stdin: StdinResult,
    pub stdout: StreamResult,
    pub stderr: StreamResult,
    pub assertions: Vec<AssertionResult>,
    pub workspace_deltas: Vec<WorkspaceDelta>,
    pub checked: Vec<String>,
    pub unchecked: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Internal cooperative cleanup/report budget; never part of schema v1.
    #[serde(skip)]
    pub publication_deadline: Option<std::time::Instant>,
    pub schema_version: u32,
    pub run_id: String,
    pub suite_id: Option<String>,
    pub suite_hash: Option<String>,
    pub target_hash: Option<String>,
    pub runner_version: String,
    pub os: String,
    pub arch: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub status: Status,
    pub exit_code: u8,
    pub limits: RunLimits,
    pub errors: Vec<RunError>,
    pub cases: Vec<CaseResult>,
    pub details_omitted: u64,
}
