//! Preflight and bounded, same-directory atomic report publication.
use crate::{
    inputs,
    model::{RunResult, Status},
    privacy,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub fn run_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn utc_now() -> String {
    chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn os_build() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Version {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            description: [u16; 128],
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn RtlGetVersion(version: *mut Version) -> i32;
        }
        let mut version = Version {
            size: std::mem::size_of::<Version>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            description: [0; 128],
        };
        // SAFETY: the buffer matches OSVERSIONINFOW and its size is initialized.
        if unsafe { RtlGetVersion(&mut version) } == 0 {
            return format!(
                "windows {}.{}.{}",
                version.major, version.minor, version.build
            );
        }
        "windows (build unavailable)".into()
    }
    #[cfg(not(windows))]
    {
        format!(
            "{} {}",
            std::env::consts::OS,
            fs::read_to_string("/proc/sys/kernel/osrelease")
                .unwrap_or_else(|_| "build unavailable".into())
                .trim()
        )
    }
}
const LIMIT: usize = 16 * 1024 * 1024;
pub struct Destinations {
    json: Option<PathBuf>,
    junit: Option<PathBuf>,
    overwrite: bool,
}
impl Destinations {
    pub fn prepare(
        suite: &Path,
        json: Option<&Path>,
        junit: Option<&Path>,
        mut overwrite: bool,
    ) -> Result<Self, String> {
        let suite = if suite.is_absolute() {
            suite.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(suite)
        };
        let base = suite.parent().ok_or("Missing suite parent")?;
        let mut protected = vec![suite.clone()];
        if let Ok(parsed) = crate::schema::validate(&suite, None) {
            let inventory = inputs::validate_inputs(base, &parsed)?;
            protected.extend(
                inventory
                    .files
                    .keys()
                    .chain(inventory.directories.iter())
                    .cloned(),
            );
            protected.push(crate::schema::source(base, &parsed.program, false)?);
        } else if let Ok(text) = inputs::read_bounded(&suite, 1024 * 1024)
            .and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()))
        {
            // Invalid configurations still get reports. Protect every recognizable
            // input reference, including references inside invalid cases.
            if let Ok(value) = toml::from_str::<toml::Value>(&text) {
                collect_references(&value, base, &mut protected);
            } else {
                // Unparseable input references cannot be inventoried. New reports
                // are safe; never replace any existing file in this situation.
                overwrite = false;
            }
        } else {
            overwrite = false;
        }
        let resolve = |path: &Path| -> Result<PathBuf, String> {
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|e| e.to_string())?
                    .join(path)
            };
            let parent = absolute
                .parent()
                .ok_or("Report has no parent")?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            crate::schema::no_reparse(&parent)?;
            let name = absolute
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Invalid report filename")?;
            crate::schema::workspace_path(name)?;
            let path = parent.join(name);
            match fs::symlink_metadata(&path) {
                Ok(meta) => {
                    crate::schema::no_reparse(&path)?;
                    if !meta.is_file() {
                        return Err("Report destination must be an ordinary file".into());
                    }
                    if !overwrite {
                        return Err("Report already exists; use --overwrite-reports".into());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            for input in &protected {
                if inputs::ordinal_cmp(&path.to_string_lossy(), &input.to_string_lossy()).is_eq()
                    || input.is_dir() && path.starts_with(input)
                    || path.exists()
                        && input.is_file()
                        && inputs::file_identity(&path)? == inputs::file_identity(input)?
                {
                    return Err("Report destination aliases a suite input".into());
                }
            }
            // Prove a same-directory temporary report can be created before launch.
            tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
            Ok(path)
        };
        let json = json.map(resolve).transpose()?;
        let junit = junit.map(resolve).transpose()?;
        if let (Some(a), Some(b)) = (&json, &junit)
            && (inputs::ordinal_cmp(&a.to_string_lossy(), &b.to_string_lossy()).is_eq()
                || a.exists()
                    && b.exists()
                    && inputs::file_identity(a)? == inputs::file_identity(b)?)
        {
            return Err("Report destinations alias each other".into());
        }
        Ok(Self {
            json,
            junit,
            overwrite,
        })
    }
    pub fn publish(&self, result: &mut RunResult) -> Result<(), String> {
        let deadline = result
            .publication_deadline
            .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(5));
        let json = bounded_json(result)?;
        let xml = junit(result);
        for (path, bytes) in [(&self.json, json.as_slice()), (&self.junit, xml.as_bytes())] {
            if let Some(path) = path {
                self.write(path, bytes, deadline)?;
            }
        }
        Ok(())
    }
    fn write(&self, path: &Path, bytes: &[u8], deadline: std::time::Instant) -> Result<(), String> {
        if bytes.len() > LIMIT {
            return Err("Report exceeds 16 MiB".into());
        }
        let parent = path.parent().ok_or("Report has no parent")?;
        crate::schema::no_reparse(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        for chunk in bytes.chunks(65536) {
            if std::time::Instant::now() >= deadline {
                return Err("Report publication budget exceeded".into());
            }
            temporary.write_all(chunk).map_err(|e| e.to_string())?;
        }
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        if std::time::Instant::now() >= deadline {
            return Err("Report publication budget exceeded".into());
        }
        if self.overwrite {
            if path.exists() {
                crate::schema::no_reparse(path)?;
                if !fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
                    return Err("Report destination changed type".into());
                }
            }
            temporary.persist(path).map_err(|e| e.error.to_string())?;
        } else {
            temporary
                .persist_noclobber(path)
                .map_err(|e| e.error.to_string())?;
        }
        Ok(())
    }
}
fn bounded_json(result: &mut RunResult) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?;
    if bytes.len() <= LIMIT {
        return Ok(bytes);
    }
    for case in &mut result.cases {
        for assertion in &mut case.assertions {
            result.details_omitted += u64::from(assertion.expected_summary.take().is_some())
                + u64::from(assertion.observed_summary.take().is_some());
        }
        result.details_omitted +=
            (case.workspace_deltas.len() + case.checked.len() + case.unchecked.len()) as u64;
        case.workspace_deltas.clear();
        case.checked.clear();
        case.unchecked.clear();
    }
    bytes = serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("Report outcomes exceed 16 MiB".into());
    }
    Ok(bytes)
}
fn escape(s: &str) -> String {
    privacy::xml_sanitize(s)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn collect_references(value: &toml::Value, base: &Path, protected: &mut Vec<PathBuf>) {
    match value {
        toml::Value::Table(table) => {
            for (key, value) in table {
                if matches!(
                    key.as_str(),
                    "program" | "fixture_dir" | "stdin_file" | "expected_file"
                ) && let Some(path) = value.as_str()
                {
                    let path = base.join(path);
                    protected.push(path.canonicalize().unwrap_or(path));
                }
                collect_references(value, base, protected);
            }
        }
        toml::Value::Array(values) => {
            for value in values {
                collect_references(value, base, protected);
            }
        }
        _ => {}
    }
}
pub fn error_result(code: u8, message: String) -> RunResult {
    let status = if code == 2 {
        Status::ConfigError
    } else {
        Status::InfraError
    };
    RunResult {
        publication_deadline: None,
        schema_version: 1,
        run_id: run_id(),
        suite_id: None,
        suite_hash: None,
        target_hash: None,
        runner_version: env!("CARGO_PKG_VERSION").into(),
        os: os_build(),
        arch: std::env::consts::ARCH.into(),
        started_at: utc_now(),
        duration_ms: 0,
        status,
        exit_code: status.exit_code(),
        limits: crate::model::RunLimits {
            run_timeout_ms: 300000,
            max_cases: 100,
        },
        errors: vec![crate::model::RunError {
            reason_code: if code == 2 {
                "config_invalid"
            } else {
                "infrastructure"
            }
            .into(),
            message,
        }],
        cases: vec![],
        details_omitted: 0,
    }
}
pub fn junit(result: &RunResult) -> String {
    let failures = result
        .cases
        .iter()
        .filter(|c| c.status == Status::Fail)
        .count();
    let errors = result
        .cases
        .iter()
        .filter(|c| matches!(c.status, Status::InfraError | Status::ConfigError))
        .count()
        + usize::from(result.cases.is_empty() && !result.errors.is_empty());
    let skipped = result
        .cases
        .iter()
        .filter(|c| c.status == Status::Inconclusive)
        .count();
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites><testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" errors=\"{errors}\" skipped=\"{skipped}\" time=\"{:.3}\">",
        escape(result.suite_id.as_deref().unwrap_or("__run__")),
        result.cases.len() + usize::from(result.cases.is_empty() && !result.errors.is_empty()),
        result.duration_ms as f64 / 1000.0
    );
    xml.push_str(&format!("<properties><property name=\"run_id\" value=\"{}\"/><property name=\"runner_exit_code\" value=\"{}\"/></properties>",escape(&result.run_id),result.exit_code));
    if result.cases.is_empty() && !result.errors.is_empty() {
        xml.push_str("<testcase classname=\"__run__\" name=\"__run__\" time=\"0.000\">");
        if let Some(error) = result.errors.first() {
            xml.push_str(&format!(
                "<error type=\"{}\" message=\"{}\"/>",
                escape(&error.reason_code),
                escape(&error.message)
            ));
        }
        xml.push_str("</testcase>");
    }
    for case in &result.cases {
        xml.push_str(&format!(
            "\n<testcase classname=\"{}\" name=\"{}\" time=\"{:.3}\">",
            escape(result.suite_id.as_deref().unwrap_or("__run__")),
            escape(&case.case_id),
            case.duration_ms as f64 / 1000.0
        ));
        let tag = match case.status {
            Status::Pass => None,
            Status::Fail => Some("failure"),
            Status::Inconclusive => Some("skipped"),
            _ => Some("error"),
        };
        if let Some(tag) = tag {
            let reason = case.reason_code.as_deref().unwrap_or("assertion_failed");
            xml.push_str(&format!(
                "<{tag} type=\"{}\" message=\"{}\">",
                escape(reason),
                escape(reason)
            ));
            for a in &case.assertions {
                if a.status != crate::model::AssertionStatus::Pass {
                    xml.push_str(&escape(&format!(
                        "{}: {}\n",
                        a.check_id,
                        a.reason_code.as_deref().unwrap_or("not_evaluated")
                    )));
                }
            }
            xml.push_str(&format!("</{tag}>"));
        }
        xml.push_str("</testcase>");
    }
    xml.push_str("\n</testsuite></testsuites>\n");
    xml
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_diagnostics_are_omitted_without_changing_outcomes() {
        let mut result: RunResult =
            serde_json::from_str(include_str!("../tests/fixtures/reports/pass.json")).unwrap();
        let mut case = result.cases[0].clone();
        case.assertions[0].expected_summary = Some("x".repeat(20000));
        result.cases = vec![case; 1000];
        let bytes = bounded_json(&mut result).unwrap();
        assert!(bytes.len() <= LIMIT);
        assert!(result.details_omitted >= 1000);
        assert_eq!(result.status, Status::Pass);
        assert_eq!(result.cases.len(), 1000);
        assert!(result.cases.iter().all(|c| c.status == Status::Pass));
    }
    #[test]
    fn synthetic_error_uses_one_terminal_element_even_with_multiple_diagnostics() {
        let mut result = error_result(3, "first".into());
        result.errors.push(crate::model::RunError {
            reason_code: "second".into(),
            message: "second".into(),
        });
        let xml = junit(&result);
        assert_eq!(xml.matches("<error ").count(), 1);
        assert!(xml.contains("tests=\"1\""));
    }
}
