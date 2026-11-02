use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::grading::QuestionScoringInput;
use pairproof::llm::MetaClient;
use pairproof::orchestration::consistency::{compare_grade_runs, run_consistency};
use pairproof::orchestration::finalize::{GradeFuture, MetaQuestionGrader, QuestionGrader};
use pairproof::scoring::{GradeOutcome, QuestionGrade};

fn q(slug: &str) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: slug.into(),
        difficulty: Difficulty::Easy,
        acceptance_pct: None,
    }
}

fn finished_session() -> Session {
    let easy = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut s = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    s.mark_waiting_for_candidate(1).expect("waiting");
    s.candidate_connected(2).expect("connected");
    s.start(3).expect("start");
    s.finish_question(10).expect("finish 0");
    s.skip_question(11).expect("skip 1");
    s.finish_question(12).expect("finish 2");
    s
}

fn graded(slot: usize, grade: f64, correction: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), correction).expect("grade"),
    )
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[derive(Default)]
struct ScriptedGrader {
    runs: Mutex<VecDeque<Vec<GradeOutcome>>>,
    seen: Mutex<Vec<Vec<QuestionScoringInput>>>,
    calls: AtomicUsize,
}

impl ScriptedGrader {
    fn scripted(runs: Vec<Vec<GradeOutcome>>) -> Self {
        Self {
            runs: Mutex::new(runs.into()),
            ..Default::default()
        }
    }
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl QuestionGrader for ScriptedGrader {
    fn grade_all<'a>(&'a self, inputs: &'a [QuestionScoringInput]) -> GradeFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().expect("seen lock").push(inputs.to_vec());
        let scripted = self.runs.lock().expect("runs lock").pop_front();
        let fallback: Vec<GradeOutcome> = inputs
            .iter()
            .map(|i| GradeOutcome::NotScored {
                slot_index: i.slot_index,
            })
            .collect();
        Box::pin(async move { scripted.unwrap_or(fallback) })
    }
}

#[tokio::test]
async fn scoring_consistency_compares_two_independent_runs() {
    let session = finished_session();
    let first = vec![graded(0, 8.0, 0.2), graded(2, 6.0, 0.1)];
    let second = vec![graded(0, 7.0, 0.2), graded(2, 9.0, 0.1)];
    let grader = ScriptedGrader::scripted(vec![first.clone(), second.clone()]);
    let report = run_consistency(
        &session,
        &BTreeMap::new(),
        &grader,
        &HashMap::new(),
        &HashMap::new(),
    )
    .await
    .expect("consistency");
    assert_eq!(grader.call_count(), 2);
    assert_eq!(report.first, first);
    assert_eq!(report.second, second);
    assert_ne!(report.first, report.second);
    assert_eq!(report.per_question.len(), 2);
    assert_eq!(report.per_question[0].slot_index, 0);
    assert_eq!(report.per_question[1].slot_index, 2);
}

#[test]
fn scoring_consistency_reports_per_question_deltas() {
    let session = finished_session();
    let report = compare_grade_runs(
        &session,
        &BTreeMap::new(),
        &[graded(0, 8.0, 0.0), graded(2, 6.0, 0.0)],
        &[graded(0, 7.0, 0.0), graded(2, 6.5, 0.0)],
    )
    .expect("compare");
    assert_eq!(report.per_question.len(), 2);
    let d0 = &report.per_question[0];
    assert_eq!(d0.slot_index, 0);
    assert_eq!(d0.first_grade, Some(8.0));
    assert_eq!(d0.second_grade, Some(7.0));
    assert_eq!(d0.grade_delta, Some(1.0));
    assert!(!d0.status_changed && !d0.not_scored_drift);
    let d1 = &report.per_question[1];
    assert_eq!(d1.slot_index, 2);
    assert_eq!(d1.grade_delta, Some(0.5));
    assert_eq!(report.max_grade_delta, Some(1.0));
    assert!(!report.any_status_drift);
}

#[test]
fn scoring_consistency_reports_final_rating_delta() {
    let session = finished_session();
    let report = compare_grade_runs(
        &session,
        &BTreeMap::new(),
        &[graded(0, 8.0, 0.2), graded(2, 6.0, 0.1)],
        &[graded(0, 8.0, 0.0), graded(2, 6.0, 0.0)],
    )
    .expect("compare");
    close(report.first_rating, 6.3);
    close(report.second_rating, 6.0);
    close(report.final_rating_delta, 0.3);
    close(report.first_breakdown.rating, report.first_rating);
    close(report.second_breakdown.rating, report.second_rating);
}

#[test]
fn scoring_consistency_detects_not_scored_drift() {
    let session = finished_session();
    let report = compare_grade_runs(
        &session,
        &BTreeMap::new(),
        &[graded(0, 8.0, 0.1), graded(2, 6.0, 0.1)],
        &[
            GradeOutcome::NotScored { slot_index: 0 },
            graded(2, 6.0, 0.1),
        ],
    )
    .expect("compare");
    let d0 = &report.per_question[0];
    assert_eq!(d0.first_grade, Some(8.0));
    assert_eq!(d0.second_grade, None);
    assert_eq!(d0.grade_delta, None);
    assert!(d0.status_changed);
    assert!(d0.not_scored_drift);
    assert!(!report.per_question[1].status_changed);
    assert!(!report.per_question[1].not_scored_drift);
    assert!(report.any_status_drift);
}

