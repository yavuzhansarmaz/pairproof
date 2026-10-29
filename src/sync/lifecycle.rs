//! Candidate-side Mutagen sync lifecycle over the session SSH endpoint.
//! Two-way-safe `pairproof-sync-<id>`; terminate drops sync, keeps files.

use std::time::Duration;

use crate::connection::{Endpoint, EphemeralUsername};
use crate::sandbox::error::bounded_stderr;
use crate::sandbox::process::{run_bounded, CommandOutcome, CommandSpec, DEFAULT_MAX_OUTPUT};
use crate::sandbox::SandboxError;

use super::command::{
    find_sync, mutagen_version, parse_mutagen_version, parse_sync_template, sync_create,
    sync_flush, sync_list, sync_list_template, sync_resume, sync_session_name, sync_terminate,
    validate_alpha_dir, SyncSessionInfo,
};
use super::error::SyncError;

/// Timeout for every mutagen runner in this module.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

// A missing mutagen binary is preflight-visible (7A MutagenInstalled check);
// timeouts keep their shape so callers can tell slowness from absence.
fn map_spawn(err: SandboxError) -> SyncError {
    match err {
        SandboxError::ExecutableNotFound { program } if program == "mutagen" => {
            SyncError::MutagenNotFound
        }
        SandboxError::Timeout {
            program,
            timeout_ms,
        } => SyncError::Timeout {
            program,
            timeout_ms,
        },
        other => SyncError::Sandbox(other),
    }
}

// Runs one spec, mapping spawn failures; builders run first so invalid ids
// or endpoints fail before any process is spawned.
async fn run_spec(spec: &CommandSpec) -> Result<CommandOutcome, SyncError> {
    run_bounded(spec, None, SYNC_TIMEOUT, DEFAULT_MAX_OUTPUT)
        .await
        .map_err(map_spawn)
}

// Runs a spec and requires exit zero, bounding stderr text.
async fn run_checked(
    spec: &CommandSpec,
    operation: &'static str,
) -> Result<CommandOutcome, SyncError> {
    let outcome = run_spec(spec).await?;
    if outcome.success() {
        Ok(outcome)
    } else {
        Err(SyncError::CommandFailed {
            operation,
            exit_code: outcome.exit_code,
            stderr: bounded_stderr(&outcome.stderr),
        })
    }
}

/// Reports the first line of `mutagen version`; absent binary is typed.
pub async fn check_mutagen_available() -> Result<String, SyncError> {
    let spec = mutagen_version()?;
    let outcome = run_checked(&spec, "mutagen_version").await?;
    parse_mutagen_version(&outcome.stdout_text())
}

/// Creates the two-way-safe session; validates id and alpha first.
pub async fn create_sync(
    session_id: &str,
    alpha: &str,
    endpoint: &Endpoint,
    user: &EphemeralUsername,
) -> Result<(), SyncError> {
    let spec = sync_create(session_id, alpha, endpoint, user)?;
    run_checked(&spec, "create_sync").await?;
    Ok(())
}

/// Runs the template query and returns our session by exact name match.
async fn template_status(name: &str) -> Result<Option<SyncSessionInfo>, SyncError> {
    let spec = sync_list_template()?;
    let outcome = run_checked(&spec, "sync_status").await?;
    Ok(parse_sync_template(&outcome.stdout_text())
        .into_iter()
        .find(|s| s.name == name))
}

/// Returns our session from the template query, or None when absent.
// Falls back to the human-readable list on old Mutagen without --template.
pub async fn sync_status(session_id: &str) -> Result<Option<SyncSessionInfo>, SyncError> {
    let name = sync_session_name(session_id)?;
    match template_status(&name).await {
        Ok(found) => Ok(found),
        Err(_) => {
            let spec = sync_list()?;
            let outcome = run_checked(&spec, "sync_status").await?;
            Ok(find_sync(&outcome.stdout_text(), &name))
        }
    }
}

/// Resumes a paused session; an already-running session counts as success.
// A failed resume that still leaves the session unpaused returns Ok.
pub async fn resume_sync(session_id: &str) -> Result<(), SyncError> {
    let spec = sync_resume(session_id)?;
    match run_checked(&spec, "resume_sync").await {
        Ok(_) => Ok(()),
        Err(first) => match template_status(&sync_session_name(session_id)?).await {
            Ok(Some(info)) if !info.paused => Ok(()),
            _ => Err(first),
        },
    }
}

/// Outcome of ensuring a sync session exists and runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureOutcome {
    Created,
    Resumed,
    AlreadyRunning,
}

/// Creates a missing session, resumes a paused one, or reports it running.
pub async fn ensure_sync(
    session_id: &str,
    alpha: &str,
    endpoint: &Endpoint,
    user: &EphemeralUsername,
) -> Result<EnsureOutcome, SyncError> {
    sync_session_name(session_id)?;
    validate_alpha_dir(alpha)?;
    match sync_status(session_id).await? {
        None => {
            create_sync(session_id, alpha, endpoint, user).await?;
            Ok(EnsureOutcome::Created)
        }
        Some(info) if info.paused => {
            resume_sync(session_id).await?;
            Ok(EnsureOutcome::Resumed)
        }
        Some(_) => Ok(EnsureOutcome::AlreadyRunning),
    }
}

/// Flushes pending sync changes for the session.
pub async fn flush_sync(session_id: &str) -> Result<(), SyncError> {
    let spec = sync_flush(session_id)?;
    run_checked(&spec, "flush_sync").await?;
    Ok(())
}

// Terminate removes the sync, never local files; a session already absent
// from `mutagen sync list` counts as success (idempotent by design).
pub async fn terminate_sync(session_id: &str) -> Result<(), SyncError> {
    let spec = sync_terminate(session_id)?;
    match run_checked(&spec, "terminate_sync").await {
        Ok(_) => Ok(()),
        Err(first) => match sync_status(session_id).await {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err(first),
            Err(_) => Err(first),
        },
    }
}
