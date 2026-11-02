//! Expert live monitor view-model.
use crate::core::{ApproachChoice, CoreError, Session, SessionStatus};
use crate::scoring::{
    final_rating, live_rating, ApproachQuality, GradeOutcome, ScoreComponent, ScoringError,
};
use crate::tui::shared::sanitize_display;
use std::collections::BTreeMap;

/// Errors from monitor operations; core/scoring failures stay typed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MonitorError {
    /// Core session failure.
    #[error(transparent)]
    Core(#[from] CoreError),
    /// Scoring failure.
    #[error(transparent)]
    Scoring(#[from] ScoringError),
}

/// Deliberate expert end versus accidental disconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    ExpertEnd,
    AccidentalDisconnect,
}

/// Returns true when `state` is terminal.
pub fn is_final_after(state: SessionStatus) -> bool {
    state.is_terminal()
}

/// Render-ready live monitor data; no chat content by construction.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorView {
    pub current_question: Option<String>,
    pub current_index: Option<usize>,
    pub approach_label: Option<String>,
    pub latest_test: Option<String>,
    pub elapsed_ms: u64,
    pub per_question_ms: Option<u64>,
    pub rating: f64,
    pub rating_lines: Vec<String>,
    pub paste_alerts: Vec<String>,
    pub status_label: String,
}

fn score_line(slot_index: usize, comp: ScoreComponent, delta: f64, detail: &str) -> String {
    sanitize_display(&format!(
        "Q{} {:?} {:+.2}: {}",
        slot_index + 1,
        comp,
        delta,
        sanitize_display(detail)
    ))
}

/// Builds the monitor view from permitted expert-side data only.
pub fn monitor_view(
    session: &Session,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    now_ms: u64,
) -> MonitorView {
    let slot = session.current_slot();
    let approach_label = slot.and_then(|s| s.approach.as_ref()).map(|a| match a {
        ApproachChoice::Preset(n) => format!("preset {n}"),
        ApproachChoice::Other(t) => sanitize_display(t),
    });
    let earliest = session.slots().iter().filter_map(|s| s.started_at_ms).min();
    let breakdown = live_rating(session, qualities);
    MonitorView {
        current_question: slot.map(|s| sanitize_display(&s.question.title)),
        current_index: session.current_index(),
        approach_label,
        latest_test: slot
            .and_then(|s| s.latest_test)
            .map(|t| format!("{} passed, {} failed", t.passed, t.failed)),
        elapsed_ms: earliest.map_or(0, |m| now_ms.saturating_sub(m)),
        per_question_ms: slot.and_then(|s| match (s.started_at_ms, s.finished_at_ms) {
            (Some(a), Some(b)) => Some(b.saturating_sub(a)),
            (Some(a), None) => Some(now_ms.saturating_sub(a)),
            _ => None,
        }),
        rating: breakdown.rating,
        rating_lines: breakdown
            .lines
            .iter()
            .map(|l| score_line(l.slot_index, l.component, l.delta, &l.detail))
            .collect(),
        paste_alerts: session
            .slots()
            .iter()
            .filter(|s| s.paste_flags > 0)
            .map(|s| {
                format!(
                    "Paste flag on Q{}: {} confirmed bulk paste(s)",
                    s.index + 1,
                    s.paste_flags
                )
            })
            .collect(),
        status_label: sanitize_display(&format!("{:?}", session.status())),
    }
}

