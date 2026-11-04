use std::collections::BTreeMap;

use pairproof::core::{ApproachChoice, Language, QuestionRef, QuestionStatus, Session};
use pairproof::leetcode::Difficulty;
use pairproof::scoring::{
    final_rating, ApproachQuality, GradeOutcome, QuestionGrade, ScoreComponent, ScoringError,
};
use ApproachQuality::*;

use crate::common_mod;
use common_mod::{q, qs};

fn q_acceptance(slug: &str, difficulty: Difficulty, acceptance_pct: Option<u32>) -> QuestionRef {
    let mut question = q(slug, difficulty);
    question.acceptance_pct = acceptance_pct;
    question
}

fn started_easy_acceptance(acceptance_pct: Option<u32>) -> Session {
    let mut questions = qs("e", 5, Difficulty::Easy);
    questions[0] = q_acceptance("e0", Difficulty::Easy, acceptance_pct);
    let mut s = Session::new("s1".into(), Language::Python, questions, 0).expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

fn base() -> Session {
    Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session")
}

fn started_easy() -> Session {
    let mut s = base();
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s
}

fn started_expert() -> Session {
    let mut s = base();
    s.set_expert_questions(
        qs("m", 3, Difficulty::Medium),
        qs("h", 2, Difficulty::Hard),
        1,
    )
    .expect("expert set");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

fn graded(slot: usize, grade: f64, rationale: &str, correction: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, rationale.to_string(), correction).unwrap(),
    )
}

#[test]
fn not_scored_contributes_zero_and_is_listed() {
    let mut s = started_easy();
    s.finish_question(10).unwrap();
    s.finish_question(11).unwrap();
    let grades = [
        GradeOutcome::NotScored { slot_index: 0 },
        graded(1, 8.0, "solid work", 0.2),
    ];
    let b = final_rating(&s, &BTreeMap::new(), &grades).unwrap();
    close(b.rating, 6.0 + 0.2 * pairproof::scoring::POSITIVE_GAIN_SCALE);
    close(b.ai_correction_total, 0.2);
    assert_eq!(b.not_scored, vec![0]);
    close(b.per_question[0].correction, 0.0);
    assert_eq!(b.per_question[0].grade, None);
    assert!(b.per_question[0]
        .evidence
        .iter()
        .any(|e| e.contains("not scored")));
}

#[test]
fn missing_grade_entry_treated_not_scored() {
    let mut s = started_easy();
    s.finish_question(10).unwrap();
    s.finish_question(11).unwrap();
    let b = final_rating(&s, &BTreeMap::new(), &[graded(0, 8.0, "good", 0.3)]).unwrap();
    assert_eq!(b.not_scored, vec![1]);
    close(b.per_question[1].correction, 0.0);
    assert_eq!(b.per_question[1].grade, None);
    assert_eq!(b.per_question[1].rationale, None);
    close(b.rating, 6.0 + 0.3 * pairproof::scoring::POSITIVE_GAIN_SCALE);
}

#[test]
fn solved_count_uses_grade_threshold() {
    let mut s = started_easy();
    s.finish_question(10).unwrap();
    s.finish_question(11).unwrap();
    s.finish_question(12).unwrap();
    let grades = [
        graded(0, 5.0, "borderline pass", 0.0),
        graded(1, 4.9, "just below", 0.0),
        graded(2, 9.5, "excellent", 0.0),
    ];
    let b = final_rating(&s, &BTreeMap::new(), &grades).unwrap();
    assert_eq!(b.solved_count, 2);
    assert_eq!(b.finished_count, 3);
    assert!(b.not_scored.is_empty());
}

