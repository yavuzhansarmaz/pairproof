//! Candidate sync adapter mapping Phase 7 lifecycle results generically.
//! Full SSH transport/auth acceptance stays Phase 10; wrappers only expose the Phase 7 lifecycle results mapped generically.

use std::path::{Path, PathBuf};

use crate::connection::{Endpoint, EphemeralUsername};
use crate::core::Language;
use crate::sandbox::validate::question_layout;
use crate::sync::paste::Snapshot;
use crate::sync::{SyncError, SyncSessionInfo};
use crate::tui::shared::sanitize_display;

/// Maximum bytes read for one local solution view.
const MAX_SOLUTION_BYTES: u64 = 256 * 1024;

/// Candidate-safe sync phase; the single canonical type lives in
/// `crate::orchestration::sync_gate` (conflict-first).
pub use crate::orchestration::sync_gate::SessionSyncPhase as SyncPhase;

/// Maps optional session info to a candidate-safe phase; delegates to
/// the canonical conflict-first mapping in `crate::orchestration::sync_gate`.
pub use crate::orchestration::sync_gate::sync_phase_for as phase_for;

/// Render-ready sync view holding only candidate-safe lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncView {
    pub phase: SyncPhase,
    pub lines: Vec<String>,
}

/// Builds the sync view; only the phase label is rendered,
/// lowercased at the first letter for the candidate screen.
pub fn sync_view(info: Option<&SyncSessionInfo>) -> SyncView {
    let phase = phase_for(info);
    let label = phase.label();
    let mut chars = label.chars();
    let display = match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => label.to_string(),
    };
    SyncView {
        phase,
        lines: vec![sanitize_display(&display)],
    }
}

/// Returns candidate-safe sync lines for the session info.
pub fn sync_lines(info: Option<&SyncSessionInfo>) -> Vec<String> {
    sync_view(info).lines
}

/// Candidate-safe sync failure; raw diagnostics are never carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CandidateSyncError {
    #[error("sync problem \u{2014} ask the expert")]
    Unavailable,
    #[error("sync problem \u{2014} ask the expert")]
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

/// True when `root/q{index+1}/solution.*` exists as a non-empty file.
pub fn has_synced_solution(root: &Path, index: usize) -> bool {
    let dir = root.join(format!("q{}", index + 1));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("solution.")
            && !name.ends_with(".pairproof-tmp")
            && entry.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
    })
}

/// Lines for the files Mutagen has synced into `qN/`: the names,
/// how to run the solution, and a short preview of that file.
/// An empty directory, or one that is not there yet, yields nothing.
pub fn workspace_preview(root: &Path, index: usize) -> Vec<String> {
    workspace_preview_with_run(root, index, None)
}

/// Like [`workspace_preview`], appending the latest execution or test output.
pub fn workspace_preview_with_run(
    root: &Path,
    index: usize,
    latest_run: Option<&crate::orchestration::control_snapshot::SnapshotRun>,
) -> Vec<String> {
    let n = index + 1;
    let dir = root.join(format!("q{n}"));
    let mut names: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.metadata().is_ok_and(|m| m.is_file()))
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                if name.starts_with("solution.") && entry.metadata().map_or(0, |m| m.len()) == 0 {
                    return None;
                }
                Some(name)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    let run_lines = format_run_output(latest_run);
    if names.is_empty() && run_lines.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![String::new(), "Workspace".to_string()];
    for name in &names {
        lines.push(sanitize_display(&format!("q{n}/{name}")));
    }
    if let Some(solution) = names.iter().find(|name| name.starts_with("solution.")) {
        if let Some(hint) = run_hint(n, solution) {
            lines.push(hint);
        }
        let preview = file_preview(&dir.join(solution));
        if !preview.is_empty() {
            lines.push(String::new());
            lines.extend(preview);
        }
    }
    if !run_lines.is_empty() {
        lines.push(String::new());
        lines.extend(run_lines);
    }
    lines
}

fn format_run_output(
    latest_run: Option<&crate::orchestration::control_snapshot::SnapshotRun>,
) -> Vec<String> {
    let Some(run) = latest_run else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match run.kind.as_str() {
        "run" | "test" => {
            let no_tests = run.stdout.to_ascii_lowercase().contains("no tests ran");
            if no_tests {
                out.push("last run: no tests (press t to write tests)".to_string());
            } else {
                let label = if run.kind == "run" {
                    "last run"
                } else {
                    "last test"
                };
                let status = if run.success { "passed" } else { "failed" };
                let code_suffix = run
                    .exit_code
                    .map_or(String::new(), |c| format!(" (exit {c})"));
                out.push(sanitize_display(&format!("{label}: {status}{code_suffix}")));
            }
        }
        "code" if !run.success => {
            out.push("syntax check: failed".to_string());
        }
        _ => return Vec::new(),
    }
    for line in run.stdout.lines().take(8) {
        if !line.trim().is_empty() {
            out.push(sanitize_display(&format!("stdout: {line}")));
        }
    }
    for line in run.stderr.lines().take(8) {
        if !line.trim().is_empty() {
            out.push(sanitize_display(&format!("stderr: {line}")));
        }
    }
    out
}

fn run_hint(question: usize, file: &str) -> Option<String> {
    let path = format!("q{question}/{file}");
    let hint = if file.ends_with(".py") {
        format!("run: python3 {path}")
    } else if file.ends_with(".js") {
        format!("run: node {path}")
    } else if file.ends_with(".sql") {
        format!("solution file: {path}")
    } else {
        return None;
    };
    Some(sanitize_display(&hint))
}

/// First lines of a synced file, capped so a long solution cannot
/// take over the question pane.
fn file_preview(path: &Path) -> Vec<String> {
    const MAX_LINES: usize = 36;
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    if bytes.is_empty() || bytes.len() as u64 > MAX_SOLUTION_BYTES {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().take(MAX_LINES).map(sanitize_display).collect();
    if text.lines().nth(MAX_LINES).is_some() {
        lines.push("...".to_string());
    }
    lines
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
