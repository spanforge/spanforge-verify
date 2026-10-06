//! Reviewed, hash-pinned outcome checks for cooperative process targets.
use crate::{inputs, model::*, schema};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use crate::process_linux as process;
#[cfg(windows)]
use crate::process_windows as process;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Verifier {
    pub id: String,
    pub schema_version: u32,
    pub executable: PinnedFile,
    pub args: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, PinnedFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualification: Option<Qualification>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_qualification: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Qualification {
    pub schema_version: u32,
    pub repeat: u32,
    pub controls: Vec<Control>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub id: String,
    pub kind: ControlKind,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Reference,
    NoOp,
    Defect,
    Alternate,
}
impl ControlKind {
    fn expected(self) -> AssertionStatus {
        if matches!(self, Self::Reference | Self::Alternate) {
            AssertionStatus::Pass
        } else {
            AssertionStatus::Fail
        }
    }
}
fn validate_qualification(q: &Qualification) -> Result<(), String> {
    if q.schema_version != 1
        || !(2..=5).contains(&q.repeat)
        || !(4..=32).contains(&q.controls.len())
    {
        return Err("Invalid qualification version or limits".into());
    }
    for kind in [
        ControlKind::Reference,
        ControlKind::NoOp,
        ControlKind::Defect,
        ControlKind::Alternate,
    ] {
        if !q.controls.iter().any(|c| c.kind == kind) {
            return Err(
                "Qualification requires reference, no_op, defect and alternate controls".into(),
            );
        }
    }
    let mut ids = HashSet::new();
    let mut snapshots = BTreeMap::new();
    let mut total = 0;
    for control in &q.controls {
        if !valid_id(&control.id)
            || !ids.insert(&control.id)
            || control.files.len() > 64
            || !inputs::unique_names(control.files.keys().map(String::as_str))
        {
            return Err("Invalid qualification control IDs or files".into());
        }
        for (path, text) in &control.files {
            schema::workspace_path(path)?;
            if path.contains('\\') {
                return Err("Qualification paths require forward slashes".into());
            }
            if control
                .files
                .keys()
                .any(|other| inputs::path_descendant(other, path))
            {
                return Err("Qualification file path conflicts with directory".into());
            }
            total += text.len();
            if total > 1024 * 1024 {
                return Err("Qualification inputs exceed 1 MiB".into());
            }
        }
        let identity = serde_json::to_string(&control.files).map_err(|e| e.to_string())?;
        if snapshots
            .insert(identity, control.kind.expected())
            .is_some_and(|old| old != control.kind.expected())
        {
            return Err("Contradictory qualification controls".into());
        }
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn validate(base: &Path, suite: &schema::Suite) -> Result<(), String> {
    if suite.verifiers.len() > 32 {
        return Err("Too many verifiers".into());
    }
    let mut ids = HashSet::new();
    for verifier in &suite.verifiers {
        if !valid_id(&verifier.id) || !ids.insert(&verifier.id) || verifier.schema_version != 1 {
            return Err("Invalid verifier ID or schema version".into());
        }
        if !(1..=60_000).contains(&verifier.timeout_ms)
            || !(1..=1024 * 1024).contains(&verifier.max_output_bytes)
            || verifier.args.len() > 256
            || verifier.dependencies.len() > 32
        {
            return Err("Invalid verifier limits".into());
        }
        let executable = schema::source(base, &verifier.executable.path, false)?;
        #[cfg(windows)]
        if !executable
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Err("Verifier requires a native executable".into());
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut magic = [0; 4];
            File::open(&executable)
                .and_then(|mut f| f.read_exact(&mut magic))
                .map_err(|e| e.to_string())?;
            if magic != *b"\x7fELF"
                || fs::metadata(&executable)
                    .map_err(|e| e.to_string())?
                    .permissions()
                    .mode()
                    & 0o111
                    == 0
            {
                return Err("Verifier requires native ELF executable permission".into());
            }
        }
        for (name, pinned) in &verifier.dependencies {
            if !valid_id(name) {
                return Err("Invalid verifier dependency name".into());
            }
            schema::source(base, &pinned.path, false)?;
        }
        for pinned in std::iter::once(&verifier.executable).chain(verifier.dependencies.values()) {
            if pinned.sha256.len() != 64
                || !pinned
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("Verifier requires lowercase SHA256 identities".into());
            }
        }
        resolve_args(base, verifier)?;
        if let Some(q) = &verifier.qualification {
            validate_qualification(q)?;
        }
    }
    for case in &suite.cases {
        if !case.steps.is_empty() && !case.verify.is_empty() {
            return Err("Declare scenario verifiers on individual steps".into());
        }
        for case in std::iter::once(case).chain(&case.steps) {
            if !inputs::unique_names(case.verify.iter().map(String::as_str))
                || case.verify.iter().any(|id| !ids.contains(id))
            {
                return Err("Unknown or duplicate case verifier".into());
            }
        }
    }
    // Verify identities during validation, including unselected cases.
    prepare(base, &suite.verifiers)?;
    Ok(())
}

fn resolve_args(base: &Path, verifier: &Verifier) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    for arg in &verifier.args {
        if arg.contains('\0') {
            return Err("NUL in verifier argv".into());
        }
        let resolved = if let Some(name) = arg
            .strip_prefix("{{dependency.")
            .and_then(|s| s.strip_suffix("}}"))
        {
            let pinned = verifier
                .dependencies
                .get(name)
                .ok_or("Unknown verifier dependency")?;
            inputs::path_text(
                &fs::canonicalize(schema::source(base, &pinned.path, false)?)
                    .map_err(|e| e.to_string())?,
            )?
        } else {
            if arg.contains("{{") {
                return Err("Verifier arguments permit only whole dependency references".into());
            }
            arg.clone()
        };
        args.push(resolved);
    }
    #[cfg(windows)]
    {
        let exe = inputs::path_text(
            &fs::canonicalize(schema::source(base, &verifier.executable.path, false)?)
                .map_err(|e| e.to_string())?,
        )?;
        let units = process::quote(&exe).encode_utf16().count()
            + args
                .iter()
                .map(|arg| 1 + process::quote(arg).encode_utf16().count())
                .sum::<usize>()
            + 1;
        if units > 32767 {
            return Err("Verifier command line exceeds limit".into());
        }
    }
    Ok(args)
}

