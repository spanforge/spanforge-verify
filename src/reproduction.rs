//! Portable, checksummed input bundles and fail-closed native replay.
use crate::{
    inputs,
    model::RunResult,
    privacy::Masker,
    runner,
    schema::{FileKind, StreamRule},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

const BUNDLE_LIMIT: u64 = 100 * 1024 * 1024;
const MANIFEST_LIMIT: u64 = 4 * 1024 * 1024;
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub runner_version: String,
    pub os: String,
    pub arch: String,
    pub target_sha256: String,
    pub original_suite_hash: String,
    pub case_id: String,
    pub required_env: Vec<String>,
    pub required_env_sha256: BTreeMap<String, String>,
    pub sensitive_inputs_included: bool,
    pub files: BTreeMap<String, String>,
    pub directories: Vec<String>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn preflight_destination(plan: &inputs::RunPlan, out: &Path) -> Result<(), String> {
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    crate::schema::no_reparse(parent)?;
    if !parent.is_dir() || out.file_name().is_none() || fs::symlink_metadata(out).is_ok() {
        return Err("Bundle destination must be new, with an existing directory parent".into());
    }
    let destination = fs::canonicalize(parent)
        .map_err(|e| e.to_string())?
        .join(out.file_name().unwrap());
    let destination = normalized_path(&destination)?;
    if plan.inventory().directories.iter().any(|directory| {
        normalized_path(directory).is_ok_and(|directory| {
            inputs::ordinal_cmp(&destination, &directory).is_eq()
                || inputs::path_descendant(&destination, &directory)
        })
    }) {
        return Err("Bundle destination cannot be inside suite fixture inputs".into());
    }
    // Prove directory creation before a run requests automatic export.
    drop(
        tempfile::tempdir_in(parent)
            .map_err(|_| "Bundle destination parent does not allow directory creation")?,
    );
    Ok(())
}
fn normalized_path(path: &Path) -> Result<String, String> {
    let text = inputs::path_text(path)?;
    #[cfg(windows)]
    let text = text.strip_prefix("//?/").unwrap_or(&text);
    let mut parts = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}
fn put(files: &mut BTreeMap<String, Vec<u8>>, name: &str, bytes: &[u8]) -> Result<(), String> {
    if let Some(previous) = files.get(name) {
        if previous != bytes {
            return Err("Conflicting bundle input destinations".into());
        }
    } else {
        files.insert(name.into(), bytes.into());
    }
    Ok(())
}
fn rewrite_rule(
    rule: &mut StreamRule,
    inputs: &inputs::CaseInputs,
    files: &mut BTreeMap<String, Vec<u8>>,
    counter: &mut usize,
) -> Result<(), String> {
    match rule {
        StreamRule::ExactFile { expected_file } | StreamRule::JsonEqualsFile { expected_file } => {
            *expected_file = expectation(expected_file, inputs, files, counter)?;
        }
        _ => (),
    }
    Ok(())
}
fn expectation(
    reference: &str,
    input: &inputs::CaseInputs,
    files: &mut BTreeMap<String, Vec<u8>>,
    counter: &mut usize,
) -> Result<String, String> {
    let bytes = input
        .expected(reference)
        .ok_or("Missing immutable expectation")?;
    let name = format!("expected/{}.bin", *counter);
    *counter += 1;
    put(files, &name, bytes)?;
    Ok(name)
}

/// Bundling validates and snapshots but never executes the target.
pub fn create(
    file: &Path,
    case: &str,
    out: &Path,
    include_sensitive: bool,
) -> Result<Manifest, (u8, String)> {
    let plan = inputs::prepare(file, Some(case)).map_err(|e| (2, e))?;
    create_from_plan(&plan, case, out, include_sensitive, None)
}
/// Uses actual run snapshots when originals were changed/deleted by the target.
pub fn create_from_plan(
    plan: &inputs::RunPlan,
    case: &str,
    out: &Path,
    include_sensitive: bool,
    failure: Option<&RunResult>,
) -> Result<Manifest, (u8, String)> {
    preflight_destination(plan, out).map_err(|e| (2, e))?;
    let input = plan
        .cases()
        .iter()
        .find(|input| plan.suite().cases[input.index()].id == case)
        .ok_or((
            2,
            "Bundle case is absent from the immutable run plan".into(),
        ))?;
    let mut suite: crate::schema::Suite = portable_suite(plan.suite())
        .map_err(|e| (3, e))?
        .try_into()
        .map_err(|e: toml::de::Error| (3, e.to_string()))?;
    let mut values = suite
        .redact_values_env
        .iter()
        .map(|name| {
            std::env::var(name)
                .map_err(|_| (2, "Missing required masking environment value".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut required_env_sha256 = BTreeMap::new();
    for name in &suite.redact_values_env {
        let value = input
            .environment()
            .iter()
            .find(|(key, _)| inputs::ordinal_cmp(key, name).is_eq())
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| std::env::var(name).expect("validated masking value"));
        if value != std::env::var(name).expect("validated masking value") {
            return Err((2,"Align configured masking and explicit case secret environment values before bundling".into()));
        }
        required_env_sha256.insert(name.clone(), digest(value.as_bytes()));
        values.push(value);
    }
    let masker = Masker::new(values).map_err(|e| (2, e))?;
    let mut selected = suite.cases.remove(input.index());
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    for (name, data) in input.fixture() {
        let name = format!("fixture/{name}");
        match data {
            Some(bytes) => put(&mut files, &name, bytes).map_err(|e| (2, e))?,
            None => {
                directories.insert(name);
            }
        }
    }
    selected.fixture_dir = if selected.fixture_dir.is_some() {
        directories.insert("fixture".into());
        Some("fixture".into())
    } else {
        None
    };
    if selected.steps.is_empty() {
        selected.stdin_text = None;
        selected.stdin_file = Some("stdin.bin".into());
        put(&mut files, "stdin.bin", input.stdin()).map_err(|e| (2, e))?;
    }
    let mut counter = 0;
    for rule in [&mut selected.stdout, &mut selected.stderr]
        .into_iter()
        .flatten()
    {
        rewrite_rule(rule, input, &mut files, &mut counter).map_err(|e| (2, e))?;
    }
    for rule in &mut selected.files {
        if let FileKind::File { expected_file, .. } = &mut rule.rule {
            *expected_file =
                expectation(expected_file, input, &mut files, &mut counter).map_err(|e| (2, e))?;
        }
    }
    for step in &mut selected.steps {
        if let Some(path) = &mut step.stdin_file {
            *path = expectation(path, input, &mut files, &mut counter).map_err(|e| (2, e))?;
        }
        for rule in [&mut step.stdout, &mut step.stderr].into_iter().flatten() {
            rewrite_rule(rule, input, &mut files, &mut counter).map_err(|e| (2, e))?;
        }
        for rule in &mut step.files {
            if let FileKind::File { expected_file, .. } = &mut rule.rule {
                *expected_file = expectation(expected_file, input, &mut files, &mut counter)
                    .map_err(|e| (2, e))?;
            }
        }
    }
    // Freeze ordinary inherited values. Configured secret variables are required
    // by name at replay, never copied into the exported environment.
    let secret_names = suite
        .redact_values_env
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let inherited = selected.inherit_env.clone();
    let is_secret = |name: &str| {
        secret_names
            .iter()
            .any(|secret| inputs::ordinal_cmp(secret, name).is_eq())
    };
    selected.inherit_env.retain(|name| is_secret(name));
    for name in inherited {
        if !is_secret(&name)
            && !selected
                .env
                .keys()
                .any(|key| inputs::ordinal_cmp(key, &name).is_eq())
            && let Some((_, value)) = input
                .environment()
                .iter()
                .find(|(key, _)| inputs::ordinal_cmp(key, &name).is_eq())
        {
            selected.env.insert(name, value.clone());
        }
    }
    for name in &secret_names {
        let expected =
            std::env::var(name).map_err(|_| (2, "Missing masking environment value".into()))?;
        let mut step_explicit = false;
        for step in &mut selected.steps {
            let keys = step
                .env
                .keys()
                .filter(|key| inputs::ordinal_cmp(key, name).is_eq())
                .cloned()
                .collect::<Vec<_>>();
            for key in keys {
                if step.env.get(&key) != Some(&expected) {
                    return Err((
                        2,
                        "Align configured masking and explicit step secret values before bundling"
                            .into(),
                    ));
                }
                step.env.remove(&key);
                step_explicit = true;
            }
        }
        let keys = selected
            .env
            .keys()
            .filter(|key| inputs::ordinal_cmp(key, name).is_eq())
            .cloned()
            .collect::<Vec<_>>();
        let explicit = !keys.is_empty() || step_explicit;
        for key in keys {
            selected.env.remove(&key);
        }
        if explicit
            && !selected
                .inherit_env
                .iter()
                .any(|key| inputs::ordinal_cmp(key, name).is_eq())
        {
            selected.inherit_env.push(name.clone());
        }
    }
    suite.program = "__replay_target__.exe".into();
    for contract in &mut suite.contracts {
        contract.cases.retain(|id| id == &selected.id);
    }
    suite.cases = vec![selected];
    let suite_text = toml::to_string_pretty(&portable_suite(&suite).map_err(|e| (3, e))?)
        .map_err(|e| (3, e.to_string()))?;
    put(&mut files, "suite.toml", suite_text.as_bytes()).map_err(|e| (2, e))?;
    if let Some(failure) = failure {
        if failure.target_hash.as_deref() != Some(plan.target_hash())
            || failure.suite_hash.as_deref() != Some(plan.suite_hash())
        {
            return Err((
                2,
                "Failure report does not match the immutable run plan".into(),
            ));
        }
        let report = serde_json::to_vec_pretty(failure).map_err(|e| (3, e.to_string()))?;
        put(&mut files, "failure.json", &report).map_err(|e| (3, e))?;
    }
    if !include_sensitive
        && (files.values().any(|b| sensitive(b, &masker))
            || files
                .keys()
                .chain(&directories)
                .any(|s| masker.contains_bytes(s.as_bytes())))
    {
        return Err((2,"Bundle inputs contain configured secret values. Supply secrets through named environment variables or explicitly use --include-sensitive-inputs for a private bundle".into()));
    }
    for name in files.keys() {
        let mut parent = Path::new(name).parent();
        while let Some(path) = parent.filter(|p| !p.as_os_str().is_empty()) {
            directories.insert(inputs::path_text(path).map_err(|e| (2, e))?);
            parent = path.parent();
        }
    }
    if files.values().map(|b| b.len() as u64).sum::<u64>() > BUNDLE_LIMIT
        || files.len() + directories.len() > 10_000
    {
        return Err((2, "Bundle exceeds input byte/entry limits".into()));
    }
    let manifest = Manifest {
        schema_version: 1,
        runner_version: env!("CARGO_PKG_VERSION").into(),
        os: crate::reports::os_build(),
        arch: std::env::consts::ARCH.into(),
        target_sha256: plan.target_hash().into(),
        original_suite_hash: plan.suite_hash().into(),
        case_id: case.into(),
        required_env: secret_names.into_iter().collect(),
        required_env_sha256,
        sensitive_inputs_included: include_sensitive,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), digest(bytes)))
            .collect(),
        directories: directories.into_iter().collect(),
    };
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| (3, e.to_string()))?;
    if manifest_bytes.len() as u64 > MANIFEST_LIMIT {
        return Err((2, "Bundle manifest exceeds 4 MiB".into()));
    }
    if !include_sensitive && sensitive(&manifest_bytes, &masker) {
        return Err((
            2,
            "Bundle metadata contains configured secret values".into(),
        ));
    }
    fs::create_dir(out).map_err(|e| {
        (
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                2
            } else {
                3
            },
            e.to_string(),
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(out, fs::Permissions::from_mode(0o700))
            .map_err(|e| (3, e.to_string()))?;
    }
    for name in &manifest.directories {
        fs::create_dir_all(out.join(name)).map_err(|e| (3, e.to_string()))?;
    }
    for (name, bytes) in files {
        let destination = out.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|e| (3, e.to_string()))?;
        }
        write_new(&destination, &bytes).map_err(|e| (3, e))?;
    }
    // Last-written manifest commits the bundle. A partial export is not replayable.
    write_new(&out.join("manifest.json"), &manifest_bytes).map_err(|e| (3, e))?;
    Ok(manifest)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
