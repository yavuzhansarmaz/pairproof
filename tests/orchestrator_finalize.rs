use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use pairproof::core::{Language, QuestionRef, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::grading::QuestionScoringInput;
use pairproof::orchestration::finalize::{
    both_roles_final, compute_final, final_visible_to_candidate, grade_session, session_elapsed_ms,
    FinalizeError, GradeFuture, QuestionGrader,
};
use pairproof::orchestration::Orchestrator;
use pairproof::scoring::{final_rating, ApproachQuality, GradeOutcome, QuestionGrade};

fn q(slug: &str, d: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: slug.into(),
        difficulty: d,
        acceptance_pct: None,
    }
}
fn qs(p: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{p}{i}"), d)).collect()
}
fn created(at: u64) -> Orchestrator {
    Orchestrator::create(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        at,
    )
    .expect("create")
}
fn started() -> Orchestrator {
    let mut o = created(100);
    o.mark_waiting_for_candidate(200).expect("waiting");
    o.attach_candidate(300).expect("attach");
    o.start(1000).expect("start");
    o
}
fn finished_skipped_finished() -> Orchestrator {
    let mut o = started();
    o.finish_question(2500).expect("finish 0");
    o.skip_question(4000).expect("skip 1");
    o.finish_question(5500).expect("finish 2");
    o
}
fn graded(slot: usize, grade: f64, correction: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), correction).expect("grade"),
    )
}

#[derive(Default)]
struct FakeGrader {
    requested: Mutex<Vec<(usize, String, String)>>,
    not_scored: Vec<usize>,
    grade: f64,
    correction: f64,
}
impl QuestionGrader for FakeGrader {
    fn grade_all<'a>(&'a self, inputs: &'a [QuestionScoringInput]) -> GradeFuture<'a> {
        let recorded: Vec<(usize, String, String)> = inputs
            .iter()
            .map(|i| {
                (
                    i.slot_index,
                    i.question_statement.clone(),
                    i.solution_code.clone(),
                )
            })
            .collect();
        let slots: Vec<usize> = inputs.iter().map(|i| i.slot_index).collect();
        let not_scored = self.not_scored.clone();
        let (grade, correction) = (self.grade, self.correction);
        Box::pin(async move {
            *self.requested.lock().expect("lock") = recorded;
            let _ = slots;
            inputs
                .iter()
                .map(|i| {
                    if not_scored.contains(&i.slot_index) {
                        GradeOutcome::NotScored {
                            slot_index: i.slot_index,
                        }
                    } else {
                        graded(i.slot_index, grade, correction)
                    }
                })
                .collect()
        })
    }
}

#[test]
fn session_elapsed_time_uses_real_lifecycle_boundaries() {
    let mut running = started();
    assert_eq!(session_elapsed_ms(running.session(), 2000), 1900);
    running.end_by_expert(5000).expect("end");
    // Terminal boundary wins over the supplied "now".
    assert_eq!(session_elapsed_ms(running.session(), 99_999), 4900);
    assert_eq!(session_elapsed_ms(created(700).session(), 700), 0);
}

#[test]
fn per_question_time_closes_once_per_question() {
    let o = finished_skipped_finished();
    let b = compute_final(&o, &BTreeMap::new(), &[]).expect("final");
    assert_eq!(b.per_question[0].time_ms, Some(1500));
    assert_eq!(b.per_question[1].time_ms, Some(1500));
    assert_eq!(b.per_question[2].time_ms, Some(1500));
    // Active and pending slots have no closed time.
    assert_eq!(b.per_question[3].time_ms, None);
    assert_eq!(b.per_question[4].time_ms, None);
    assert_eq!(b.total_time_ms, 4500);
    let again = compute_final(&o, &BTreeMap::new(), &[]).expect("final again");
    assert_eq!(again.per_question, b.per_question);
}

#[tokio::test]
async fn skipped_questions_are_not_sent_for_grading() {
    let o = finished_skipped_finished();
    let grader = FakeGrader {
        grade: 7.0,
        correction: 0.1,
        ..Default::default()
    };
    let statements = HashMap::from([("e0".to_string(), "statement zero".to_string())]);
    let codes = HashMap::from([(0usize, "code zero".to_string())]);
    let out = grade_session(&o, &grader, &statements, &codes)
        .await
        .expect("grade");
    assert_eq!(out.len(), 2);
    let requested = grader.requested.lock().expect("lock").clone();
    // Only finished slots 0 and 2; skipped slot 1 is never sent.
    assert_eq!(
        requested,
        vec![
            (
                0usize,
                "statement zero".to_string(),
                "code zero".to_string()
            ),
            (2usize, String::new(), String::new()),
        ]
    );
}

