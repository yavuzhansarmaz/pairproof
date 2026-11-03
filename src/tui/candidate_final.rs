//! Candidate final view reusing the expert final breakdown.

use std::collections::BTreeMap;

use crate::core::{Session, SessionStatus};
use crate::scoring::{ApproachQuality, GradeOutcome};
use crate::tui::monitor::{final_view, FinalView, MonitorError};
use crate::tui::shared::sanitize_display;
use crate::tui::viewport::visible_lines;

/// Candidate-facing final outcome; technical failure has no rating.
#[derive(Debug, Clone, PartialEq)]
pub enum CandidateFinal {
    Breakdown(FinalView),
    TechnicalFailure,
}

/// Typed candidate final failure; monitor failures stay typed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CandidateFinalError {
    /// The session has not reached a final state yet.
    #[error("session is not final yet.")]
    NotFinal,
    /// Monitor final-view failure.
    #[error(transparent)]
    Monitor(#[from] MonitorError),
}

/// Builds the candidate final view without recomputing any scores.
pub fn candidate_final(
    session: &Session,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    grades: &[GradeOutcome],
) -> Result<CandidateFinal, CandidateFinalError> {
    match session.status() {
        SessionStatus::Completed | SessionStatus::EndedByExpert => Ok(CandidateFinal::Breakdown(
            final_view(session, qualities, grades)?,
        )),
        SessionStatus::TechnicalFailure => Ok(CandidateFinal::TechnicalFailure),
        _ => Err(CandidateFinalError::NotFinal),
    }
}

/// Builds sanitized candidate final lines from the final outcome.
pub fn candidate_final_lines(final_result: &CandidateFinal) -> Vec<String> {
    let lines: Vec<String> = match final_result {
        CandidateFinal::Breakdown(view) => {
            let mut lines = vec![
                "Final breakdown".to_string(),
                String::new(),
                format!("final rating: {:.2}", view.rating),
                String::new(),
                format!("solved: {}", view.solved_count),
                format!("finished: {}", view.finished_count),
                format!("skipped: {}", view.skipped_count),
                format!("paste flags: {}", view.paste_flag_count),
            ];
            if !view.per_question.is_empty() {
                lines.push(String::new());
                lines.push("Questions".to_string());
                for block in &view.per_question {
                    lines.push(String::new());
                    lines.extend(block.split('\n').map(str::to_string));
                }
            }
            if !view.lines.is_empty() {
                lines.push(String::new());
                lines.push("How the rating moved".to_string());
                lines.extend(view.lines.iter().cloned());
            }
            lines
        }
        CandidateFinal::TechnicalFailure => vec![
            "the session ended as a technical failure.".to_string(),
            "no evaluation was produced; the interview can be rescheduled.".to_string(),
        ],
    };
    lines.iter().map(|line| sanitize_display(line)).collect()
}

/// Returns the pinned completion lines kept visible on short terminals.
pub fn candidate_final_pinned_lines(final_result: &CandidateFinal) -> Vec<String> {
    match final_result {
        CandidateFinal::Breakdown(view) => vec![
            format!("final rating: {:.2}", view.rating),
            format!("solved: {}", view.solved_count),
            format!("finished: {}", view.finished_count),
            format!("skipped: {}", view.skipped_count),
            format!("paste flags: {}", view.paste_flag_count),
        ],
        CandidateFinal::TechnicalFailure => Vec::new(),
    }
    .iter()
    .map(|line| sanitize_display(line))
    .collect()
}

/// Builds final lines for a terminal height, pinning the summary.
pub fn candidate_final_lines_for_height(
    final_result: &CandidateFinal,
    height: usize,
) -> Vec<String> {
    match final_result {
        CandidateFinal::Breakdown(view) => {
            let mut content = vec!["Final breakdown".to_string(), String::new()];
            if !view.per_question.is_empty() {
                content.push("Questions".to_string());
                for block in &view.per_question {
                    content.push(String::new());
                    content.extend(block.split('\n').map(str::to_string));
                }
            }
            if !view.lines.is_empty() {
                content.push(String::new());
                content.push("How the rating moved".to_string());
                content.extend(view.lines.iter().cloned());
            }
            let pinned = candidate_final_pinned_lines(final_result);
            visible_lines(&content, height, &pinned)
                .iter()
                .map(|line| sanitize_display(line))
                .collect()
        }
        CandidateFinal::TechnicalFailure => {
            let lines = candidate_final_lines(final_result);
            visible_lines(&lines, height, &[])
        }
    }
}
