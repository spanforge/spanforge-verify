//! Fixture materialization, persistent state checks and bounded safe cleanup.
use crate::{
    inputs::{CaseInputs, ENTRY_LIMIT, path_text},
    model::{Change, WorkspaceDelta},
    schema::{FileKind, FileRule, workspace_path},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Instant,
};
const BYTE_LIMIT: u64 = 1024 * 1024 * 1024;
#[derive(Debug)]
pub struct WorkspaceError {
    pub reason_code: &'static str,
    pub message: String,
}
fn error(code: &'static str, message: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError {
        reason_code: code,
        message: message.to_string(),
    }
}
impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason_code, self.message)
    }
}
impl std::error::Error for WorkspaceError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Directory,
    File { sha256: String, bytes: u64 },
}
pub type Snapshot = BTreeMap<String, Entry>;
fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
pub fn snapshot(root: &Path, deadline: Instant) -> Result<Snapshot, WorkspaceError> {
    crate::schema::no_reparse(root).map_err(|e| error("workspace_failed", e))?;
    let mut snapshot = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0;
    while let Some(directory) = pending.pop() {
        if Instant::now() >= deadline {
            return Err(error("run_deadline", "Workspace scan deadline"));
        }
        for entry in fs::read_dir(directory).map_err(|e| error("workspace_failed", e))? {
            if Instant::now() >= deadline {
                return Err(error("run_deadline", "Workspace scan deadline"));
            }
            let path = entry.map_err(|e| error("workspace_failed", e))?.path();
            let relative = path_text(
                path.strip_prefix(root)
                    .map_err(|e| error("workspace_failed", e))?,
            )
            .map_err(|e| error("unsupported_entry", e))?;
            workspace_path(&relative).map_err(|e| error("unsupported_entry", e))?;
            let metadata = fs::symlink_metadata(&path).map_err(|e| error("workspace_failed", e))?;
            if reparse(&metadata) || (!metadata.is_dir() && !metadata.is_file()) {
                return Err(error(
                    "unsupported_entry",
                    format!("Unsupported workspace entry: {relative}"),
                ));
            }
            if metadata.is_dir() {
                pending.push(path);
                snapshot.insert(relative, Entry::Directory);
            } else {
                if metadata.len() > BYTE_LIMIT - total {
                    return Err(error("workspace_limit", "Workspace exceeds 1 GiB"));
                }
                crate::schema::no_reparse(&path).map_err(|e| error("workspace_failed", e))?;
                let mut file = fs::File::open(&path).map_err(|e| error("workspace_failed", e))?;
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 65536];
                let mut count = 0;
                loop {
                    if Instant::now() >= deadline {
                        return Err(error("run_deadline", "Workspace hash deadline"));
                    }
                    let bytes = file
                        .read(&mut buffer)
                        .map_err(|e| error("workspace_failed", e))?;
                    if bytes == 0 {
                        break;
                    }
                    count += bytes as u64;
                    if count > BYTE_LIMIT - total {
                        return Err(error("workspace_limit", "Workspace exceeds 1 GiB"));
                    }
                    hash.update(&buffer[..bytes]);
                }
                total += count;
                snapshot.insert(
                    relative,
                    Entry::File {
                        sha256: format!("{:x}", hash.finalize()),
                        bytes: count,
                    },
                );
            }
            if snapshot.len() > ENTRY_LIMIT {
                return Err(error("workspace_limit", "Workspace exceeds 10000 entries"));
            }
        }
    }
    if !crate::inputs::unique_names(snapshot.keys().map(String::as_str)) {
        return Err(error("unsupported_entry", "Workspace case collision"));
    }
    Ok(snapshot)
}
pub fn deltas(before: &Snapshot, after: &Snapshot) -> Vec<WorkspaceDelta> {
    let keys: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
    keys.into_iter()
        .filter_map(|path| {
            let change = match (before.get(path), after.get(path)) {
                (None, Some(_)) => Change::Created,
                (Some(_), None) => Change::Removed,
                (Some(a), Some(b)) if std::mem::discriminant(a) != std::mem::discriminant(b) => {
                    Change::TypeChanged
                }
                (Some(a), Some(b)) if a != b => Change::Modified,
                _ => return None,
            };
            Some(WorkspaceDelta {
                path: path.clone(),
                change,
            })
        })
        .collect()
}
fn equal(a: &str, b: &str) -> bool {
    crate::inputs::ordinal_cmp(&a.replace('\\', "/"), &b.replace('\\', "/")).is_eq()
}
fn descendant(child: &str, parent: &str) -> bool {
    crate::inputs::path_descendant(child, parent)
}
pub fn undeclared_changes(
    before: &Snapshot,
    changes: &[WorkspaceDelta],
    declarations: &[FileRule],
) -> Vec<String> {
    changes
        .iter()
        .filter(|delta| {
            !declarations.iter().any(|rule| {
                equal(&delta.path, &rule.path)
                    || (matches!(rule.rule, FileKind::Absent)
                        && matches!(delta.change, Change::Removed)
                        && descendant(&delta.path, &rule.path))
                    || (matches!(delta.change, Change::Created)
                        && !before.keys().any(|p| equal(p, &delta.path))
                        && descendant(&rule.path, &delta.path)
                        && !matches!(rule.rule, FileKind::Absent))
            })
        })
        .map(|delta| delta.path.clone())
        .collect()
}
pub fn undeclared_changes_with_types(
    before: &Snapshot,
    after: &Snapshot,
    changes: &[WorkspaceDelta],
    declarations: &[FileRule],
) -> Vec<String> {
    let mut unexpected = undeclared_changes(before, changes, declarations);
    for delta in changes {
        if matches!(delta.change, Change::Created)
            && after.iter().any(|(path, entry)| {
                equal(path, &delta.path) && matches!(entry, Entry::File { .. })
            })
            && !declarations
                .iter()
                .any(|rule| equal(&rule.path, &delta.path))
            && !unexpected.contains(&delta.path)
        {
            unexpected.push(delta.path.clone());
        }
    }
    unexpected
}
pub struct CaseWorkspace {
    root: tempfile::TempDir,
    workspace: PathBuf,
    baseline: Snapshot,
    profile: PathBuf,
}
impl CaseWorkspace {
    pub fn create(
        parent: &Path,
        inputs: &CaseInputs,
        deadline: Instant,
    ) -> Result<Self, WorkspaceError> {
        let root = tempfile::tempdir_in(parent).map_err(|e| error("workspace_failed", e))?;
        let workspace = root.path().join("workspace");
        let profile = root.path().join("profile");
        if profile.to_str().is_none() {
            return Err(error(
                "workspace_failed",
                "Private profile path must be Unicode",
            ));
        }
        fs::create_dir(&workspace).map_err(|e| error("workspace_failed", e))?;
        fs::create_dir(&profile).map_err(|e| error("workspace_failed", e))?;
        for (path, bytes) in inputs.fixture() {
            if Instant::now() >= deadline {
                return Err(error("run_deadline", "Fixture preparation deadline"));
            }
            let destination = workspace.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| error("workspace_failed", e))?;
            }
            if let Some(bytes) = bytes {
                fs::write(destination, bytes).map_err(|e| error("workspace_failed", e))?;
            } else {
                fs::create_dir_all(destination).map_err(|e| error("workspace_failed", e))?;
            }
        }
        for directory in ["temp", "roaming", "local", "cache", "state"] {
            fs::create_dir(profile.join(directory)).map_err(|e| error("workspace_failed", e))?;
        }
        let baseline = snapshot(&workspace, deadline)?;
        Ok(Self {
            root,
            workspace,
            baseline,
            profile,
        })
    }
    pub fn path(&self) -> &Path {
        &self.workspace
    }
    pub fn baseline(&self) -> &Snapshot {
        &self.baseline
    }
    pub fn private_environment(&self) -> BTreeMap<String, String> {
        #[cfg(windows)]
        let directories = [
            ("TEMP", self.profile.join("temp")),
            ("TMP", self.profile.join("temp")),
            ("USERPROFILE", self.profile.clone()),
            ("HOME", self.profile.clone()),
            ("APPDATA", self.profile.join("roaming")),
            ("LOCALAPPDATA", self.profile.join("local")),
        ];
        #[cfg(not(windows))]
        let directories = [
            ("HOME", self.profile.clone()),
            ("TMPDIR", self.profile.join("temp")),
            ("XDG_CONFIG_HOME", self.profile.join("roaming")),
            ("XDG_DATA_HOME", self.profile.join("local")),
            ("XDG_CACHE_HOME", self.profile.join("cache")),
            ("XDG_STATE_HOME", self.profile.join("state")),
        ];
        directories
            .into_iter()
            .map(|(name, path)| (name.into(), path.to_string_lossy().into()))
            .collect()
    }
    pub fn cleanup(self, deadline: Instant) -> Result<(), WorkspaceError> {
        let root = self.root.keep();
        remove_without_following(&root, deadline).map_err(|e| {
            error(
                "cleanup_failed",
                format!("Residual path {}: {e}", root.display()),
            )
        })
    }
}
fn remove_without_following(path: &Path, deadline: Instant) -> Result<(), WorkspaceError> {
    let mut pending = vec![(path.to_path_buf(), false)];
    while let Some((path, visited)) = pending.pop() {
        if Instant::now() >= deadline {
            return Err(error("cleanup_failed", "Cleanup budget exceeded"));
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| error("cleanup_failed", e))?;
        if reparse(&metadata) {
            // Windows RemoveDirectory removes the junction itself; no traversal.
            #[cfg(windows)]
            let directory = {
                use std::os::windows::fs::MetadataExt;
                metadata.file_attributes() & 0x10 != 0
            };
            #[cfg(not(windows))]
            let directory = metadata.is_dir();
            if directory {
                fs::remove_dir(&path)
            } else {
                fs::remove_file(&path)
            }
            .map_err(|e| error("cleanup_failed", e))?;
        } else if metadata.is_dir() {
            crate::schema::no_reparse(&path).map_err(|e| error("cleanup_failed", e))?;
            if visited {
                fs::remove_dir(&path).map_err(|e| error("cleanup_failed", e))?;
            } else {
                pending.push((path.clone(), true));
                for entry in fs::read_dir(&path).map_err(|e| error("cleanup_failed", e))? {
                    pending.push((entry.map_err(|e| error("cleanup_failed", e))?.path(), false));
                }
            }
        } else {
            fs::remove_file(&path).map_err(|e| error("cleanup_failed", e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(path: &str, kind: FileKind) -> FileRule {
        FileRule {
            path: path.into(),
            rule: kind,
        }
    }
    #[test]
    fn directory_declaration_does_not_allow_children() {
        let delta = vec![
            WorkspaceDelta {
                path: "out".into(),
                change: Change::Created,
            },
            WorkspaceDelta {
                path: "out/sibling".into(),
                change: Change::Created,
            },
        ];
        assert_eq!(
            undeclared_changes(
                &Snapshot::new(),
                &delta,
                &[rule("out", FileKind::Directory)]
            ),
            vec!["out/sibling"]
        );
    }
    #[test]
    fn allowed_new_parents_do_not_allow_siblings_or_existing_parent_removal() {
        let declaration = rule(
            "out/result",
            FileKind::File {
                mode: "exact_file".into(),
                expected_file: "expected".into(),
            },
        );
        let delta = vec![
            WorkspaceDelta {
                path: "out".into(),
                change: Change::Created,
            },
            WorkspaceDelta {
                path: "out/sibling".into(),
                change: Change::Created,
            },
        ];
        assert_eq!(
            undeclared_changes(&Snapshot::new(), &delta, &[declaration]),
            vec!["out/sibling"]
        );
        let before = BTreeMap::from([("out".into(), Entry::Directory)]);
        let delta = vec![WorkspaceDelta {
            path: "out".into(),
            change: Change::Removed,
        }];
        assert_eq!(
            undeclared_changes(&before, &delta, &[rule("out/result", FileKind::Directory)]),
            vec!["out"]
        );
    }
    #[test]
    fn absent_parent_allows_subtree_removal_only() {
        let delta = vec![
            WorkspaceDelta {
                path: "out/child".into(),
                change: Change::Removed,
            },
            WorkspaceDelta {
                path: "out/sibling".into(),
                change: Change::Created,
            },
        ];
        assert_eq!(
            undeclared_changes(&Snapshot::new(), &delta, &[rule("out", FileKind::Absent)]),
            vec!["out/sibling"]
        );
    }
}
