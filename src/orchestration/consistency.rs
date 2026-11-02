//! Scoring-consistency harness: grade one recorded session twice.
//! Comparison is pure, preserves both runs, and invents no threshold.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::core::Session;
use crate::llm::grading::{scorable_slots, QuestionScoringInput};
use crate::orchestration::finalize::{FinalizeError, QuestionGrader};
use crate::scoring::{final_rating, ApproachQuality, FinalBreakdown, GradeOutcome, ScoringError};

/// Per-question comparison between two grading runs.
/// Missing outcome on one side is treated as not graded.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionDelta {
    pub slot_index: usize,
    pub first_grade: Option<f64>,
    pub second_grade: Option<f64>,
    pub grade_delta: Option<f64>,
    pub status_changed: bool,
    pub not_scored_drift: bool,
}

/// Both grading runs, their per-question deltas, and both ratings.
/// No averaging and no pass/fail variance threshold are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsistencyReport {
    pub first: Vec<GradeOutcome>,
    pub second: Vec<GradeOutcome>,
    pub per_question: Vec<QuestionDelta>,
    pub first_rating: f64,
    pub second_rating: f64,
    pub final_rating_delta: f64,
    pub max_grade_delta: Option<f64>,
    pub any_status_drift: bool,
    pub first_breakdown: FinalBreakdown,
    pub second_breakdown: FinalBreakdown,
}

fn grade_of(outcome: Option<&GradeOutcome>) -> Option<f64> {
    match outcome {
        Some(GradeOutcome::Graded(g)) => Some(g.grade),
        _ => None,
    }
}

fn is_graded(outcome: Option<&GradeOutcome>) -> bool {
    matches!(outcome, Some(GradeOutcome::Graded(_)))
}

fn slot_of(outcome: &GradeOutcome) -> usize {
    match outcome {
        GradeOutcome::Graded(g) => g.slot_index,
        GradeOutcome::NotScored { slot_index } => *slot_index,
    }
}

/// Compares two grading runs over the union of their slots, sorted.
/// Both final ratings delegate to `final_rating` once per run.
pub fn compare_grade_runs(
    session: &Session,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    first: &[GradeOutcome],
    second: &[GradeOutcome],
) -> Result<ConsistencyReport, ScoringError> {
    let first_breakdown = final_rating(session, qualities, first)?;
    let second_breakdown = final_rating(session, qualities, second)?;
    let first_rating = first_breakdown.rating;
    let second_rating = second_breakdown.rating;
    let first_map: BTreeMap<usize, &GradeOutcome> = first.iter().map(|o| (slot_of(o), o)).collect();
    let second_map: BTreeMap<usize, &GradeOutcome> =
        second.iter().map(|o| (slot_of(o), o)).collect();
    let slots: BTreeSet<usize> = first_map.keys().chain(second_map.keys()).copied().collect();
    let mut per_question = Vec::with_capacity(slots.len());
    for slot_index in slots {
        let first_outcome = first_map.get(&slot_index).copied();
        let second_outcome = second_map.get(&slot_index).copied();
        let first_grade = grade_of(first_outcome);
        let second_grade = grade_of(second_outcome);
        let grade_delta = match (first_grade, second_grade) {
            (Some(a), Some(b)) => Some((a - b).abs()),
            _ => None,
        };
        let status_changed = is_graded(first_outcome) != is_graded(second_outcome);
        per_question.push(QuestionDelta {
            slot_index,
            first_grade,
            second_grade,
            grade_delta,
            status_changed,
            not_scored_drift: status_changed,
        });
    }
    let max_grade_delta = per_question
        .iter()
        .filter_map(|d| d.grade_delta)
        .fold(None::<f64>, |acc, d| Some(acc.map_or(d, |a: f64| a.max(d))));
    let any_status_drift = per_question
        .iter()
        .any(|d| d.status_changed || d.not_scored_drift);
    Ok(ConsistencyReport {
        first: first.to_vec(),
        second: second.to_vec(),
        per_question,
        first_rating,
        second_rating,
        final_rating_delta: (first_rating - second_rating).abs(),
        max_grade_delta,
        any_status_drift,
        first_breakdown,
        second_breakdown,
    })
}

/// Grades the session's scorable slots twice with identical inputs.
/// Builds inputs once and never mutates the session.
pub async fn run_consistency(
    session: &Session,
    qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    grader: &dyn QuestionGrader,
    statements: &HashMap<String, String>,
    codes: &HashMap<usize, String>,
) -> Result<ConsistencyReport, FinalizeError> {
    let mut inputs: Vec<QuestionScoringInput> = Vec::new();
    for slot_index in scorable_slots(session) {
        let Some(slot) = session.slots().iter().find(|s| s.index == slot_index) else {
            continue;
        };
        let statement = statements
            .get(&slot.question.title_slug)
            .cloned()
            .unwrap_or_default();
        let code = codes.get(&slot_index).cloned().unwrap_or_default();
        if let Some(input) =
            QuestionScoringInput::from_session(session, slot_index, &statement, &code)
        {
            inputs.push(input);
        }
    }
    let first = grader.grade_all(&inputs).await;
    let second = grader.grade_all(&inputs).await;
    Ok(compare_grade_runs(session, qualities, &first, &second)?)
}
