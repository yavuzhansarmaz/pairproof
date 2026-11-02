//! Session sync orchestration with candidate-safe error mapping.

use std::fmt;

use crate::connection::{Endpoint, EphemeralUsername};
use crate::sync::{SyncError, SyncSessionInfo};

/// The exact candidate-safe sync failure message.
pub const CANDIDATE_SYNC_MESSAGE: &str = "Sync problem \u{2014} ask the expert";

/// Candidate-safe sync phase for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSyncPhase {
    NotStarted,
    Active,
    Paused,
    Conflict,
    Problem,
}

impl SessionSyncPhase {
    /// Returns the candidate-safe label for this phase.
    pub fn label(self) -> &'static str {
        match self {
            SessionSyncPhase::NotStarted => "Sync: not started",
            SessionSyncPhase::Active => "Sync: active",
            SessionSyncPhase::Paused => "Sync: paused",
            SessionSyncPhase::Conflict => "Sync: conflict needs attention \u{2014} ask the expert",
            SessionSyncPhase::Problem => CANDIDATE_SYNC_MESSAGE,
        }
    }
}

/// Maps optional session info to a phase; conflicts are never clean.
pub fn sync_phase_for(info: Option<&SyncSessionInfo>) -> SessionSyncPhase {
    match info {
        None => SessionSyncPhase::NotStarted,
        Some(info) if info.conflicts > 0 => SessionSyncPhase::Conflict,
        Some(info) if info.paused => SessionSyncPhase::Paused,
        Some(_) => SessionSyncPhase::Active,
    }
}

/// Sync failure with a candidate-safe face and expert-level detail.
#[derive(Clone, PartialEq, Eq)]
pub struct SyncGateError {
    expert: SyncError,
}

impl SyncGateError {
    /// Returns the exact candidate-safe message.
    pub fn candidate_message(&self) -> &'static str {
        CANDIDATE_SYNC_MESSAGE
    }

    /// Returns the typed expert-level error.
    pub fn expert_error(&self) -> &SyncError {
        &self.expert
    }

    /// Consumes the wrapper and returns the typed expert-level error.
    pub fn into_expert_error(self) -> SyncError {
        self.expert
    }
}

impl From<SyncError> for SyncGateError {
    fn from(expert: SyncError) -> Self {
        Self { expert }
    }
}

impl fmt::Display for SyncGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", CANDIDATE_SYNC_MESSAGE)
    }
}

impl fmt::Debug for SyncGateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyncGateError")
            .field("expert", &self.expert)
            .finish()
    }
}

impl std::error::Error for SyncGateError {}

/// Ensures the session sync exists, then returns its candidate-safe phase.
pub async fn ensure_session_sync(
    session_id: &str,
    alpha: &str,
    endpoint: &Endpoint,
    user: &EphemeralUsername,
) -> Result<SessionSyncPhase, SyncGateError> {
    crate::sync::ensure_sync(session_id, alpha, endpoint, user).await?;
    session_sync_phase(session_id).await
}

/// Returns the current candidate-safe sync phase for the session.
pub async fn session_sync_phase(session_id: &str) -> Result<SessionSyncPhase, SyncGateError> {
    let info = crate::sync::sync_status(session_id).await?;
    Ok(sync_phase_for(info.as_ref()))
}

/// Flushes pending sync changes for the session.
pub async fn flush_session_sync(session_id: &str) -> Result<(), SyncGateError> {
    crate::sync::flush_sync(session_id).await?;
    Ok(())
}

/// Terminates sync for the session; local files are never removed.
pub async fn terminate_session_sync(session_id: &str) -> Result<(), SyncGateError> {
    crate::sync::terminate_sync(session_id).await?;
    Ok(())
}
