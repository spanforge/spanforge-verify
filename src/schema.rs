use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

const MIB: u64 = 1024 * 1024;
fn timeout() -> u64 {
    5000
}
fn output() -> u64 {
    MIB
}
fn run_timeout() -> u64 {
    300_000
}
fn max_cases() -> usize {
    100
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub schema_version: u32,
    pub suite_id: String,
    pub program: String,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub redact_values_env: Vec<String>,
    pub cases: Vec<Case>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<Contract>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub cases: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    #[serde(default = "output")]
    pub max_output_bytes: u64,
}
impl Default for Defaults {
    fn default() -> Self {
        Self {
            timeout_ms: timeout(),
            max_output_bytes: output(),
        }
    }
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "run_timeout")]
    pub run_timeout_ms: u64,
    #[serde(default = "max_cases")]
    pub max_cases: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            run_timeout_ms: run_timeout(),
            max_cases: max_cases(),
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub args: Vec<String>,
    pub expect: Expect,
    pub fixture_dir: Option<String>,
    pub cwd: Option<String>,
    pub stdin_text: Option<String>,
    pub stdin_file: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub inherit_env: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub stdout: Option<StreamRule>,
    pub stderr: Option<StreamRule>,
    #[serde(default)]
    pub files: Vec<FileRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<crate::http_fixture::Fixture>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matrix: Vec<crate::workflow::Variant>,
    #[serde(
        default = "crate::workflow::one",
        skip_serializing_if = "crate::workflow::is_one"
    )]
    pub repeat: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<crate::workflow::Attempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Case>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extract: BTreeMap<String, crate::workflow::Extraction>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continue_on_failure: bool,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub exit_code: u32,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum StreamRule {
    ExactFile {
        expected_file: String,
    },
    TextEquals {
        text: String,
        #[serde(default)]
        normalize: Vec<String>,
    },
    Contains {
        text: String,
        #[serde(default)]
        normalize: Vec<String>,
    },
    NotContains {
        text: String,
        #[serde(default)]
        normalize: Vec<String>,
    },
    Regex {
        pattern: String,
        #[serde(default)]
        normalize: Vec<String>,
    },
    JsonEqualsFile {
        expected_file: String,
    },
    JsonPointers {
        values: BTreeMap<String, String>,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "RawFileRule")]