struct LockedFile {
    path: PathBuf,
    file: File,
    sha256: String,
}
impl LockedFile {
    fn open(base: &Path, pinned: &PinnedFile) -> Result<Self, String> {
        let path = fs::canonicalize(schema::source(base, &pinned.path, false)?)
            .map_err(|e| e.to_string())?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let file = options.open(&path).map_err(|e| e.to_string())?;
        let locked = Self {
            path,
            file,
            sha256: pinned.sha256.clone(),
        };
        locked.check(Instant::now() + Duration::from_secs(60))?;
        Ok(locked)
    }
    fn check(&self, deadline: Instant) -> Result<(), String> {
        schema::no_reparse(&self.path)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let current = fs::metadata(&self.path).map_err(|e| e.to_string())?;
            let held = self.file.metadata().map_err(|e| e.to_string())?;
            if current.dev() != held.dev() || current.ino() != held.ino() {
                return Err("verifier_input_changed".into());
            }
        }
        let mut file = self.file.try_clone().map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        let mut size = 0u64;
        loop {
            if Instant::now() >= deadline {
                return Err("run_deadline".into());
            }
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            size += n as u64;
            if size > inputs::INPUT_BUDGET {
                return Err("Verifier input exceeds 100 MiB".into());
            }
            hash.update(&buffer[..n]);
        }
        if format!("{:x}", hash.finalize()) != self.sha256 {
            return Err("Verifier input SHA256 mismatch".into());
        }
        Ok(())
    }
}