#[tokio::test]
async fn grading_retries_only_failed_question() {
    // grade_questions retries internally per question; at this level a
    // failed slot becomes NotScored and never blocks a peer slot.
    let o = finished_skipped_finished();
    let grader = FakeGrader {
        not_scored: vec![0],
        grade: 8.0,
        correction: 0.2,
        ..Default::default()
    };
    let out = grade_session(&o, &grader, &HashMap::new(), &HashMap::new())
        .await
        .expect("grade");
    assert_eq!(out[0], GradeOutcome::NotScored { slot_index: 0 });
    assert_eq!(out[1], graded(2, 8.0, 0.2));
    let slots: Vec<usize> = grader
        .requested
        .lock()
        .expect("lock")
        .iter()
        .map(|r| r.0)
        .collect();
    assert_eq!(slots, vec![0, 2]);
}

#[tokio::test]
async fn grading_exhaustion_becomes_not_scored() {
    let o = finished_skipped_finished();
    let grader = FakeGrader {
        not_scored: vec![0, 2],
        ..Default::default()
    };
    let out = grade_session(&o, &grader, &HashMap::new(), &HashMap::new())
        .await
        .expect("grade");
    assert_eq!(
        out,
        vec![
            GradeOutcome::NotScored { slot_index: 0 },
            GradeOutcome::NotScored { slot_index: 2 },
        ]
    );
    let b = compute_final(&o, &BTreeMap::new(), &out).expect("final");
    assert_eq!(b.not_scored, vec![0, 2]);
}

#[test]
fn not_scored_question_contributes_zero_correction() {
    let o = finished_skipped_finished();
    let grades = vec![
        GradeOutcome::NotScored { slot_index: 0 },
        graded(2, 9.0, 0.4),
    ];
    let b = compute_final(&o, &BTreeMap::new(), &grades).expect("final");
    assert_eq!(b.per_question[0].grade, None);
    assert_eq!(b.per_question[0].correction, 0.0);
    assert!((b.ai_correction_total - 0.4).abs() < 1e-9);
    assert_eq!(b.not_scored, vec![0]);
}

#[test]
fn final_score_uses_phase3_final_rating() {
    let o = finished_skipped_finished();
    let qualities = BTreeMap::from([
        (0usize, vec![ApproachQuality::Best]),
        (2usize, vec![ApproachQuality::Weak]),
    ]);
    let grades = vec![graded(0, 9.0, 0.3), graded(2, 4.0, -0.1)];
    let via = compute_final(&o, &qualities, &grades).expect("compute");
    let direct = final_rating(o.session(), &qualities, &grades).expect("direct");
    assert_eq!(via, direct);
    // A grade for the skipped slot is rejected by Phase 3 validation.
    let err = compute_final(&o, &qualities, &[graded(1, 5.0, 0.1)]).expect_err("skipped");
    assert!(matches!(err, FinalizeError::Scoring(_)), "{err:?}");
}

#[test]
fn final_breakdown_is_identical_for_both_roles() {
    let mut o = finished_skipped_finished();
    o.end_by_expert(6000).expect("end");
    assert!(final_visible_to_candidate(o.status()));
    let grades = vec![graded(0, 8.0, 0.2), graded(2, 6.0, 0.1)];
    let (expert, candidate) = both_roles_final(&o, &BTreeMap::new(), &grades).expect("both");
    assert_eq!(candidate, Some(expert));
}

#[test]
fn candidate_final_score_is_hidden_until_terminal_final_state() {
    let grades = vec![graded(0, 8.0, 0.2), graded(2, 6.0, 0.1)];
    let running = finished_skipped_finished();
    assert!(!final_visible_to_candidate(running.status()));
    let (_, c) = both_roles_final(&running, &BTreeMap::new(), &grades).expect("running");
    assert_eq!(c, None);
    let mut failed = started();
    failed.mark_technical_failure(2000).expect("tf");
    assert_eq!(failed.status(), SessionStatus::TechnicalFailure);
    assert!(!final_visible_to_candidate(failed.status()));
    let (_, c) = both_roles_final(&failed, &BTreeMap::new(), &[]).expect("tf final");
    assert_eq!(c, None);
    assert!(final_visible_to_candidate(SessionStatus::Completed));
    assert!(final_visible_to_candidate(SessionStatus::EndedByExpert));
    assert!(!final_visible_to_candidate(SessionStatus::Scoring));
    assert!(!final_visible_to_candidate(SessionStatus::InProgress));
}
