//! Expert live monitor view-model and kill-switch controller.
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

/// Outcome of attempting to cancel in-flight work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    Cancelled,
    Unsupported,
}

/// Typed cancel seam; transport cannot cancel, Phase 10 wires it.
pub struct CancellationSeam;
impl CancellationSeam {
    /// Returns Cancelled only when caller reports support.
    pub fn try_cancel_inflight(supported: bool) -> CancelOutcome {
        try_cancel_inflight(supported)
    }
}

/// Returns Cancelled if supported else Unsupported (Phase 10).
pub fn try_cancel_inflight(supported: bool) -> CancelOutcome {
    if supported {
        CancelOutcome::Cancelled
    } else {
        CancelOutcome::Unsupported
    }
}

/// Record of every cleanup step attempted by the kill switch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanupReport {
    pub attempted: Vec<String>,
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub already_done: bool,
}
impl CleanupReport {
    /// Returns true when no attempted step failed.
    pub fn all_succeeded(&self) -> bool {
        self.failed.is_empty()
    }
    /// Derives the cancel outcome from the recorded step.
    pub fn cancel_outcome(&self) -> Option<CancelOutcome> {
        let n = "cancel_inflight";
        let bad = self
            .failed
            .iter()
            .any(|(s, m)| s == n && m.contains("unsupported"));
        if self.succeeded.iter().any(|s| s == n) {
            Some(CancelOutcome::Cancelled)
        } else if bad {
            Some(CancelOutcome::Unsupported)
        } else {
            None
        }
    }
}

/// Idempotence gate for shutdown; second begin is a no-op.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShutdownGate {
    done: bool,
}
impl ShutdownGate {
    /// Creates an unused gate.
    pub fn new() -> Self {
        Self::default()
    }
    /// Marks shutdown started; false when already done.
    pub fn begin(&mut self) -> bool {
        !std::mem::replace(&mut self.done, true)
    }
    /// Returns true after shutdown has begun.
    pub fn is_done(&self) -> bool {
        self.done
    }
}
pub type ShutdownState = ShutdownGate;

/// Kill-switch controller; stops new actions then runs all cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSwitch {
    done: bool,
    new_actions_allowed: bool,
}
impl KillSwitch {
    /// Creates a kill switch accepting new actions.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            done: false,
            new_actions_allowed: true,
        }
    }
    /// Returns true after execution.
    pub fn is_done(&self) -> bool {
        self.done
    }
    /// Returns false after the kill switch fires.
    pub fn accepts_new_actions(&self) -> bool {
        self.new_actions_allowed && !self.done
    }
    /// Runs every cleanup step in fixed order even after failure.
    pub fn execute(
        &mut self,
        cancel_supported: bool,
        term: impl FnOnce() -> Result<(), String>,
        sync: impl FnOnce() -> Result<(), String>,
        sandbox: impl FnOnce() -> Result<(), String>,
        raw: impl FnOnce() -> Result<(), String>,
    ) -> CleanupReport {
        if self.done {
            return CleanupReport {
                attempted: vec![],
                succeeded: vec![],
                failed: vec![],
                already_done: true,
            };
        }
        self.done = true;
        self.new_actions_allowed = false;
        let mut r = CleanupReport::default();
        r.attempted.push("cancel_inflight".into());
        match try_cancel_inflight(cancel_supported) {
            CancelOutcome::Cancelled => r.succeeded.push("cancel_inflight".into()),
            CancelOutcome::Unsupported => r.failed.push((
                "cancel_inflight".into(),
                "unsupported: transport cannot cancel (Phase 10 wire-up)".into(),
            )),
        }
        run_step(&mut r, "terminate_connection", term());
        run_step(&mut r, "stop_sync", sync());
        run_step(&mut r, "destroy_sandbox", sandbox());
        run_step(&mut r, "delete_raw", raw());
        r
    }
}

fn run_step(r: &mut CleanupReport, name: &str, result: Result<(), String>) {
    r.attempted.push(name.into());
    match result {
        Ok(()) => r.succeeded.push(name.into()),
        Err(e) => r.failed.push((name.into(), sanitize_display(&e))),
    }
}

/// Runs the kill switch via a fresh controller.
pub fn execute_kill_switch(
    cancel_supported: bool,
    term: impl FnOnce() -> Result<(), String>,
    sync: impl FnOnce() -> Result<(), String>,
    sandbox: impl FnOnce() -> Result<(), String>,
    raw: impl FnOnce() -> Result<(), String>,
) -> CleanupReport {
    KillSwitch::new().execute(cancel_supported, term, sync, sandbox, raw)
}

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
