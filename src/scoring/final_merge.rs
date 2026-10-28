//! Final rating: skipped-slot token settlement plus capped AI corrections.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::{QuestionStatus, Session, TestSummary};
use crate::leetcode::Difficulty;
use crate::scoring::live::{
    live_rating, multiplier_and_detail, token_band_step, ScoreComponent, ScoreLine,
};
use crate::scoring::types::{ApproachQuality, GradeOutcome, QuestionGrade, ScoringError};

/// Final rating with basis lines, capped AI correction, and per-question detail.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalBreakdown {
    pub rating: f64,
    pub unclamped: f64,
    pub lines: Vec<ScoreLine>,
    pub ai_correction_total: f64,
    pub per_question: Vec<PerQuestionFinal>,
    pub solved_count: usize,
    pub finished_count: usize,
    pub skipped_count: usize,
    pub paste_flag_count: u32,
    pub total_time_ms: u64,
    pub not_scored: Vec<usize>,
}

/// Final per-question record: grade, scaled correction, tokens, time, evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct PerQuestionFinal {
    pub slot_index: usize,
    pub status: QuestionStatus,
    pub difficulty: Difficulty,
    pub grade: Option<f64>,
    pub rationale: Option<String>,
    pub correction: f64,
    pub expected_tokens: Option<u64>,
    pub actual_tokens: u64,
    pub token_ratio: Option<f64>,
    pub latest_test: Option<TestSummary>,
    pub time_ms: Option<u64>,
    pub evidence: Vec<String>,
}

fn outcome_slot(outcome: &GradeOutcome) -> usize {
    match outcome {
        GradeOutcome::Graded(g) => g.slot_index,
        GradeOutcome::NotScored { slot_index } => *slot_index,
    }
}

fn validate_grades(session: &Session, grades: &[GradeOutcome]) -> Result<(), ScoringError> {
    let mut seen = BTreeSet::new();
    for outcome in grades {
        let slot_index = outcome_slot(outcome);
        let slot = session
            .slots()
            .iter()
            .find(|s| s.index == slot_index)
            .ok_or(ScoringError::UnknownSlot(slot_index))?;
        if !seen.insert(slot_index) {
            return Err(ScoringError::DuplicateGrade(slot_index));
        }
        if let GradeOutcome::Graded(g) = outcome {
            match slot.status {
                QuestionStatus::Skipped => return Err(ScoringError::GradeForSkipped(slot_index)),
                QuestionStatus::Finished => {}
                status => {
                    return Err(ScoringError::InvalidGrade(format!(
                        "grade for slot {slot_index} with status {status:?} is not finished"
                    )));
                }
            }
            QuestionGrade::new(g.slot_index, g.grade, g.rationale.clone(), g.correction)?;
        }
    }
    Ok(())
}

fn basis_lines(
    session: &Session,
    approach_qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
) -> Vec<ScoreLine> {
    let mut lines = live_rating(session, approach_qualities).lines;
    for slot in session.slots() {
        if slot.status != QuestionStatus::Skipped {
            continue;
        }
        let Some(lock) = session.token_locks(slot.index).last().copied() else {
            continue;
        };
        let ratio = slot.actual_tokens as f64 / lock as f64;
        let step = token_band_step(ratio).min(0.0);
        if step != 0.0 {
            let (mult, ac) = multiplier_and_detail(&slot.question);
            lines.push(ScoreLine {
                slot_index: slot.index,
                component: ScoreComponent::Tokens,
                delta: step * mult,
                detail: format!(
                    "tokens skipped {}/{} ratio {ratio:.3} step {step:+.2} x{mult:.2}{ac}",
                    slot.actual_tokens, lock
                ),
            });
        }
    }
    lines.sort_by_key(|l| (l.slot_index, l.component));
    lines
}