pub struct Prepared {
    executable: LockedFile,
    dependencies: Vec<LockedFile>,
    args: Vec<String>,
    timeout_ms: u64,
    max_output_bytes: usize,
    qualification: Option<Qualification>,
    require_qualification: bool,
    health: std::cell::RefCell<Option<(String, Vec<AssertionResult>)>>,
}
pub fn prepare(
    base: &Path,
    definitions: &[Verifier],
) -> Result<BTreeMap<String, Prepared>, String> {
    let mut distinct = HashSet::new();
    let mut bytes = 0u64;
    for pinned in definitions
        .iter()
        .flat_map(|v| std::iter::once(&v.executable).chain(v.dependencies.values()))
    {
        let path = schema::source(base, &pinned.path, false)?;
        if distinct.insert(inputs::file_identity(&path)?) {
            bytes = bytes.saturating_add(fs::metadata(path).map_err(|e| e.to_string())?.len());
            if bytes > inputs::INPUT_BUDGET {
                return Err("Verifier inputs exceed 100 MiB".into());
            }
        }
    }
    definitions
        .iter()
        .map(|v| {
            Ok((
                v.id.clone(),
                Prepared {
                    executable: LockedFile::open(base, &v.executable)?,
                    dependencies: v
                        .dependencies
                        .values()
                        .map(|p| LockedFile::open(base, p))
                        .collect::<Result<_, _>>()?,
                    args: resolve_args(base, v)?,
                    timeout_ms: v.timeout_ms,
                    max_output_bytes: v.max_output_bytes as usize,
                    qualification: v.qualification.clone(),
                    require_qualification: v.require_qualification,
                    health: std::cell::RefCell::new(None),
                },
            ))
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    schema_version: u32,
    status: Verdict,
    summary: String,
}
#[derive(Deserialize)]
enum Verdict {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "FAIL")]
    Fail,
    #[serde(rename = "INCONCLUSIVE")]
    Inconclusive,
}

