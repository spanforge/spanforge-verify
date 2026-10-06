//! Bounded input inventory and runner-owned immutable snapshots.
use crate::schema::{Case, FileKind, StreamRule, Suite, no_reparse, source, workspace_path};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const INPUT_BUDGET: u64 = 100 * 1024 * 1024;
pub const ENTRY_LIMIT: usize = 10_000;

pub fn ordinal_cmp(a: &str, b: &str) -> Ordering {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{CSTR_EQUAL, CSTR_LESS_THAN, CompareStringOrdinal};
        let a: Vec<_> = a.encode_utf16().collect();
        let b: Vec<_> = b.encode_utf16().collect();
        // SAFETY: both slices remain valid for this synchronous comparison.
        let result = unsafe { CompareStringOrdinal(&a, &b, true) };
        if result == CSTR_EQUAL {
            Ordering::Equal
        } else if result == CSTR_LESS_THAN {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
    #[cfg(not(windows))]
    {
        a.cmp(b)
    }
}
pub fn unique_names<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    let mut names: Vec<_> = names.into_iter().collect();
    names.sort_by(|a, b| ordinal_cmp(a, b));
    !names
        .windows(2)
        .any(|p| ordinal_cmp(p[0], p[1]) == Ordering::Equal)
}
pub fn path_descendant(child: &str, parent: &str) -> bool {
    let child: Vec<_> = child.split(['/', '\\']).collect();
    let parent: Vec<_> = parent.split(['/', '\\']).collect();
    child.len() > parent.len()
        && child
            .iter()
            .zip(parent.iter())
            .all(|(a, b)| ordinal_cmp(a, b).is_eq())
}
pub fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|s| {
            if cfg!(windows) {
                s.replace('\\', "/")
            } else {
                s.to_owned()
            }
        })
        .ok_or_else(|| "Non-Unicode input path".into())
}
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    no_reparse(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Prevent concurrent writers/deletion while reading the opened input.
        options.share_mode(1);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("Cannot open input: {e}"))?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() {
        return Err("Input is not a regular file".into());
    }
    if before.len() > limit {
        return Err("Input exceeds byte budget".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Input exceeds byte budget".into());
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != after.len()
    {
        return Err("input_changed".into());
    }
    // Re-read the held file and compare content, including same-length changes.
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut other = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut other)
        .map_err(|e| e.to_string())?;
    if other != bytes {
        return Err("input_changed".into());
    }
    Ok(bytes)
}
pub(crate) fn file_identity(path: &Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
        };
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: handle belongs to live File; output is valid initialized storage.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        path_text(&fs::canonicalize(path).map_err(|e| e.to_string())?)
    }
}
#[derive(Debug)]
pub struct TreeEntry {
    pub relative: String,
    pub source: PathBuf,
    pub directory: bool,
}
pub fn tree(root: &Path) -> Result<Vec<TreeEntry>, String> {
    no_reparse(root)?;
    let mut entries = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in
            fs::read_dir(directory).map_err(|e| format!("Cannot read fixture tree: {e}"))?
        {
            let path = entry.map_err(|e| e.to_string())?.path();
            no_reparse(&path)?;
            let relative = path_text(path.strip_prefix(root).map_err(|e| e.to_string())?)?;
            workspace_path(&relative)?;
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !metadata.is_file() && !metadata.is_dir() {
                return Err("Unsupported fixture entry".into());
            }
            if metadata.is_dir() {
                pending.push(path.clone());
            }
            entries.push(TreeEntry {
                relative,
                source: path,
                directory: metadata.is_dir(),
            });
            if entries.len() > ENTRY_LIMIT {
                return Err("Fixture exceeds 10000 entries".into());
            }
        }
    }
    if !unique_names(entries.iter().map(|e| e.relative.as_str())) {
        return Err("Fixture path collision".into());
    }
    entries.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(entries)
}
fn referenced(case: &Case) -> Vec<(&str, bool)> {
    let mut refs = Vec::new();
    if let Some(stdin) = &case.stdin_file {
        refs.push((stdin.as_str(), false));
    }
    for rule in [&case.stdout, &case.stderr].into_iter().flatten() {
        match rule {
            StreamRule::ExactFile { expected_file } => refs.push((expected_file.as_str(), false)),
            StreamRule::JsonEqualsFile { expected_file } => {
                refs.push((expected_file.as_str(), true))
            }
            _ => (),
        }
    }
    for rule in &case.files {
        if let FileKind::File {
            mode,
            expected_file,
        } = &rule.rule
        {
            refs.push((expected_file.as_str(), mode == "json_equals_file"));
        }
    }
    for step in &case.steps {
        refs.extend(referenced(step));
    }
    refs
}
pub struct Inventory {
    pub(crate) files: BTreeMap<PathBuf, String>,
    pub(crate) directories: BTreeSet<PathBuf>,
}
pub fn validate_inputs(base: &Path, suite: &Suite) -> Result<Inventory, String> {
    let mut distinct = BTreeMap::<String, u64>::new();
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    let mut total = 0u64;
    let mut account = |path: &Path, json: bool| -> Result<(), String> {
        let key = file_identity(path)?;
        let limit = if let Some(size) = distinct.get(&key) {
            *size
        } else {
            INPUT_BUDGET.saturating_sub(total)
        };
        let bytes = read_bounded(path, limit)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if files
            .insert(path.to_path_buf(), digest.clone())
            .is_some_and(|old| old != digest)
        {
            return Err("input_changed".into());
        }
        distinct.entry(key).or_insert_with(|| {
            total += bytes.len() as u64;
            bytes.len() as u64
        });
        if json {
            crate::strict_json::parse(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    for case in &suite.cases {
        for (reference, json) in referenced(case) {
            account(&source(base, reference, false)?, json)?;
        }
        if let Some(fixture) = &case.fixture_dir {
            let root = source(base, fixture, true)?;
            directories.insert(root.clone());
            for entry in tree(&root)? {
                if !entry.directory {
                    account(&entry.source, false)?;
                } else {
                    directories.insert(entry.source);
                }
            }
        }
    }
    Ok(Inventory { files, directories })
}

/// Fields are private so callers cannot mutate a validated plan.
pub struct RunPlan {
    suite: Suite,
    executable: PathBuf,
    cases: Vec<CaseInputs>,
    suite_hash: String,
    target_hash: String,
    _target_lock: File,
    inventory: Inventory,
}
#[derive(Clone)]
pub struct CaseInputs {
    index: usize,
    fixture: BTreeMap<String, Option<Arc<[u8]>>>,
    expected: BTreeMap<String, Arc<[u8]>>,
    stdin: Arc<[u8]>,
    environment: BTreeMap<String, String>,
}
impl CaseInputs {
    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn fixture(&self) -> &BTreeMap<String, Option<Arc<[u8]>>> {
        &self.fixture
    }
    pub fn expected(&self, path: &str) -> Option<&[u8]> {
        self.expected.get(path).map(AsRef::as_ref)
    }
    pub fn stdin(&self) -> &[u8] {
        &self.stdin
    }
}
impl RunPlan {
    pub(crate) fn inventory(&self) -> &Inventory {
        &self.inventory
    }
    pub fn target_file(&self) -> &File {
        &self._target_lock
    }
    #[cfg(target_os = "linux")]
    pub fn verify_target(&self, deadline: std::time::Instant) -> Result<(), String> {
        use std::{
            io::{Seek, SeekFrom},
            os::unix::fs::MetadataExt,
        };
        crate::schema::no_reparse(&self.executable).map_err(|_| "input_changed")?;
        let current = fs::metadata(&self.executable).map_err(|_| "input_changed")?;
        let held = self._target_lock.metadata().map_err(|e| e.to_string())?;
        if current.dev() != held.dev() || current.ino() != held.ino() {
            return Err("input_changed".into());
        }
        let mut file = self._target_lock.try_clone().map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            if std::time::Instant::now() >= deadline {
                return Err("run_deadline".into());
            }
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        if format!("{:x}", hash.finalize()) != self.target_hash {
            return Err("input_changed".into());
        }
        Ok(())
    }
    pub fn suite(&self) -> &Suite {
        &self.suite
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub fn cases(&self) -> &[CaseInputs] {
        &self.cases
    }
    pub fn suite_hash(&self) -> &str {
        &self.suite_hash
    }
    pub fn target_hash(&self) -> &str {
        &self.target_hash
    }
}
pub fn prepare(file: &Path, selected: Option<&str>) -> Result<RunPlan, String> {
    prepare_target(file, selected, None)
}
pub fn prepare_for_program(
    file: &Path,
    selected: Option<&str>,
    program: &Path,
) -> Result<RunPlan, String> {
    let program = if program.is_absolute() {
        program.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(program)
    };
    prepare_target(file, selected, Some(&program))
}
/// Both targets and the complete suite are validated before either launch.
/// Candidate cases share the baseline's immutable bytes and captured environment.
pub fn prepare_comparison(
    file: &Path,
    selected: Option<&str>,
    baseline: &Path,
    candidate: &Path,
) -> Result<(RunPlan, RunPlan), String> {
    let absolute = |path: &Path| -> Result<PathBuf, String> {
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path))
        }
    };
    let baseline = prepare_target(file, selected, Some(&absolute(baseline)?))?;
    let mut candidate = prepare_target(file, selected, Some(&absolute(candidate)?))?;
    if baseline.suite_hash != candidate.suite_hash {
        return Err("input_changed".into());
    }
    candidate.cases = baseline.cases.clone();
    Ok((baseline, candidate))
}
fn prepare_target(
    file: &Path,
    selected: Option<&str>,
    program: Option<&Path>,
) -> Result<RunPlan, String> {
    let suite = crate::schema::validate_with_program(file, selected, program)?;
    let suite_file = fs::canonicalize(file).map_err(|e| e.to_string())?;
    let base = suite_file.parent().ok_or("Suite has no parent")?;
    let executable = source(base, &suite.program, false)?;
    let inventory = validate_inputs(base, &suite)?;
    let mut hash = Sha256::new();
    // Typed canonical serialization makes comments/whitespace immaterial.
    if program.is_some() {
        let mut identity = serde_json::to_value(&suite).map_err(|e| e.to_string())?;
        // Comparison identity deliberately excludes executable location; each
        // actual binary is independently locked and SHA256-identified below.
        identity["program"] = "__comparison_target__".into();
        hash.update(serde_json::to_vec(&identity).map_err(|e| e.to_string())?);
    } else {
        hash.update(serde_json::to_vec(&suite).map_err(|e| e.to_string())?);
    }
    for (path, digest) in &inventory.files {
        let name = path_text(path.strip_prefix(base).unwrap_or(path))?;
        hash.update(b"file");
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update(digest.as_bytes());
    }
    for path in &inventory.directories {
        let name = path_text(path.strip_prefix(base).unwrap_or(path))?;
        hash.update(b"directory");
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
    }
    let mut snapshots = BTreeMap::<String, Arc<[u8]>>::new();
    let mut total = 0;
    let mut capture = |path: &Path| -> Result<Arc<[u8]>, String> {
        let key = file_identity(path)?;
        if let Some(bytes) = snapshots.get(&key) {
            if inventory
                .files
                .get(path)
                .is_none_or(|digest| digest != &format!("{:x}", Sha256::digest(bytes)))
            {
                return Err("input_changed".into());
            }
            return Ok(bytes.clone());
        }
        let bytes: Arc<[u8]> = read_bounded(path, INPUT_BUDGET - total)?.into();
        if inventory
            .files
            .get(path)
            .is_none_or(|digest| digest != &format!("{:x}", Sha256::digest(&bytes)))
        {
            return Err("input_changed".into());
        }
        total += bytes.len() as u64;
        snapshots.insert(key, bytes.clone());
        Ok(bytes)
    };
    let mut cases = Vec::new();
    for (index, case) in suite.cases.iter().enumerate() {
        let environment = capture_environment(case)?;
        if selected.is_some_and(|id| !crate::workflow::selected(case, id)) {
            continue;
        }
        let mut fixture = BTreeMap::new();
        if let Some(root) = &case.fixture_dir {
            let root = source(base, root, true)?;
            let entries = tree(&root)?;
            let actual: BTreeSet<_> = entries.iter().map(|e| e.source.clone()).collect();
            let approved: BTreeSet<_> = inventory
                .files
                .keys()
                .chain(inventory.directories.iter())
                .filter(|p| p.starts_with(&root) && *p != &root)
                .cloned()
                .collect();
            if actual != approved {
                return Err("input_changed".into());
            }
            for entry in entries {
                let bytes = if entry.directory {
                    None
                } else {
                    Some(capture(&entry.source)?)
                };
                fixture.insert(entry.relative, bytes);
            }
        }
        let mut expected = BTreeMap::new();
        for (reference, json) in referenced(case) {
            let bytes = capture(&source(base, reference, false)?)?;
            if json {
                crate::strict_json::parse(&bytes).map_err(|e| e.to_string())?;
            }
            expected.insert(reference.into(), bytes);
        }
        let stdin = if let Some(path) = &case.stdin_file {
            expected.get(path).ok_or("Missing stdin snapshot")?.clone()
        } else {
            Arc::from(case.stdin_text.as_deref().unwrap_or("").as_bytes())
        };
        if stdin.len() > 1024 * 1024 {
            return Err("Stdin exceeds limit".into());
        }
        cases.push(CaseInputs {
            index,
            fixture,
            expected,
            stdin,
            environment,
        });
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut target = options.open(&executable).map_err(|e| e.to_string())?;
    let mut target_hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = target.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        target_hash.update(&buffer[..count]);
    }
    Ok(RunPlan {
        suite,
        executable,
        cases,
        suite_hash: format!("{:x}", hash.finalize()),
        target_hash: format!("{:x}", target_hash.finalize()),
        _target_lock: target,
        inventory,
    })
}
fn capture_environment(case: &crate::schema::Case) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    #[cfg(windows)]
    for name in ["SystemRoot", "WINDIR", "COMSPEC"] {
        if let Ok(value) = std::env::var(name) {
            env.insert(name.to_string(), value);
        }
    }
    env.insert("PATH".to_string(), String::new());
    for name in &case.inherit_env {
        let value = std::env::var(name).map_err(|_| {
            format!("Inherited environment variable unavailable or not Unicode: {name}")
        })?;
        env.retain(|key, _| !ordinal_cmp(key, name).is_eq());
        env.insert(name.clone(), value);
    }
    for (name, value) in &case.env {
        env.retain(|key, _| !ordinal_cmp(key, name).is_eq());
        env.insert(name.clone(), value.clone());
    }
    Ok(env)
}
