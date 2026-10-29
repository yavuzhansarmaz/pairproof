//! Candidate sync adapter mapping Phase 7 lifecycle results generically.
//! Full SSH transport/auth acceptance stays Phase 10; wrappers only expose the Phase 7 lifecycle results mapped generically.

use std::path::{Path, PathBuf};

use crate::connection::{Endpoint, EphemeralUsername};
use crate::core::Language;
use crate::sandbox::validate::question_layout;
use crate::sync::paste::Snapshot;
use crate::sync::{SyncError, SyncSessionInfo};
use crate::tui::candidate::candidate_sync_message;
use crate::tui::shared::sanitize_display;

/// Maximum bytes read for one local solution view.
const MAX_SOLUTION_BYTES: u64 = 256 * 1024;

/// Candidate-safe sync phase derived from lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPhase {
    NotStarted,
    Active,
    Paused,
    Conflict,
    Problem,
}

impl SyncPhase {
    /// Returns the candidate-safe label for this phase.
    pub fn label(self) -> &'static str {
        match self {
            SyncPhase::NotStarted => "Sync: not started",
            SyncPhase::Active => "Sync: active",
            SyncPhase::Paused => "Sync: paused",
            SyncPhase::Conflict => "Sync: conflict needs attention \u{2014} ask the expert",
            SyncPhase::Problem => candidate_sync_message(),
        }
    }
}

/// Maps optional session info to a candidate-safe phase.
pub fn phase_for(info: Option<&SyncSessionInfo>) -> SyncPhase {
    match info {
        None => SyncPhase::NotStarted,
        Some(info) if info.paused => SyncPhase::Paused,
        Some(info) if info.conflicts > 0 => SyncPhase::Conflict,
        Some(_) => SyncPhase::Active,
    }
}

/// Render-ready sync view holding only candidate-safe lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncView {
    pub phase: SyncPhase,
    pub lines: Vec<String>,
}

/// Builds the sync view; only the phase label is rendered.
pub fn sync_view(info: Option<&SyncSessionInfo>) -> SyncView {
    let phase = phase_for(info);
    SyncView {
        phase,
        lines: vec![sanitize_display(phase.label())],
    }
}

/// Returns candidate-safe sync lines for the session info.
pub fn sync_lines(info: Option<&SyncSessionInfo>) -> Vec<String> {
    sync_view(info).lines
}

/// Candidate-safe sync failure; raw diagnostics are never carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CandidateSyncError {
    #[error("Sync problem \u{2014} ask the expert")]
    Unavailable,
    #[error("Sync problem \u{2014} ask the expert")]
    Failed,
}

/// Maps a raw sync error to a generic candidate-safe error.
pub fn map_sync_error(err: &SyncError) -> CandidateSyncError {
    match err {
        SyncError::MutagenNotFound => CandidateSyncError::Unavailable,
        _ => CandidateSyncError::Failed,
    }
}

/// Ensures sync via the lifecycle API, then returns its phase.
pub async fn candidate_ensure_sync(
    session_id: &str,
    alpha: &str,
    endpoint: &Endpoint,
    user: &EphemeralUsername,
) -> Result<SyncPhase, CandidateSyncError> {
    crate::sync::ensure_sync(session_id, alpha, endpoint, user)
        .await
        .map_err(|e| map_sync_error(&e))?;
    candidate_sync_status(session_id).await
}

/// Returns the current sync phase for the session.
pub async fn candidate_sync_status(session_id: &str) -> Result<SyncPhase, CandidateSyncError> {
    let info = crate::sync::sync_status(session_id)
        .await
        .map_err(|e| map_sync_error(&e))?;
    Ok(phase_for(info.as_ref()))
}

/// Flushes pending sync changes for the session.
pub async fn candidate_flush_sync(session_id: &str) -> Result<(), CandidateSyncError> {
    crate::sync::flush_sync(session_id)
        .await
        .map_err(|e| map_sync_error(&e))
}

/// Terminates sync for the session; local files are kept.
pub async fn candidate_terminate_sync(session_id: &str) -> Result<(), CandidateSyncError> {
    crate::sync::terminate_sync(session_id)
        .await
        .map_err(|e| map_sync_error(&e))
}

/// Local workspace progress for one solution file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolutionProgress {
    Missing,
    Empty,
    Present { bytes: u64, lines: u64 },
}

/// Resolves the local solution path from the question layout.
fn solution_path(alpha: &Path, index: usize, language: Language) -> Option<PathBuf> {
    let (_, solution) = question_layout(index, language).ok()?;
    Some(alpha.join(solution.as_str()))
}

/// Counts logical lines: newline count plus a trailing partial line.
fn line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    if bytes.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    }
}

/// Reports local solution progress without modifying the file.
pub fn solution_progress(alpha: &Path, index: usize, language: Language) -> SolutionProgress {
    let Some(path) = solution_path(alpha, index, language) else {
        return SolutionProgress::Missing;
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return SolutionProgress::Missing;
    };
    if !meta.is_file() {
        return SolutionProgress::Missing;
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return SolutionProgress::Missing;
    };
    if bytes.is_empty() {
        SolutionProgress::Empty
    } else {
        SolutionProgress::Present {
            bytes: bytes.len() as u64,
            lines: line_count(&bytes),
        }
    }
}

/// Reads the local solution text, capped at 256 KiB.
pub fn read_local_solution(alpha: &Path, index: usize, language: Language) -> Option<String> {
    let path = solution_path(alpha, index, language)?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_SOLUTION_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    if bytes.len() as u64 > MAX_SOLUTION_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Captures a paste-detection snapshot of the local solution.
pub fn solution_snapshot(alpha: &Path, index: usize, language: Language) -> Option<Snapshot> {
    let path = solution_path(alpha, index, language)?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some(Snapshot::from_bytes(&bytes))
}