#[test]
fn scoring_consistency_handles_identical_results() {
    let session = finished_session();
    let run = vec![graded(0, 8.0, 0.2), graded(2, 6.0, 0.1)];
    let report = compare_grade_runs(&session, &BTreeMap::new(), &run, &run).expect("compare");
    assert_eq!(report.per_question.len(), 2);
    for d in &report.per_question {
        assert_eq!(d.grade_delta, Some(0.0));
        assert!(!d.status_changed && !d.not_scored_drift);
    }
    assert_eq!(report.max_grade_delta, Some(0.0));
    close(report.final_rating_delta, 0.0);
    close(report.first_rating, report.second_rating);
    assert!(!report.any_status_drift);
}

#[tokio::test]
async fn scoring_consistency_does_not_mutate_recorded_session() {
    let session = finished_session();
    let before = session.clone();
    let events_before = session.events().to_vec();
    let slots_before = session.slots().to_vec();
    let grader = ScriptedGrader::scripted(vec![
        vec![graded(0, 8.0, 0.1), graded(2, 6.0, 0.1)],
        vec![graded(0, 7.0, 0.1), graded(2, 6.0, 0.1)],
    ]);
    let statements = HashMap::from([("e0".to_string(), "stmt zero".to_string())]);
    let codes = HashMap::from([(0usize, "code zero".to_string())]);
    run_consistency(&session, &BTreeMap::new(), &grader, &statements, &codes)
        .await
        .expect("consistency");
    assert_eq!(session, before);
    assert_eq!(session.events(), events_before.as_slice());
    assert_eq!(session.slots(), slots_before.as_slice());
    let seen = grader.seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0], seen[1]);
    assert_eq!(seen[0].len(), 2);
    assert_eq!(seen[0][0].slot_index, 0);
    assert_eq!(seen[0][1].slot_index, 2);
}

#[test]
fn scoring_consistency_preserves_both_original_results() {
    let session = finished_session();
    let first = vec![graded(2, 6.0, 0.1), graded(0, 8.0, 0.2)];
    let second = vec![graded(0, 7.5, 0.0), graded(2, 5.0, 0.0)];
    let report = compare_grade_runs(&session, &BTreeMap::new(), &first, &second).expect("compare");
    assert_eq!(report.first, first);
    assert_eq!(report.second, second);
    let slots: Vec<usize> = report.per_question.iter().map(|d| d.slot_index).collect();
    assert_eq!(slots, vec![0, 2]);
}

#[tokio::test]
async fn scoring_consistency_handles_grading_failure() {
    let session = finished_session();
    let grader = ScriptedGrader::default();
    let report = run_consistency(
        &session,
        &BTreeMap::new(),
        &grader,
        &HashMap::new(),
        &HashMap::new(),
    )
    .await
    .expect("not-scored runs still compare");
    assert_eq!(grader.call_count(), 2);
    assert_eq!(report.per_question.len(), 2);
    for d in &report.per_question {
        assert_eq!(d.first_grade, None);
        assert_eq!(d.second_grade, None);
        assert_eq!(d.grade_delta, None);
        assert!(!d.not_scored_drift);
    }
    assert_eq!(report.max_grade_delta, None);
    assert!(!report.any_status_drift);
    let mismatched = compare_grade_runs(
        &session,
        &BTreeMap::new(),
        &[graded(0, 8.0, 0.1), graded(2, 6.0, 0.1)],
        &[graded(0, 8.0, 0.1)],
    )
    .expect("mismatched lengths compare");
    assert_eq!(mismatched.per_question.len(), 2);
    assert_eq!(mismatched.per_question[1].second_grade, None);
    assert!(mismatched.per_question[1].not_scored_drift);
    assert!(mismatched.any_status_drift);
    let empty =
        compare_grade_runs(&session, &BTreeMap::new(), &[], &[]).expect("empty runs compare");
    assert!(empty.per_question.is_empty());
    assert_eq!(empty.max_grade_delta, None);
}

#[tokio::test]
#[ignore = "live network smoke, requires MODEL_API_KEY"]
async fn live_scoring_consistency_grades_recorded_session_twice() {
    let client =
        MetaClient::from_env().expect("MODEL_API_KEY must be set for live consistency test");
    let grader = MetaQuestionGrader::new(client);
    let session = finished_session();
    let statements = HashMap::from([
        (
            "e0".to_string(),
            "Given nums, return two indices summing to target.".to_string(),
        ),
        ("e2".to_string(), "Reverse a string in place.".to_string()),
    ]);
    let codes = HashMap::from([
        (
            0usize,
            "def solve(nums, target):\n    return []".to_string(),
        ),
        (2usize, "def solve(s):\n    return s[::-1]".to_string()),
    ]);
    let report = run_consistency(&session, &BTreeMap::new(), &grader, &statements, &codes)
        .await
        .expect("live consistency");
    println!(
        "first_rating={} second_rating={} final_rating_delta={} max_grade_delta={:?}",
        report.first_rating,
        report.second_rating,
        report.final_rating_delta,
        report.max_grade_delta
    );
    for d in &report.per_question {
        println!(
            "slot={} first={:?} second={:?} delta={:?} drift={}",
            d.slot_index, d.first_grade, d.second_grade, d.grade_delta, d.not_scored_drift
        );
    }
}