#[test]
fn final_breakdown_counts_and_times() {
    let mut s = started_expert();
    s.select_approach(ApproachChoice::Preset(0), 100, 10)
        .unwrap();
    s.add_tokens(50).unwrap();
    s.record_paste_flag(500, 11).unwrap();
    s.finish_question(104).unwrap();
    s.skip_question(204).unwrap();
    s.select_approach(ApproachChoice::Preset(1), 200, 210)
        .unwrap();
    s.add_tokens(300).unwrap();
    s.record_test_result(4, 1, 220).unwrap();
    s.finish_question(354).unwrap();
    let b = final_rating(&s, &BTreeMap::new(), &[]).unwrap();
    assert_eq!(b.per_question.len(), 10);
    assert_eq!(b.finished_count, 2);
    assert_eq!(b.skipped_count, 1);
    assert_eq!(b.paste_flag_count, 1);
    assert_eq!(b.total_time_ms, 350);
    assert_eq!(b.per_question[0].time_ms, Some(100));
    assert_eq!(b.per_question[1].time_ms, Some(100));
    assert_eq!(b.per_question[2].time_ms, Some(150));
    assert_eq!(b.per_question[3].status, QuestionStatus::Active);
    assert_eq!(b.per_question[3].time_ms, None);
    assert_eq!(b.per_question[4].status, QuestionStatus::Pending);
    assert_eq!(b.per_question[4].time_ms, None);
    assert_eq!(b.per_question[0].expected_tokens, Some(100));
    close(b.per_question[0].token_ratio.unwrap(), 0.5);
    assert_eq!(b.per_question[1].expected_tokens, None);
    assert_eq!(b.per_question[1].token_ratio, None);
    assert_eq!(b.per_question[2].actual_tokens, 300);
    close(b.per_question[2].token_ratio.unwrap(), 1.5);
    let test = b.per_question[2].latest_test.expect("test summary");
    assert_eq!((test.passed, test.failed), (4, 1));
    assert_eq!(b.not_scored, vec![0, 2]);
}

#[test]
fn per_question_evidence_references_numbers() {
    let mut s = started_easy();
    s.select_approach(ApproachChoice::Preset(0), 1000, 10)
        .unwrap();
    s.add_tokens(700).unwrap();
    s.finish_question(11).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 100, 12)
        .unwrap();
    s.add_tokens(250).unwrap();
    s.skip_question(13).unwrap();
    s.finish_question(14).unwrap();
    let map = BTreeMap::from([(0, vec![Best])]);
    let grades = [graded(0, 9.0, "clean heap solution", 0.25)];
    let b = final_rating(&s, &map, &grades).unwrap();
    let ev0 = b.per_question[0].evidence.join(" | ");
    assert!(ev0.contains("700/1000"), "{ev0}");
    assert!(ev0.contains("9.0"), "{ev0}");
    assert!(ev0.contains("clean heap solution"), "{ev0}");
    assert!(ev0.contains("+0.250"), "{ev0}");
    let ev1 = b.per_question[1].evidence.join(" | ");
    assert!(ev1.contains("skipped"), "{ev1}");
    assert!(ev1.contains("250/100"), "{ev1}");
    let ev2 = b.per_question[2].evidence.join(" | ");
    assert!(ev2.contains("not scored"), "{ev2}");
    for pq in &b.per_question {
        assert!(!pq.evidence.is_empty(), "slot {}", pq.slot_index);
        assert!(
            pq.evidence.join(" ").chars().any(|c| c.is_ascii_digit()),
            "slot {} evidence {:?}",
            pq.slot_index,
            pq.evidence
        );
    }
}

