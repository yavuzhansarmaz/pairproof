//! Automatic startup update: check once at launch and re-exec if replaced.

use std::path::{Path, PathBuf};

use super::error::DistributionError;
use super::release::{ReleaseClient, ReqwestClient};
use super::updater::{UpdateOutcome, Updater};
use super::version::{parse_version, Version};

/// Guard set on the re-exec'd process so it does not check again.
pub const REEXEC_GUARD_ENV: &str = "PAIRPROOF_AUTO_UPDATE_REEXEC";

/// Outcome of a single startup update check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupOutcome {
    /// Installed binary is current; caller may proceed in this process.
    Current,
    /// Binary was replaced; caller must re-exec and never launch the TUI here.
    Updated,
}

/// Gate decision for the TUI entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupGate {
    /// Proceed to launch the TUI in this process.
    Proceed,
    /// A newer binary was installed and re-exec was requested.
    Reexec,
}

/// Alias used by the one-line gate call in `tui::run`.
pub type Gate = StartupGate;

/// Returns false only when the re-exec guard is exactly `"1"`.
pub fn should_run_startup_check(env_value: Option<&str>) -> bool {
    env_value != Some("1")
}

/// Production wrapper reading [`REEXEC_GUARD_ENV`] from the environment.
pub fn should_run_startup_check_from_env() -> bool {
    should_run_startup_check(std::env::var(REEXEC_GUARD_ENV).ok().as_deref())
}

/// Runs a single [`Updater::update`] call: only release metadata is
/// fetched when `current` is up to date, never an archive.
pub fn startup_check<C: ReleaseClient>(
    client: C,
    exe: PathBuf,
    current: &Version,
) -> Result<StartupOutcome, DistributionError> {
    match Updater::new(client, exe).update(current)? {
        UpdateOutcome::UpToDate | UpdateOutcome::CheckOnly { .. } => Ok(StartupOutcome::Current),
        UpdateOutcome::Updated { .. } => Ok(StartupOutcome::Updated),
    }
}

/// Preserves the original CLI arguments exactly, including `--role`.
pub fn reexec_args(original: &[String]) -> Vec<String> {
    original.to_vec()
}

/// Seam for replacing the current process image after an update.
pub trait Reexecer {
    /// Re-executes `exe` with `args`; production never returns on success.
    fn reexec(&self, exe: &Path, args: &[String]) -> Result<(), DistributionError>;
}

/// Production [`Reexecer`] using `exec` on unix.
#[derive(Debug, Default, Clone, Copy)]
pub struct ExecReexecer;

#[cfg(unix)]
impl Reexecer for ExecReexecer {
    fn reexec(&self, exe: &Path, args: &[String]) -> Result<(), DistributionError> {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(exe)
            .args(args)
            .env(REEXEC_GUARD_ENV, "1")
            .exec();
        Err(DistributionError::Io {
            path: exe.display().to_string(),
            source: err,
        })
    }
}

#[cfg(not(unix))]
impl Reexecer for ExecReexecer {
    fn reexec(&self, exe: &Path, _args: &[String]) -> Result<(), DistributionError> {
        Err(DistributionError::CurrentExe(format!(
            "re-exec after update is only supported on unix (exe: {})",
            exe.display()
        )))
    }
}

/// Testable gate: guard set means proceed with zero client calls;
/// an update re-execs with the original args; any error propagates.
pub fn startup_gate_with<C: ReleaseClient, R: Reexecer>(
    client: C,
    reexecer: R,
    exe: PathBuf,
    current: &Version,
    original_args: &[String],
    guard: Option<&str>,
) -> Result<StartupGate, DistributionError> {
    if !should_run_startup_check(guard) {
        return Ok(StartupGate::Proceed);
    }
    match startup_check(client, exe.clone(), current)? {
        StartupOutcome::Current => Ok(StartupGate::Proceed),
        StartupOutcome::Updated => {
            reexecer.reexec(&exe, &reexec_args(original_args))?;
            Ok(StartupGate::Reexec)
        }
    }
}

/// Production entry called by `tui::run` before launching the TUI.
pub fn ensure_latest_before_tui(original_args: &[String]) -> anyhow::Result<StartupGate> {
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;
    let exe =
        std::env::current_exe().map_err(|err| DistributionError::CurrentExe(err.to_string()))?;
    let guard = std::env::var(REEXEC_GUARD_ENV).ok();
    Ok(startup_gate_with(
        ReqwestClient,
        ExecReexecer,
        exe,
        &current,
        original_args,
        guard.as_deref(),
    )?)
}

/// One-line production gate alias for `tui::run`.
pub fn startup_gate(original_args: &[String]) -> anyhow::Result<Gate> {
    ensure_latest_before_tui(original_args)
}
