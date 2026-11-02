//! Safe removal of PairProof-owned files (`pairproof uninstall`).
//!
//! Only the running binary and, unless `--keep-data` is given, the
//! saved API key and its config directory are ever planned. User
//! project files (`sessions/`, `artifacts/`, `pairproof-workspace/`,
//! `connection-string*.txt`, `.pairproof/`, ...) are never listed.
//!
//! `remove_dir_all` on a real directory does not follow symlinks
//! inside it: symlink entries are removed as links, never traversed.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::error::DistributionError;

/// Kind of an app-owned path in a [`RemovalPlan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    /// The `pairproof` executable.
    Binary,
    /// The saved API key file.
    ApiKey,
    /// The `pairproof` config directory.
    ConfigDir,
}

impl PlanKind {
    fn label(self) -> &'static str {
        match self {
            PlanKind::Binary => "binary",
            PlanKind::ApiKey => "api key",
            PlanKind::ConfigDir => "config directory",
        }
    }
}

impl std::fmt::Display for PlanKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One app-owned path planned for removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    /// Absolute path to remove.
    pub path: PathBuf,
    /// What kind of app-owned path this is.
    pub kind: PlanKind,
    /// Human-readable description shown in the plan.
    pub description: String,
}

/// The closed set of paths an uninstall may remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalPlan {
    /// Entries in removal order (API key before its directory).
    pub entries: Vec<PlanEntry>,
}

/// Builds the removal plan for `exe` and `home`. Pure: checks nothing
/// on disk; missing paths are handled as success at execution time.
pub fn build_plan(exe: &Path, home: &Path, keep_data: bool) -> RemovalPlan {
    let mut entries = vec![PlanEntry {
        path: exe.to_path_buf(),
        kind: PlanKind::Binary,
        description: "PairProof binary".to_string(),
    }];
    if !keep_data {
        let config_dir = home.join(".config").join("pairproof");
        entries.push(PlanEntry {
            path: config_dir.join("api_key"),
            kind: PlanKind::ApiKey,
            description: "saved API key".to_string(),
        });
        entries.push(PlanEntry {
            path: config_dir,
            kind: PlanKind::ConfigDir,
            description: "PairProof config directory".to_string(),
        });
    }
    RemovalPlan { entries }
}

fn unsafe_path(path: &Path, reason: &str) -> DistributionError {
    DistributionError::UnsafePath {
        path: path.display().to_string(),
        reason: reason.to_string(),
    }
}

fn file_name_is(path: &Path, name: &str) -> bool {
    path.file_name().is_some_and(|n| n == name)
}

fn parent_name_is(path: &Path, name: &str) -> bool {
    path.parent().is_some_and(|p| file_name_is(p, name))
}

/// Validates one entry against the closed plan shape.
fn validate_entry(entry: &PlanEntry) -> Result<(), DistributionError> {
    let path = &entry.path;
    if path.as_os_str().is_empty() {
        return Err(unsafe_path(path, "path is empty"));
    }
    if !path.is_absolute() {
        return Err(unsafe_path(path, "path is not absolute"));
    }
    if path == Path::new("/") {
        return Err(unsafe_path(path, "path is the filesystem root"));
    }
    let ok = match entry.kind {
        PlanKind::Binary => file_name_is(path, "pairproof"),
        PlanKind::ApiKey => {
            file_name_is(path, "api_key")
                && parent_name_is(path, "pairproof")
                && path
                    .parent()
                    .and_then(Path::parent)
                    .is_some_and(|p| file_name_is(p, ".config"))
        }
        PlanKind::ConfigDir => file_name_is(path, "pairproof") && parent_name_is(path, ".config"),
    };
    if ok {
        Ok(())
    } else {
        Err(unsafe_path(
            path,
            "path does not match the expected app-owned location",
        ))
    }
}

fn write_line(out: &mut dyn Write, line: &str) -> Result<(), DistributionError> {
    writeln!(out, "{line}").map_err(|e| DistributionError::from_io(Path::new("stdout"), e))
}