#[test]
fn golden_final_rating_over_scripted_session() {
    let mut s = started_expert();
    s.select_approach(ApproachChoice::Preset(0), 1000, 10)
        .unwrap();
    s.add_tokens(700).unwrap();
    s.finish_question(11).unwrap();
    s.select_approach(ApproachChoice::Preset(1), 500, 12)
        .unwrap();
    s.add_tokens(1000).unwrap();
    s.redirect("use a heap", 13).unwrap();
    s.finish_question(14).unwrap();
    s.select_approach(ApproachChoice::Preset(2), 400, 15)
        .unwrap();
    s.add_tokens(100).unwrap();
    s.change_approach(16).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 600, 17)
        .unwrap();
    s.add_tokens(200).unwrap();
    s.record_paste_flag(800, 18).unwrap();
    s.finish_question(19).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 100, 20)
        .unwrap();
    s.add_tokens(200).unwrap();
    s.skip_question(21).unwrap();
    s.select_approach(ApproachChoice::Preset(3), 800, 22)
        .unwrap();
    s.add_tokens(800).unwrap();
    s.finish_question(23).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 1000, 24)
        .unwrap();
    s.add_tokens(500).unwrap();
    s.finish_question(25).unwrap();
    s.select_approach(ApproachChoice::Preset(1), 1000, 26)
        .unwrap();
    s.add_tokens(1500).unwrap();
    s.redirect("mind the edge case", 27).unwrap();
    s.finish_question(28).unwrap();
    s.skip_question(29).unwrap();
    s.select_approach(ApproachChoice::Preset(0), 2000, 30)
        .unwrap();
    s.add_tokens(1000).unwrap();
    s.finish_question(31).unwrap();
    s.select_approach(ApproachChoice::Preset(2), 1000, 32)
        .unwrap();
    s.add_tokens(900).unwrap();
    s.record_paste_flag(300, 33).unwrap();
    s.finish_question(34).unwrap();
    let map = BTreeMap::from([
        (0, vec![Best]),
        (1, vec![Workable]),
        (2, vec![Best, Weak]),
        (3, vec![Workable]),
        (4, vec![Workable]),
        (5, vec![Best]),
        (6, vec![Weak]),
        (8, vec![Best]),
        (9, vec![Workable]),
    ]);
    let grades = [
        graded(0, 9.0, "optimal and clean", 0.3),
        graded(1, 4.0, "correct but wasteful", -0.2),
        GradeOutcome::NotScored { slot_index: 2 },
        graded(4, 6.5, "steady solution", 0.1),
        graded(5, 8.0, "efficient medium", 0.2),
        graded(6, 5.0, "barely solved", -0.1),
        graded(9, 7.0, "good hard solution", 0.2),
    ];
    let b = final_rating(&s, &map, &grades).unwrap();
    close(b.rating, 6.8);
    close(b.unclamped, 6.8);
    close(b.ai_correction_total, 0.5);
    assert_eq!(b.solved_count, 5);
    assert_eq!(b.finished_count, 8);
    assert_eq!(b.skipped_count, 2);
    assert_eq!(b.paste_flag_count, 2);
    assert_eq!(b.not_scored, vec![2, 8]);
    assert_eq!(b.per_question.len(), 10);
    let keys: Vec<(usize, ScoreComponent)> = b
        .lines
        .iter()
        .map(|l| (l.slot_index, l.component))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    let skip_line = b
        .lines
        .iter()
        .find(|l| l.slot_index == 3 && l.component == ScoreComponent::Tokens)
        .expect("skipped settlement line");
    close(skip_line.delta, -0.15);
    assert!(skip_line.detail.contains("skipped"), "{}", skip_line.detail);
}

#[test]
fn final_rating_revalidates_struct_literal_grades() {
    let mut s = started_easy();
    s.finish_question(10).unwrap();
    let literal = |grade: f64, rationale: &str, correction: f64| {
        GradeOutcome::Graded(QuestionGrade {
            slot_index: 0,
            grade,
            rationale: rationale.to_string(),
            correction,
        })
    };
    for bad in [
        literal(f64::NAN, "ok", 0.0),
        literal(8.0, "ok", f64::INFINITY),
        literal(42.0, "ok", 0.0),
        literal(8.0, "   ", 0.0),
    ] {
        assert!(
            matches!(
                final_rating(&s, &BTreeMap::new(), &[bad]),
                Err(ScoringError::InvalidGrade(_))
            ),
            "expected InvalidGrade"
        );
    }
    let ok = final_rating(&s, &BTreeMap::new(), &[literal(8.0, "solid", 0.1)]).unwrap();
    assert_eq!(ok.per_question[0].grade, Some(8.0));
}

#[test]
fn skipped_token_settlement_uses_acceptance_factor() {
    let mut s = started_easy_acceptance(Some(30));
    s.select_approach(ApproachChoice::Preset(0), 100, 10)
        .unwrap();
    s.add_tokens(200).unwrap();
    s.skip_question(11).unwrap();
    let b = final_rating(&s, &BTreeMap::new(), &[]).unwrap();
    let line = b
        .lines
        .iter()
        .find(|l| l.slot_index == 0 && l.component == ScoreComponent::Tokens)
        .expect("skipped settlement line");
    close(line.delta, -0.30 * 0.5 * 1.25);
    assert!(line.detail.contains("skipped"), "{}", line.detail);
    assert!(line.detail.contains("200/100"), "{}", line.detail);
    assert!(line.detail.contains("ac 30%"), "{}", line.detail);
}