/// Renders the monitor view as deterministic lines.
pub fn render_monitor_lines(view: &MonitorView) -> Vec<String> {
    let q = match (&view.current_index, &view.current_question) {
        (Some(i), Some(t)) => format!("Q{}: {}", i + 1, t),
        _ => "none".into(),
    };
    let a = view.approach_label.as_deref().unwrap_or("none");
    let t = view.latest_test.as_deref().unwrap_or("no test run yet");
    let pq = view
        .per_question_ms
        .map_or("n/a".into(), |ms| format!("{ms} ms"));
    let mut lines = vec![
        format!("Status: {}", view.status_label),
        format!("Current: {q}"),
        format!("Approach: {a}"),
        format!("Latest test: {t}"),
        format!("Elapsed: {} ms", view.elapsed_ms),
        format!("Per-question: {pq}"),
        format!("Live rating: {:.2}", view.rating),
    ];
    if view.rating_lines.is_empty() {
        lines.push("Rating breakdown: none".into());
    } else {
        lines.extend(view.rating_lines.iter().map(|l| format!("  {l}")));
    }
    if view.paste_alerts.is_empty() {
        lines.push("Paste alerts: none".into());
    } else {
        lines.extend(view.paste_alerts.iter().cloned());
    }
    lines
}

/// Records accidental disconnect; session stays non-terminal.
pub fn handle_disconnect(session: &mut Session, at_ms: u64) -> Result<(), MonitorError> {
    session.candidate_disconnected(at_ms)?;
    Ok(())
}

/// Resumes after reconnect at the same current question.
pub fn handle_reconnect(session: &mut Session, at_ms: u64) -> Result<(), MonitorError> {
    session.candidate_connected(at_ms)?;
    Ok(())
}

/// Deliberately ends the session by expert action.
pub fn handle_expert_end(session: &mut Session, at_ms: u64) -> Result<(), MonitorError> {
    session.end_by_expert(at_ms)?;
    Ok(())
}

/// Cleanup is owned by orchestration lifecycle; re-exported here so
/// the monitor view and its consumers share the sole report type.
pub use crate::orchestration::lifecycle::{
    CancelOutcome, CleanupReport, CleanupSequence, CleanupStep, CLEANUP_STEP_NAMES,
};

/// Final breakdown view; report generation remains Phase 10.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalView {
    pub rating: f64,
    pub lines: Vec<String>,
    pub per_question: Vec<String>,
    pub solved_count: usize,
    pub finished_count: usize,
    pub skipped_count: usize,
    pub paste_flag_count: u32,
    pub total_time_ms: u64,
    pub not_scored: Vec<usize>,
    pub artefact_note: String,
}

/// Builds the final view by delegating to scoring.
pub fn final_view(
    session: &Session,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    grades: &[GradeOutcome],
) -> Result<FinalView, MonitorError> {
    let b = final_rating(session, qualities, grades)?;
    let per_question = b
        .per_question
        .iter()
        .map(|pq| {
            let title = session
                .slots()
                .iter()
                .find(|s| s.index == pq.slot_index)
                .map(|s| sanitize_display(&s.question.title))
                .unwrap_or_default();
            let grade = pq.grade.map_or("not scored".into(), |g| format!("{g:.1}"));
            let exp = pq.expected_tokens.map_or("n/a".into(), |t| t.to_string());
            let time = pq.time_ms.map_or("n/a".into(), |t| format!("{t} ms"));
            let test = pq
                .latest_test
                .map(|t| format!("{} passed, {} failed", t.passed, t.failed))
                .unwrap_or_else(|| "no test run yet".into());
            sanitize_display(&format!(
                "Q{} {}: grade {}, correction {:+.3}, tokens {}/{}, time {}, test {}",
                pq.slot_index + 1,
                title,
                grade,
                pq.correction,
                pq.actual_tokens,
                exp,
                time,
                test
            ))
        })
        .collect();
    Ok(FinalView {
        rating: b.rating,
        lines: b
            .lines
            .iter()
            .map(|l| score_line(l.slot_index, l.component, l.delta, &l.detail))
            .collect(),
        per_question,
        solved_count: b.solved_count,
        finished_count: b.finished_count,
        skipped_count: b.skipped_count,
        paste_flag_count: b.paste_flag_count,
        total_time_ms: b.total_time_ms,
        not_scored: b.not_scored,
        artefact_note:
            "Final breakdown computed; Markdown report generation is Phase 10 (not claimed here)."
                .into(),
    })
}