/// Executes a validated plan. Nothing is removed when `confirmed`
/// is false or when any entry fails safety validation.
pub fn execute_plan(plan: &RemovalPlan, confirmed: bool) -> Result<Vec<String>, DistributionError> {
    if !confirmed {
        return Err(DistributionError::UninstallAborted);
    }
    for entry in &plan.entries {
        validate_entry(entry)?;
    }
    let mut report = Vec::new();
    for entry in &plan.entries {
        report.push(remove_entry(entry)?);
    }
    Ok(report)
}

fn remove_entry(entry: &PlanEntry) -> Result<String, DistributionError> {
    let path = &entry.path;
    // A symlinked config dir must never be traversed via the key path.
    if entry.kind == PlanKind::ApiKey {
        if let Some(parent) = path.parent() {
            if let Ok(md) = std::fs::symlink_metadata(parent) {
                if md.file_type().is_symlink() {
                    return Ok(format!("Skipped (parent is a symlink): {}", path.display()));
                }
            }
        }
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!("Not present (skipped): {}", path.display()));
        }
        Err(e) => return Err(DistributionError::from_io(path, e)),
    };
    if metadata.file_type().is_symlink() {
        if entry.kind == PlanKind::ConfigDir {
            return Ok(format!(
                "Skipped (symlink, not traversed): {}",
                path.display()
            ));
        }
        // Remove the link itself only; never follow it.
        std::fs::remove_file(path).map_err(|e| DistributionError::from_io(path, e))?;
        return Ok(format!("Removed symlink: {}", path.display()));
    }
    match entry.kind {
        PlanKind::ConfigDir => {
            if !metadata.is_dir() {
                return Ok(format!("Skipped (not a directory): {}", path.display()));
            }
            std::fs::remove_dir_all(path).map_err(|e| DistributionError::from_io(path, e))?;
            Ok(format!("Removed: {}", path.display()))
        }
        PlanKind::Binary | PlanKind::ApiKey => {
            if metadata.is_dir() {
                return Ok(format!("Skipped (not a file): {}", path.display()));
            }
            std::fs::remove_file(path).map_err(|e| DistributionError::from_io(path, e))?;
            Ok(format!("Removed: {}", path.display()))
        }
    }
}

/// Testable CLI core over an explicit plan: prints the plan, gates
/// on confirmation, executes, and prints the report.
pub fn uninstall_with_io(
    plan: &RemovalPlan,
    yes: bool,
    keep_data: bool,
    is_tty: bool,
    confirm: &dyn Fn() -> bool,
    out: &mut dyn Write,
) -> Result<(), DistributionError> {
    for entry in &plan.entries {
        write_line(
            out,
            &format!(
                "Will remove: {} ({})",
                entry.path.display(),
                entry.kind.label()
            ),
        )?;
    }
    if keep_data {
        write_line(
            out,
            "Keeping saved data: the API key and config directory will not be removed.",
        )?;
    }
    if !yes {
        if !is_tty {
            return Err(DistributionError::NonInteractiveConfirmation);
        }
        let _ = out.flush();
        if !confirm() {
            write_line(out, "Aborted.")?;
            return Ok(());
        }
    }
    for line in execute_plan(plan, true)? {
        write_line(out, &line)?;
    }
    write_line(out, "PairProof uninstalled.")?;
    if keep_data {
        write_line(
            out,
            "The binary was removed; any PATH entry for it is harmless and may be left in place.",
        )?;
    }
    Ok(())
}

/// Full flow for explicit `exe`/`home` paths (hermetic entry point).
pub fn run_uninstall_at(
    exe: &Path,
    home: &Path,
    yes: bool,
    keep_data: bool,
    is_tty: bool,
    confirm: &dyn Fn() -> bool,
    out: &mut dyn Write,
) -> Result<(), DistributionError> {
    let plan = build_plan(exe, home, keep_data);
    uninstall_with_io(&plan, yes, keep_data, is_tty, confirm, out)
}

/// Production flow: resolves the running exe and `$HOME`.
pub fn run_uninstall(
    yes: bool,
    keep_data: bool,
    is_tty: bool,
    confirm: &dyn Fn() -> bool,
    out: &mut dyn Write,
) -> Result<(), DistributionError> {
    let exe = std::env::current_exe().map_err(|e| DistributionError::CurrentExe(e.to_string()))?;
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or(DistributionError::MissingHome)?;
    run_uninstall_at(&exe, &home, yes, keep_data, is_tty, confirm, out)
}