fn portable_suite(suite: &crate::schema::Suite) -> Result<toml::Value, String> {
    let mut value = toml::Value::try_from(suite).map_err(|e| e.to_string())?;
    fn flatten(value: &mut toml::Value) -> Result<(), String> {
        match value {
            toml::Value::Table(table) => {
                if let Some(files) = table.get_mut("files").and_then(toml::Value::as_array_mut) {
                    for file in files {
                        let table = file.as_table_mut().ok_or("Invalid typed file rule")?;
                        if let Some(toml::Value::Table(rule)) = table.remove("rule") {
                            table.extend(rule);
                        }
                    }
                }
                for (_, child) in table.iter_mut() {
                    flatten(child)?;
                }
            }
            toml::Value::Array(array) => {
                for child in array {
                    flatten(child)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    flatten(&mut value)?;
    Ok(value)
}
fn sensitive(bytes: &[u8], masker: &Masker) -> bool {
    if masker.contains_bytes(bytes) {
        return true;
    }
    fn value(v: &serde_json::Value, masker: &Masker, depth: usize) -> bool {
        match v {
            serde_json::Value::String(s) => {
                masker.contains_bytes(s.as_bytes())
                    || (depth < 4
                        && serde_json::from_str::<serde_json::Value>(s)
                            .is_ok_and(|v| value(&v, masker, depth + 1)))
            }
            serde_json::Value::Array(a) => a.iter().any(|v| value(v, masker, depth)),
            serde_json::Value::Object(o) => o
                .iter()
                .any(|(k, v)| masker.contains_bytes(k.as_bytes()) || value(v, masker, depth)),
            _ => false,
        }
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes)
        && value(&v, masker, 0)
    {
        return true;
    }
    if let Ok(text) = std::str::from_utf8(bytes)
        && let Ok(v) = toml::from_str::<toml::Value>(text)
        && let Ok(v) = serde_json::to_value(v)
    {
        return value(&v, masker, 0);
    }
    false
}

pub fn verify(root: &Path) -> Result<Manifest, String> {
    crate::schema::no_reparse(root)?;
    let manifest_bytes = inputs::read_bounded(&root.join("manifest.json"), MANIFEST_LIMIT)?;
    crate::strict_json::parse(&manifest_bytes).map_err(|_| "Invalid bundle manifest JSON")?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "Invalid bundle manifest schema")?;
    if manifest.schema_version != 1
        || manifest.files.len() + manifest.directories.len() > 10_000
        || !manifest.files.contains_key("suite.toml")
        || manifest.target_sha256.len() != 64
    {
        return Err("Unsupported/incomplete bundle manifest".into());
    }
    let entries = inputs::tree(root)?;
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut total = 0;
    for entry in entries {
        if entry.directory {
            directories.insert(entry.relative);
            continue;
        }
        if entry.relative == "manifest.json" {
            continue;
        }
        let expected = manifest
            .files
            .get(&entry.relative)
            .ok_or("Unexpected bundle file")?;
        let bytes = inputs::read_bounded(&entry.source, BUNDLE_LIMIT.saturating_sub(total))?;
        total += bytes.len() as u64;
        if digest(&bytes) != *expected {
            return Err("Bundle input checksum mismatch".into());
        }
        files.insert(entry.relative);
    }
    if files != manifest.files.keys().cloned().collect() {
        return Err("Missing bundle inputs".into());
    }
    if directories != manifest.directories.iter().cloned().collect() {
        return Err("Bundle directory inventory changed".into());
    }
    for directory in &manifest.directories {
        crate::schema::workspace_path(directory)?;
        if !root.join(directory).is_dir() {
            return Err("Missing bundle fixture directory".into());
        }
    }
    for name in manifest.files.keys() {
        crate::schema::workspace_path(name)?;
    }
    Ok(manifest)
}
pub fn replay(
    root: &Path,
    program: &Path,
    cancelled: Arc<AtomicBool>,
    started: Instant,
    started_at: String,
) -> Result<RunResult, (u8, String)> {
    let manifest = verify(root).map_err(|e| (2, e))?;
    if manifest.os.split_whitespace().next() != Some(std::env::consts::OS)
        || manifest.arch != std::env::consts::ARCH
    {
        return Err((
            2,
            "Bundle platform differs; replay requires the original OS family and architecture"
                .into(),
        ));
    }
    for name in &manifest.required_env {
        if std::env::var(name).is_err() {
            return Err((
                2,
                "Replay requires the bundle's named secret environment inputs".into(),
            ));
        }
        if manifest
            .required_env_sha256
            .get(name)
            .is_none_or(|expected| {
                *expected
                    != digest(
                        std::env::var(name)
                            .expect("checked Unicode environment")
                            .as_bytes(),
                    )
            })
        {
            return Err((
                2,
                "Replay secret environment input differs from the captured value".into(),
            ));
        }
    }
    let plan =
        inputs::prepare_for_program(&root.join("suite.toml"), Some(&manifest.case_id), program)
            .map_err(|e| (2, e))?;
    if plan.target_hash() != manifest.target_sha256 {
        return Err((
            2,
            "Replay executable SHA256 differs from the bundle target".into(),
        ));
    }
    // Reverify after snapshotting to detect changed bundle inputs before launch.
    verify(root).map_err(|e| (2, e))?;
    runner::execute_plan_started(&plan, cancelled, started, started_at, None)
}

pub fn suite_path(root: &Path) -> PathBuf {
    root.join("suite.toml")
}

#[derive(Serialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub ready: bool,
    pub checks: Vec<DoctorCheck>,
}
#[derive(Serialize)]
pub struct DoctorCheck {
    pub id: String,
    pub status: String,
    pub message: String,
}
/// Reports prerequisites together, without launching the target or exposing values.
pub fn doctor(root: &Path, program: &Path) -> DoctorReport {
    let mut report = DoctorReport {
        schema_version: 1,
        ready: true,
        checks: vec![],
    };
    let mut add = |id: &str, pass: bool, message: String| {
        report.ready &= pass;
        report.checks.push(DoctorCheck {
            id: id.into(),
            status: if pass { "pass" } else { "fail" }.into(),
            message,
        });
    };
    let manifest = match verify(root) {
        Ok(manifest) => {
            add(
                "bundle_integrity",
                true,
                "Bundle file and directory inventory verified".into(),
            );
            manifest
        }
        Err(error) => {
            add("bundle_integrity", false, error);
            return report;
        }
    };
    let platform = manifest.os.split_whitespace().next() == Some(std::env::consts::OS)
        && manifest.arch == std::env::consts::ARCH;
    add(
        "platform",
        platform,
        format!(
            "Required {} {}; current {} {}",
            manifest.os,
            manifest.arch,
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    let mut environment_ready = true;
    for name in &manifest.required_env {
        let matched = std::env::var(name).is_ok_and(|v| {
            manifest
                .required_env_sha256
                .get(name)
                .is_some_and(|expected| *expected == digest(v.as_bytes()))
        });
        environment_ready &= matched;
        add(
            &format!("environment:{name}"),
            matched,
            if matched {
                "Required secret input matches captured hash"
            } else {
                "Supply the original secret value through this named environment variable"
            }
            .into(),
        );
    }
    if environment_ready {
        match inputs::prepare_for_program(
            &root.join("suite.toml"),
            Some(&manifest.case_id),
            program,
        ) {
            Ok(plan) => add(
                "target_and_suite",
                plan.target_hash() == manifest.target_sha256,
                "Native target and suite validated; executable hash must match captured target"
                    .into(),
            ),
            Err(error) => add("target_and_suite", false, error),
        }
    } else {
        report.checks.push(DoctorCheck {
            id: "target_and_suite".into(),
            status: "not_checked".into(),
            message: "Supply required secret inputs to finish native target/suite validation"
                .into(),
        });
    }
    report
}