impl Prepared {
    pub(crate) fn evaluate(
        &self,
        id: &str,
        workspace: &Path,
        run_id: &str,
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(AssertionResult, Vec<AssertionResult>), (String, String)> {
        let health = self.qualify(id, workspace, run_id, deadline, cancelled.clone())?;
        let qualified =
            !health.is_empty() && health.iter().all(|a| a.status == AssertionStatus::Pass);
        if (self.qualification.is_some() || self.require_qualification) && !qualified {
            return Ok((
                AssertionResult {
                    check_id: format!("verifier:{id}"),
                    status: AssertionStatus::NotEvaluated,
                    reason_code: Some(
                        health
                            .iter()
                            .find_map(|a| {
                                a.reason_code
                                    .as_deref()
                                    .filter(|r| matches!(*r, "cancelled" | "run_deadline"))
                            })
                            .unwrap_or("evaluator_unqualified")
                            .into(),
                    ),
                    expected_summary: Some(format!(
                        "Independent executable check; qualification={}; SHA256={}",
                        if self.qualification.is_some() {
                            "failed"
                        } else {
                            "missing"
                        },
                        self.executable.sha256
                    )),
                    observed_summary: Some(
                        "Mandatory evaluator did not qualify; task outcome was not graded".into(),
                    ),
                },
                health,
            ));
        }
        let mut finding = self.evaluate_raw(id, workspace, deadline, cancelled)?;
        if qualified {
            finding.expected_summary = finding.expected_summary.map(|s| {
                s.replace(
                    "qualification=not_evaluated",
                    "qualification=qualified_controls",
                )
            });
        }
        Ok((finding, health))
    }

    fn qualify(
        &self,
        id: &str,
        workspace: &Path,
        run_id: &str,
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Vec<AssertionResult>, (String, String)> {
        let Some(q) = &self.qualification else {
            return Ok(vec![]);
        };
        if let Some((run, results)) = &*self.health.borrow()
            && run == run_id
        {
            return Ok(results.clone());
        }
        let mut results = Vec::new();
        let mut stop = false;
        for control in &q.controls {
            for attempt in 1..=q.repeat {
                let check_id = format!("qualification:{id}:{}:{attempt:03}", control.id);
                let mut result = AssertionResult {
                    check_id,
                    status: AssertionStatus::NotEvaluated,
                    reason_code: Some("not_run_after_qualification_error".into()),
                    expected_summary: Some(format!(
                        "Control {:?}; expected {:?}; verifier SHA256={}",
                        control.kind,
                        control.kind.expected(),
                        self.executable.sha256
                    )),
                    observed_summary: None,
                };
                if !stop {
                    if cancelled.load(std::sync::atomic::Ordering::SeqCst)
                        || Instant::now() >= deadline
                    {
                        result.reason_code = Some(
                            if Instant::now() >= deadline {
                                "run_deadline"
                            } else {
                                "cancelled"
                            }
                            .into(),
                        );
                        stop = true;
                    } else {
                        let temp = tempfile::tempdir_in(workspace.parent().ok_or((
                            "qualification_failed".into(),
                            "Missing workspace parent".into(),
                        ))?)
                        .map_err(|e| ("qualification_failed".into(), e.to_string()))?;
                        let root = temp.path().join("workspace");
                        fs::create_dir(&root)
                            .map_err(|e| ("qualification_failed".into(), e.to_string()))?;
                        for (path, text) in &control.files {
                            let path = root.join(path);
                            fs::create_dir_all(path.parent().unwrap())
                                .and_then(|_| fs::write(path, text))
                                .map_err(|e| ("qualification_failed".into(), e.to_string()))?;
                        }
                        let before = crate::workspace::snapshot(&root, deadline)
                            .map_err(|e| (e.reason_code.into(), e.to_string()))?;
                        let observed = self.evaluate_raw(id, &root, deadline, cancelled.clone())?;
                        let after = crate::workspace::snapshot(&root, deadline)
                            .map_err(|e| (e.reason_code.into(), e.to_string()))?;
                        temp.close()
                            .map_err(|e| ("cleanup_failed".into(), e.to_string()))?;
                        result.observed_summary = Some(format!(
                            "Observed {:?}; {}; {}",
                            observed.status,
                            observed.reason_code.as_deref().unwrap_or("completed"),
                            observed.observed_summary.as_deref().unwrap_or("No summary")
                        ));
                        if observed.status == AssertionStatus::NotEvaluated || before != after {
                            result.reason_code = Some(
                                observed
                                    .reason_code
                                    .as_deref()
                                    .filter(|r| matches!(*r, "cancelled" | "run_deadline"))
                                    .unwrap_or("qualification_evaluator_error")
                                    .into(),
                            );
                            stop = true;
                        } else if observed.status == control.kind.expected() {
                            result.status = AssertionStatus::Pass;
                            result.reason_code = None;
                        } else {
                            result.status = AssertionStatus::Fail;
                            result.reason_code =
                                Some(
                                    if matches!(
                                        control.kind,
                                        ControlKind::NoOp | ControlKind::Defect
                                    ) {
                                        "qualification_defect_escaped"
                                    } else {
                                        "qualification_valid_rejected"
                                    }
                                    .into(),
                                );
                        }
                    }
                }
                results.push(result);
            }
        }
        *self.health.borrow_mut() = Some((run_id.into(), results.clone()));
        Ok(results)
    }

    fn evaluate_raw(
        &self,
        id: &str,
        workspace: &Path,
        deadline: Instant,
        cancelled: Arc<AtomicBool>,
    ) -> Result<AssertionResult, (String, String)> {
        let mut result = AssertionResult {
            check_id: format!("verifier:{id}"),
            status: AssertionStatus::NotEvaluated,
            reason_code: Some("evaluator_error".into()),
            expected_summary: Some(format!(
                "Independent executable check; qualification=not_evaluated; SHA256={}",
                self.executable.sha256
            )),
            observed_summary: None,
        };
        for file in std::iter::once(&self.executable).chain(&self.dependencies) {
            if let Err(e) = file.check(deadline) {
                result.observed_summary = Some(e);
                return Ok(result);
            }
        }
        let temp = tempfile::tempdir_in(
            workspace
                .parent()
                .ok_or(("verifier_failed".into(), "Missing workspace parent".into()))?,
        )
        .map_err(|e| ("verifier_failed".into(), e.to_string()))?;
        let mut env = BTreeMap::from([("PATH".into(), String::new())]);
        #[cfg(windows)]
        for name in ["SystemRoot", "WINDIR"] {
            if let Ok(value) = std::env::var(name) {
                env.insert(name.into(), value);
            }
        }
        for name in ["TEMP", "TMP", "TMPDIR", "HOME", "USERPROFILE"] {
            env.insert(
                name.into(),
                inputs::path_text(temp.path()).map_err(|e| ("verifier_failed".into(), e))?,
            );
        }
        let stdin = serde_json::to_vec(&serde_json::json!({"schema_version":1,"workspace":inputs::path_text(workspace).map_err(|e| ("verifier_failed".into(), e))?})).map_err(|e| ("verifier_failed".into(),e.to_string()))?;
        let observed = process::run(&process::Request {
            executable: &self.executable.path,
            #[cfg(target_os = "linux")]
            executable_file: &self.executable.file,
            args: &self.args,
            cwd: temp.path(),
            env: &env,
            stdin: &stdin,
            max_output_bytes: self.max_output_bytes,
            timeout: Duration::from_millis(self.timeout_ms),
            run_deadline: deadline,
            cancelled,
        })
        .map_err(|e| (e.reason_code.into(), e.to_string()))?;
        if matches!(
            observed.termination,
            Some(TerminationReason::Cancelled | TerminationReason::RunDeadline)
        ) {
            result.reason_code = Some(
                if matches!(observed.termination, Some(TerminationReason::Cancelled)) {
                    "cancelled"
                } else {
                    "run_deadline"
                }
                .into(),
            );
            return Ok(result);
        }
        for file in std::iter::once(&self.executable).chain(&self.dependencies) {
            if let Err(e) = file.check(deadline) {
                result.observed_summary = Some(e);
                return Ok(result);
            }
        }
        if observed.termination.is_some()
            || observed.raw_exit_code != Some(0)
            || observed.stdin_incomplete
            || observed.stdout.truncated
            || observed.stderr.truncated
        {
            result.observed_summary = Some(format!(
                "Verifier did not complete: exit={:?}, termination={:?}",
                observed.raw_exit_code, observed.termination
            ));
            return Ok(result);
        }
        let reply = crate::strict_json::parse(&observed.stdout.bytes)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                serde_json::from_slice::<Reply>(&observed.stdout.bytes)
                    .map_err(|_| "Invalid verifier response".into())
            });
        match reply {
            Ok(reply) if reply.schema_version == 1 && reply.summary.len() <= 4096 => {
                result.observed_summary = Some(reply.summary);
                (result.status, result.reason_code) = match reply.status {
                    Verdict::Pass => (AssertionStatus::Pass, None),
                    Verdict::Fail => (
                        AssertionStatus::Fail,
                        Some("verified_outcome_failure".into()),
                    ),
                    Verdict::Inconclusive => (
                        AssertionStatus::NotEvaluated,
                        Some("verifier_inconclusive".into()),
                    ),
                };
            }
            _ => result.observed_summary = Some("Invalid or oversized verifier response".into()),
        }
        Ok(result)
    }
}