/// Computes the final rating by merging capped AI grades into the live basis.
pub fn final_rating(
    session: &Session,
    approach_qualities: &BTreeMap<usize, Vec<ApproachQuality>>,
    grades: &[GradeOutcome],
) -> Result<FinalBreakdown, ScoringError> {
    validate_grades(session, grades)?;
    let graded: BTreeMap<usize, &QuestionGrade> = grades
        .iter()
        .filter_map(|o| match o {
            GradeOutcome::Graded(g) => Some((g.slot_index, g)),
            GradeOutcome::NotScored { .. } => None,
        })
        .collect();
    let raw_sum: f64 = graded.values().map(|g| g.correction).sum();
    let factor = if raw_sum.abs() > 0.8 {
        0.8 / raw_sum.abs()
    } else {
        1.0
    };
    let scaled: BTreeMap<usize, f64> = graded
        .iter()
        .map(|(i, g)| (*i, g.correction * factor))
        .collect();
    let ai_correction_total: f64 = scaled.values().sum();
    let lines = basis_lines(session, approach_qualities);
    let unclamped = 6.0 + lines.iter().map(|l| l.delta).sum::<f64>() + ai_correction_total;
    let mut per_question = Vec::new();
    let mut not_scored = Vec::new();
    let mut total_time_ms: u64 = 0;
    for slot in session.slots() {
        let grade = graded.get(&slot.index).copied();
        let correction = scaled.get(&slot.index).copied().unwrap_or(0.0);
        let expected_tokens = session.token_locks(slot.index).last().copied();
        let token_ratio = expected_tokens.map(|lock| slot.actual_tokens as f64 / lock as f64);
        let time_ms = match (slot.started_at_ms, slot.finished_at_ms) {
            (Some(start), Some(end)) => Some(end.saturating_sub(start)),
            _ => None,
        };
        if let Some(t) = time_ms {
            total_time_ms = total_time_ms.saturating_add(t);
        }
        if slot.status == QuestionStatus::Finished && grade.is_none() {
            not_scored.push(slot.index);
        }
        let mut evidence: Vec<String> = lines
            .iter()
            .filter(|l| l.slot_index == slot.index)
            .map(|l| l.detail.clone())
            .collect();
        evidence.push(match grade {
            Some(g) => format!(
                "grade {:.1} correction {correction:+.3}: {}",
                g.grade, g.rationale
            ),
            None => match slot.status {
                QuestionStatus::Finished => {
                    format!("finished with {} tokens but not scored", slot.actual_tokens)
                }
                QuestionStatus::Skipped => {
                    format!("skipped after {} tokens", slot.actual_tokens)
                }
                status => format!(
                    "slot {} status {status:?} with {} tokens, not scored",
                    slot.index, slot.actual_tokens
                ),
            },
        });
        per_question.push(PerQuestionFinal {
            slot_index: slot.index,
            status: slot.status,
            difficulty: slot.question.difficulty,
            grade: grade.map(|g| g.grade),
            rationale: grade.map(|g| g.rationale.clone()),
            correction,
            expected_tokens,
            actual_tokens: slot.actual_tokens,
            token_ratio,
            latest_test: slot.latest_test,
            time_ms,
            evidence,
        });
    }
    let count = |st: QuestionStatus| session.slots().iter().filter(|s| s.status == st).count();
    Ok(FinalBreakdown {
        rating: unclamped.clamp(3.0, 10.0),
        unclamped,
        lines,
        ai_correction_total,
        per_question,
        solved_count: graded.values().filter(|g| g.grade >= 5.0).count(),
        finished_count: count(QuestionStatus::Finished),
        skipped_count: count(QuestionStatus::Skipped),
        paste_flag_count: session.paste_flag_count(),
        total_time_ms,
        not_scored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_support::*;
    use crate::core::types::ApproachChoice;

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn graded(slot: usize, grade: f64, correction: f64) -> GradeOutcome {
        GradeOutcome::Graded(
            QuestionGrade::new(slot, grade, format!("rationale {slot}"), correction).unwrap(),
        )
    }

    fn two_finished() -> Session {
        let mut s = started();
        s.finish_question(10).unwrap();
        s.finish_question(11).unwrap();
        s
    }

    #[test]
    fn cap_scales_down_proportionally_above_point_eight() {
        let s = two_finished();
        let b = final_rating(
            &s,
            &BTreeMap::new(),
            &[graded(0, 9.0, 1.0), graded(1, 8.0, 0.6)],
        )
        .unwrap();
        close(b.ai_correction_total, 0.8);
        close(b.per_question[0].correction, 0.5);
        close(b.per_question[1].correction, 0.3);
        close(b.rating, 6.8);
        let empty = final_rating(&s, &BTreeMap::new(), &[]).unwrap();
        close(empty.ai_correction_total, 0.0);
        close(empty.rating, 6.0);
    }

    #[test]
    fn cap_boundary_exact_point_eight_unscaled() {
        let s = two_finished();
        let pos = final_rating(
            &s,
            &BTreeMap::new(),
            &[graded(0, 9.0, 0.5), graded(1, 8.0, 0.3)],
        )
        .unwrap();
        close(pos.ai_correction_total, 0.8);
        close(pos.per_question[0].correction, 0.5);
        close(pos.per_question[1].correction, 0.3);
        let neg = final_rating(
            &s,
            &BTreeMap::new(),
            &[graded(0, 3.0, -0.5), graded(1, 4.0, -0.3)],
        )
        .unwrap();
        close(neg.ai_correction_total, -0.8);
        close(neg.per_question[0].correction, -0.5);
        close(neg.per_question[1].correction, -0.3);
        let beyond = final_rating(
            &s,
            &BTreeMap::new(),
            &[graded(0, 3.0, -1.0), graded(1, 4.0, -0.6)],
        )
        .unwrap();
        close(beyond.ai_correction_total, -0.8);
        close(beyond.per_question[0].correction, -0.5);
        close(beyond.per_question[1].correction, -0.3);
        close(beyond.rating, 5.2);
    }

    #[test]
    fn skipped_tokens_settled_in_final_but_not_live() {
        let mut s = started();
        s.select_approach(ApproachChoice::Preset(0), 100, 5)
            .unwrap();
        s.add_tokens(200).unwrap();
        s.skip_question(6).unwrap();
        s.select_approach(ApproachChoice::Preset(0), 1000, 7)
            .unwrap();
        s.add_tokens(100).unwrap();
        s.skip_question(8).unwrap();
        let live = live_rating(&s, &BTreeMap::new());
        assert!(live
            .lines
            .iter()
            .all(|l| l.component != ScoreComponent::Tokens));
        close(live.rating, 6.0);
        let b = final_rating(&s, &BTreeMap::new(), &[]).unwrap();
        let tokens: Vec<&ScoreLine> = b
            .lines
            .iter()
            .filter(|l| l.component == ScoreComponent::Tokens)
            .collect();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].slot_index, 0);
        close(tokens[0].delta, -0.15);
        assert!(tokens[0].detail.contains("skipped"), "{}", tokens[0].detail);
        assert!(tokens[0].detail.contains("200/100"), "{}", tokens[0].detail);
        close(b.rating, 5.85);
        close(b.per_question[1].token_ratio.unwrap(), 0.1);
    }

    #[test]
    fn grade_for_skipped_is_error() {
        let mut s = started();
        s.skip_question(5).unwrap();
        assert_eq!(
            final_rating(&s, &BTreeMap::new(), &[graded(0, 5.0, 0.1)]),
            Err(ScoringError::GradeForSkipped(0))
        );
        assert!(final_rating(
            &s,
            &BTreeMap::new(),
            &[GradeOutcome::NotScored { slot_index: 0 }]
        )
        .is_ok());
    }

    #[test]
    fn unknown_slot_grade_is_error() {
        let s = two_finished();
        assert_eq!(
            final_rating(&s, &BTreeMap::new(), &[graded(99, 5.0, 0.1)]),
            Err(ScoringError::UnknownSlot(99))
        );
        assert_eq!(
            final_rating(
                &s,
                &BTreeMap::new(),
                &[GradeOutcome::NotScored { slot_index: 99 }]
            ),
            Err(ScoringError::UnknownSlot(99))
        );
    }

    #[test]
    fn duplicate_grade_is_error() {
        let s = two_finished();
        assert_eq!(
            final_rating(
                &s,
                &BTreeMap::new(),
                &[
                    graded(0, 9.0, 0.1),
                    GradeOutcome::NotScored { slot_index: 0 }
                ]
            ),
            Err(ScoringError::DuplicateGrade(0))
        );
        assert_eq!(
            final_rating(
                &s,
                &BTreeMap::new(),
                &[graded(1, 9.0, 0.1), graded(1, 8.0, 0.2)]
            ),
            Err(ScoringError::DuplicateGrade(1))
        );
        for (slot, name) in [(2, "Active"), (4, "Pending")] {
            match final_rating(&s, &BTreeMap::new(), &[graded(slot, 5.0, 0.1)]) {
                Err(ScoringError::InvalidGrade(m)) => assert!(m.contains(name), "{m}"),
                other => panic!("expected InvalidGrade, got {other:?}"),
            }
        }
    }
}