pub struct FileRule {
    pub path: String,
    pub rule: FileKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFileRule {
    path: String,
    kind: String,
    mode: Option<String>,
    expected_file: Option<String>,
}
impl TryFrom<RawFileRule> for FileRule {
    type Error = String;
    fn try_from(raw: RawFileRule) -> Result<Self, String> {
        let rule = match raw.kind.as_str() {
            "absent" | "directory" => {
                require(
                    raw.mode.is_none() && raw.expected_file.is_none(),
                    "Unexpected file payload",
                )?;
                if raw.kind == "absent" {
                    FileKind::Absent
                } else {
                    FileKind::Directory
                }
            }
            "file" => FileKind::File {
                mode: raw.mode.ok_or("Missing file mode")?,
                expected_file: raw.expected_file.ok_or("Missing expected file")?,
            },
            _ => return Err("Unknown file kind".into()),
        };
        Ok(Self {
            path: raw.path,
            rule,
        })
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FileKind {
    Absent,
    Directory,
    File { mode: String, expected_file: String },
}

fn require(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}
pub fn workspace_path(value: &str) -> Result<(), String> {
    #[cfg(not(windows))]
    require(
        !value.contains('\\'),
        "Use forward slashes in portable workspace paths",
    )?;
    require(
        !value.is_empty() && !value.contains(['\0', ':']) && !value.starts_with(['/', '\\']),
        "Invalid workspace path",
    )?;
    for part in value.split(['/', '\\']) {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|s| {
                matches!(
                    s,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        require(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !device
                && !part.chars().any(|c| c < ' ' || "<>\"|?*".contains(c)),
            "Unsafe workspace path component",
        )?;
    }
    Ok(())
}
pub(crate) fn no_reparse(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                Win32::{Foundation::CloseHandle, Storage::FileSystem::*},
                core::PCWSTR,
            };
            let name: Vec<u16> = ancestor.as_os_str().encode_wide().chain(Some(0)).collect();
            // Open the entry itself, not its link target, including directory entries.
            let handle = unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                    None,
                )
            }
            .map_err(|e| format!("Input handle access failed for {}: {e}", ancestor.display()))?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            let query = unsafe { GetFileInformationByHandle(handle, &mut info) };
            let close = unsafe { CloseHandle(handle) };
            query.map_err(|e| e.to_string())?;
            close.map_err(|e| e.to_string())?;
            require(
                info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
                "Reparse points are unsupported",
            )?;
        }
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|e| format!("Input access failed: {e}"))?;
        require(!metadata.file_type().is_symlink(), "Links are unsupported")?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            require(
                metadata.file_attributes() & 0x400 == 0,
                "Reparse points are unsupported",
            )?;
        }
    }
    Ok(())
}
pub(crate) fn source(base: &Path, value: &str, directory: bool) -> Result<PathBuf, String> {
    require(
        !value.is_empty() && !value.contains('\0'),
        "Invalid input path",
    )?;
    require(
        !value.starts_with("\\\\") && !value.starts_with("//"),
        "Network and device paths are unsupported",
    )?;
    for (i, part) in value.split(['/', '\\']).enumerate() {
        require(
            !part.contains(':')
                || (i == 0 && part.len() == 2 && part.as_bytes()[0].is_ascii_alphabetic()),
            "Alternate streams are unsupported",
        )?;
    }
    #[cfg(windows)]
    require(
        !value.contains(':') || Path::new(value).is_absolute(),
        "Drive relative input path",
    )?;
    let path = base.join(value);
    no_reparse(&path)?;
    let metadata = fs::metadata(&path).map_err(|e| format!("Input access failed: {e}"))?;
    require(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "Wrong input entry type",
    )?;
    if !directory {
        fs::File::open(&path).map_err(|e| format!("Unreadable input: {e}"))?;
    }
    Ok(path)
}
fn names<'a>(values: impl Iterator<Item = &'a str>, reserved: bool) -> Result<(), String> {
    let values: Vec<_> = values.collect();
    require(
        crate::inputs::unique_names(values.iter().copied()),
        "Duplicate environment name",
    )?;
    for name in values {
        require(
            !name.is_empty() && !name.contains(['\0', '=']),
            "Invalid environment name",
        )?;
        let key = if cfg!(windows) {
            name.to_uppercase()
        } else {
            name.to_owned()
        };
        #[cfg(windows)]
        require(
            !reserved
                || !matches!(
                    key.as_str(),
                    "TEMP" | "TMP" | "USERPROFILE" | "HOME" | "APPDATA" | "LOCALAPPDATA"
                ),
            "Reserved environment name",
        )?;
        #[cfg(not(windows))]
        require(
            !reserved
                || !matches!(
                    key.as_str(),
                    "HOME"
                        | "TMPDIR"
                        | "XDG_CONFIG_HOME"
                        | "XDG_DATA_HOME"
                        | "XDG_CACHE_HOME"
                        | "XDG_STATE_HOME"
                ),
            "Reserved environment name",
        )?;
    }
    Ok(())
}
fn stream(base: &Path, rule: &StreamRule) -> Result<(), String> {
    let normalize = match rule {
        StreamRule::ExactFile { expected_file } | StreamRule::JsonEqualsFile { expected_file } => {
            source(base, expected_file, false)?;
            return Ok(());
        }
        StreamRule::TextEquals { normalize, .. } | StreamRule::Contains { normalize, .. } => {
            normalize
        }
        StreamRule::NotContains { text, normalize } => {
            require(!text.is_empty(), "not_contains text must be nonempty")?;
            normalize
        }
        StreamRule::Regex { pattern, normalize } => {
            require(pattern.len() <= 65536, "Regex pattern exceeds limit")?;
            regex::Regex::new(pattern).map_err(|_| "Invalid regex".to_string())?;
            normalize
        }
        StreamRule::JsonPointers { values } => {
            require(!values.is_empty(), "JSON pointers must be nonempty")?;
            for (pointer, literal) in values {
                crate::strict_json::parse(literal.as_bytes()).map_err(|e| e.to_string())?;
                require(
                    pointer.is_empty() || pointer.starts_with('/'),
                    "Invalid JSON pointer",
                )?;
                let mut chars = pointer.chars();
                while let Some(c) = chars.next() {
                    if c == '~' {
                        require(
                            matches!(chars.next(), Some('0' | '1')),
                            "Invalid JSON pointer escape",
                        )?;
                    }
                }
            }
            return Ok(());
        }
    };
    require(
        normalize.is_empty() || normalize == &["crlf_to_lf"],
        "Unsupported or duplicate normalization",
    )
}
pub fn validate(file: &Path, selected: Option<&str>) -> Result<Suite, String> {
    validate_with_program(file, selected, None)
}
pub(crate) fn validate_with_program(
    file: &Path,
    selected: Option<&str>,
    program: Option<&Path>,
) -> Result<Suite, String> {
    let original = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(file)
    };
    no_reparse(&original)?;
    let file = fs::canonicalize(file).map_err(|e| format!("Cannot resolve suite: {e}"))?;
    no_reparse(&file)?;
    require(
        fs::metadata(&file).map_err(|e| e.to_string())?.len() <= MIB,
        "Suite exceeds 1 MiB",
    )?;
    let bytes = crate::inputs::read_bounded(&file, MIB)?;
    let text = String::from_utf8(bytes).map_err(|_| "Suite must be UTF-8")?;
    require(text.len() as u64 <= MIB, "Suite exceeds 1 MiB")?;
    let mut suite: Suite =
        toml::from_str(&text).map_err(|e| format!("Invalid suite: {}", e.message()))?;
    if let Some(program) = program {
        suite.program = crate::inputs::path_text(program)?;
    }
    require(suite.schema_version == 1, "Unsupported schema version")?;
    require(id(&suite.suite_id), "Invalid suite ID")?;
    require(
        (1..=60_000).contains(&suite.defaults.timeout_ms)
            && (1..=16 * MIB).contains(&suite.defaults.max_output_bytes),
        "Invalid default limits",
    )?;
    require(
        (1..=1_800_000).contains(&suite.limits.run_timeout_ms)
            && (1..=1000).contains(&suite.limits.max_cases),
        "Invalid run limits",
    )?;
    crate::workflow::expand(&mut suite)?;
    require(
        !suite.cases.is_empty() && suite.cases.len() <= suite.limits.max_cases,
        "Invalid case count",
    )?;
    let base = file.parent().ok_or("Suite has no parent")?;
    let executable = source(base, &suite.program, false)?;
    #[cfg(windows)]
    require(
        executable
            .extension()
            .is_some_and(|v| v.eq_ignore_ascii_case("exe")),
        "V1 requires a native .exe target",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        require(
            fs::metadata(&executable)
                .map_err(|e| e.to_string())?
                .permissions()
                .mode()
                & 0o111
                != 0,
            "Target must have native executable permission",
        )?;
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        let mut magic = [0u8; 4];
        fs::File::open(&executable)
            .and_then(|mut file| file.read_exact(&mut magic))
            .map_err(|_| "Cannot read native ELF target")?;
        require(
            magic == *b"\x7fELF",
            "Linux requires a native ELF target; scripts are unsupported",
        )?;
    }
    names(suite.redact_values_env.iter().map(String::as_str), false)?;
    for name in &suite.redact_values_env {
        require(
            std::env::var(name).is_ok_and(|v| !v.is_empty()),
            "Missing or empty masking environment value",
        )?;
    }
    require(suite.contracts.len() <= 128, "Too many declared contracts")?;
    let mut contract_ids = HashSet::new();
    for contract in &suite.contracts {
        require(
            id(&contract.id) && contract_ids.insert(&contract.id),
            "Invalid or duplicate contract ID",
        )?;
        require(
            !contract.description.is_empty() && contract.description.len() <= 1024,
            "Invalid contract description",
        )?;
        require(
            crate::inputs::unique_names(contract.cases.iter().map(String::as_str))
                && contract
                    .cases
                    .iter()
                    .all(|id| suite.cases.iter().any(|c| &c.id == id)),
            "Invalid contract case references",
        )?;
    }
    let mut ids = HashSet::new();
    let mut validation_cases = Vec::new();
    for case in &suite.cases {
        require(
            id(&case.id) && case.id != "__run__" && ids.insert(&case.id),
            "Invalid, reserved or duplicate case ID",
        )?;
        crate::workflow::validate_scenario(case)?;
        validation_cases.push(case.clone());
        for step in &case.steps {
            let mut step = step.clone();
            step.fixture_dir = case.fixture_dir.clone();
            if step.cwd.is_none() {
                step.cwd = case.cwd.clone();
            }
            for (key, value) in &case.env {
                if !step
                    .env
                    .keys()
                    .any(|k| crate::inputs::ordinal_cmp(k, key).is_eq())
                {
                    step.env.insert(key.clone(), value.clone());
                }
            }
            validation_cases.push(step);
        }
    }
    require(
        validation_cases.len() <= 1000,
        "Expanded workflow exceeds 1000 commands",
    )?;
    for case in &validation_cases {
        require(
            case.args.iter().all(|v| !v.contains('\0')),
            "NUL in arguments",
        )?;
        require(
            case.http.is_some() || !case.args.iter().any(|a| a.contains("{{http.url}}")),
            "HTTP URL argument requires a case HTTP fixture",
        )?;
        #[cfg(windows)]
        {
            let executable = executable.to_str().ok_or("Non-Unicode target path")?;
            let units = crate::process_windows::quote(executable)
                .encode_utf16()
                .count()
                + case
                    .args
                    .iter()
                    .map(|a| {
                        let expanded = a.replace("{{http.url}}", "http://127.0.0.1:65535");
                        1 + crate::process_windows::quote(&expanded)
                            .encode_utf16()
                            .count()
                    })
                    .sum::<usize>()
                + 1;
            require(units <= 32767, "Windows command line exceeds limit")?;
        }
        require(
            (1..=60_000).contains(&case.timeout_ms.unwrap_or(suite.defaults.timeout_ms))
                && (1..=16 * MIB).contains(
                    &case
                        .max_output_bytes
                        .unwrap_or(suite.defaults.max_output_bytes),
                ),
            "Invalid case limits",
        )?;
        require(
            case.stdin_text.is_none() || case.stdin_file.is_none(),
            "Conflicting stdin sources",
        )?;
        if let Some(text) = &case.stdin_text {
            require(text.len() as u64 <= MIB, "Stdin exceeds limit")?;
        }
        if let Some(input) = &case.stdin_file {
            let path = source(base, input, false)?;
            require(
                fs::metadata(path).map_err(|e| e.to_string())?.len() <= MIB,
                "Stdin exceeds limit",
            )?;
        }
        let fixture = case
            .fixture_dir
            .as_ref()
            .map(|v| source(base, v, true))
            .transpose()?;
        if let Some(cwd) = &case.cwd {
            workspace_path(cwd)?;
            let root = fixture.as_ref().ok_or("cwd requires fixture_dir")?;
            source(root, cwd, true)?;
        }
        names(case.env.keys().map(String::as_str), true)?;
        if let Some(http) = &case.http {
            http.validate()?;
            require(
                !case
                    .env
                    .keys()
                    .chain(&case.inherit_env)
                    .any(|n| crate::inputs::ordinal_cmp(n, &http.url_env).is_eq()),
                "HTTP URL environment name conflicts with case environment",
            )?;
        }
        names(case.inherit_env.iter().map(String::as_str), true)?;
        require(
            case.env.values().all(|v| !v.contains('\0')),
            "NUL in environment value",
        )?;
        for name in &case.inherit_env {
            require(
                std::env::var(name).is_ok(),
                "Missing inherited environment name",
            )?;
        }
        for rule in [&case.stdout, &case.stderr].into_iter().flatten() {
            stream(base, rule)?;
        }
        require(
            crate::inputs::unique_names(case.files.iter().map(|f| f.path.as_str())),
            "Colliding file paths",
        )?;
        let mut paths = Vec::new();
        for file in &case.files {
            workspace_path(&file.path)?;
            paths.push(file.path.replace('\\', "/"));
            if let FileKind::File {
                mode,
                expected_file,
            } = &file.rule
            {
                require(
                    matches!(mode.as_str(), "exact_file" | "json_equals_file"),
                    "Invalid file mode",
                )?;
                source(base, expected_file, false)?;
            }
        }
        require(
            crate::inputs::unique_names(paths.iter().map(String::as_str)),
            "Colliding file paths",
        )?;
        for parent in &case.files {
            if matches!(parent.rule, FileKind::Absent | FileKind::File { .. }) {
                require(
                    !case.files.iter().any(|child| {
                        crate::inputs::path_descendant(&child.path, &parent.path)
                            && (!matches!(parent.rule, FileKind::Absent)
                                || !matches!(child.rule, FileKind::Absent))
                    }),
                    "Contradictory file declarations",
                )?;
            }
        }
    }
    crate::inputs::validate_inputs(base, &suite)?;
    if let Some(selected) = selected {
        require(
            suite
                .cases
                .iter()
                .any(|c| crate::workflow::selected(c, selected)),
            "Selected case does not exist",
        )?;
    }
    Ok(suite)
}