#[derive(Serialize)]
pub struct HealthReport {
    pub schema_version: u32,
    pub run_id: String,
    pub suite_hash: String,
    pub status: Status,
    pub exit_code: u8,
    pub details_omitted: u64,
    pub verifiers: Vec<VerifierHealth>,
    pub note: &'static str,
}
#[derive(Serialize)]
pub struct VerifierHealth {
    pub id: String,
    pub executable_sha256: String,
    pub declaration_sha256: String,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
    pub repeat: Option<u32>,
    pub status: Status,
    pub controls: Vec<AssertionResult>,
}
/// Runs reviewed oracle controls without launching the evaluated target.
pub fn health_report(
    file: &Path,
    selected: Option<&str>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
) -> Result<HealthReport, (u8, String)> {
    let plan = inputs::prepare(file, None).map_err(|e| (2, e))?;
    if plan.suite().verifiers.is_empty()
        || selected.is_some_and(|id| !plan.suite().verifiers.iter().any(|v| v.id == id))
    {
        return Err((2, "No matching verifier declarations".into()));
    }
    let masker = crate::privacy::Masker::new(
        plan.suite()
            .redact_values_env
            .iter()
            .map(|n| std::env::var(n).map_err(|e| (2, e.to_string())))
            .collect::<Result<Vec<_>, _>>()?,
    )
    .map_err(|e| (2, e))?;
    let parent = std::env::var_os("SPANFORGE_VERIFY_WORK_ROOT")
        .or_else(|| std::env::var_os("CLIVERIFYR_WORK_ROOT"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let temp = tempfile::tempdir_in(parent).map_err(|e| (3, e.to_string()))?;
    let workspace = temp.path().join("workspace");
    fs::create_dir(&workspace).map_err(|e| (3, e.to_string()))?;
    let deadline = started + Duration::from_millis(plan.suite().limits.run_timeout_ms);
    let mut report = HealthReport {
        schema_version: 1,
        run_id: crate::reports::run_id(),
        suite_hash: plan.suite_hash().into(),
        status: Status::Pass,
        exit_code: 0,
        details_omitted: 0,
        verifiers: vec![],
        note: "Qualification applies to declared controls and verdict repeats only; it does not prove general oracle correctness. No evaluated target was launched.",
    };
    for definition in &plan.suite().verifiers {
        if selected.is_some_and(|id| id != definition.id) {
            continue;
        }
        let prepared = plan.verifier(&definition.id);
        let mut controls = prepared
            .qualify(
                &definition.id,
                &workspace,
                &report.run_id,
                deadline,
                cancelled.clone(),
            )
            .map_err(|(reason, message)| (if reason == "run_deadline" { 4 } else { 3 }, message))?;
        if controls.is_empty() {
            controls.push(AssertionResult {
                check_id: format!("qualification:{}", definition.id),
                status: AssertionStatus::NotEvaluated,
                reason_code: Some("qualification_missing".into()),
                expected_summary: None,
                observed_summary: None,
            });
        }
        let status = if controls
            .iter()
            .any(|a| a.status == AssertionStatus::NotEvaluated)
        {
            Status::Inconclusive
        } else if controls.iter().any(|a| a.status == AssertionStatus::Fail) {
            Status::Fail
        } else {
            Status::Pass
        };
        for control in &mut controls {
            control.check_id = masker.mask(&control.check_id);
            for summary in [&mut control.expected_summary, &mut control.observed_summary]
                .into_iter()
                .flatten()
            {
                let diagnostic = masker.diagnostic(summary);
                report.details_omitted += u64::from(diagnostic.truncated);
                *summary = diagnostic.text;
            }
        }
        report.verifiers.push(VerifierHealth {
            id: masker.mask(&definition.id),
            executable_sha256: prepared.executable.sha256.clone(),
            declaration_sha256: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(definition).map_err(|e| (3, e.to_string()))?)
            ),
            timeout_ms: definition.timeout_ms,
            max_output_bytes: definition.max_output_bytes,
            repeat: definition.qualification.as_ref().map(|q| q.repeat),
            status,
            controls,
        });
    }
    temp.close().map_err(|e| (3, e.to_string()))?;
    report.status = Status::aggregate(report.verifiers.iter().map(|v| v.status));
    report.exit_code = report.status.exit_code();
    Ok(report)
}
