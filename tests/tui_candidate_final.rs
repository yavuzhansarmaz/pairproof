use std::collections::BTreeMap;

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{ApproachChoice, Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::scoring::{
    final_rating, ApproachGrade, ApproachQuality, GradeOutcome, QuestionGrade,
};
use pairproof::tui::candidate::{CandidateApp, CandidateConnectError, CandidateScreen};
use pairproof::tui::candidate_actions::{OtherGrader, UnavailableOtherGrader};
use pairproof::tui::candidate_final::{
    candidate_final, candidate_final_lines, CandidateFinal, CandidateFinalError,
};
use pairproof::tui::candidate_work::{
    CandidateWork, CodingRunner, RunnerError, UnavailableCodingRunner, WorkPhase,
};

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: format!("Title {slug}"),
        difficulty,
        acceptance_pct: None,
    }
}

fn qs(prefix: &str, n: usize, difficulty: Difficulty) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| q(&format!("{prefix}{i}"), difficulty))
        .collect()
}

fn started() -> Session {
    let mut session = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn completed() -> Session {
    let mut session = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    session
        .set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            1,
        )
        .expect("expert questions");
    session.mark_waiting_for_candidate(2).expect("waiting");
    session.candidate_connected(3).expect("connected");
    session.start(4).expect("start");
    for i in 0..10u64 {
        session.finish_question(10 + i).expect("finish");
    }
    assert_eq!(session.status(), SessionStatus::Scoring);
    session.mark_completed(100).expect("completed");
    session
}

fn graded(slot: usize, grade: f64, correction: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), correction)
            .expect("question grade"),
    )
}

struct OkVerifier;

impl pairproof::tui::candidate::ConnectionVerifier for OkVerifier {
    fn verify(&self, _c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        Ok(())
    }
}

fn solving_app(session: Session) -> CandidateApp {
    let value = ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        10_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    );
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&value.encode());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    app.attach_session(session);
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    app
}

#[test]
fn candidate_score_hidden_before_final_state() {
    let session = started();
    assert_eq!(session.status(), SessionStatus::InProgress);
    let error = candidate_final(&session, &BTreeMap::new(), &[]).expect_err("not final");
    assert_eq!(error, CandidateFinalError::NotFinal);
    let app = solving_app(session);
    assert_eq!(app.screen(), CandidateScreen::Solving);
    assert_ne!(app.screen(), CandidateScreen::Final);
}

#[test]
fn candidate_final_view_uses_real_final_breakdown() {
    let session = completed();
    let grades = vec![graded(0, 9.0, 0.2)];
    let expected = final_rating(&session, &BTreeMap::new(), &grades).expect("expected rating");
    let result = candidate_final(&session, &BTreeMap::new(), &grades).expect("candidate final");
    let CandidateFinal::Breakdown(view) = &result else {
        panic!("expected breakdown, got {result:?}");
    };
    assert_eq!(view.rating, expected.rating);
    assert_eq!(view.solved_count, expected.solved_count);
    assert_eq!(view.finished_count, expected.finished_count);
    let lines = candidate_final_lines(&result);
    let joined = lines.join("\n");
    assert!(joined.contains("Final breakdown"), "{joined}");
    assert!(
        joined.contains(&format!("final rating: {:.2}", expected.rating)),
        "{joined}"
    );
    assert!(joined.contains("grade 9.0"), "{joined}");
    assert!(joined.contains("solved:"), "{joined}");
    assert!(joined.contains("finished:"), "{joined}");
    assert!(joined.contains("skipped:"), "{joined}");
    assert!(joined.contains("paste flags:"), "{joined}");
}

#[test]
fn candidate_final_view_does_not_fabricate_report_or_artifacts() {
    let session = completed();
    let grades = vec![graded(0, 9.0, 0.2)];
    let result = candidate_final(&session, &BTreeMap::new(), &grades).expect("candidate final");
    let CandidateFinal::Breakdown(view) = &result else {
        panic!("expected breakdown, got {result:?}");
    };
    assert!(
        !view.artefact_note.contains("Phase"),
        "{}",
        view.artefact_note
    );
    let joined = candidate_final_lines(&result).join("\n");
    assert!(!joined.contains("Phase"), "{joined}");
    assert!(!joined.contains("report.md"), "{joined}");
    assert!(
        !joined.to_lowercase().contains("report generated"),
        "{joined}"
    );
    let mut failed = started();
    failed
        .mark_technical_failure(50)
        .expect("technical failure");
    assert_eq!(failed.status(), SessionStatus::TechnicalFailure);
    let result = candidate_final(&failed, &BTreeMap::new(), &[]).expect("technical final");
    assert_eq!(result, CandidateFinal::TechnicalFailure);
    let lines = candidate_final_lines(&result);
    assert_eq!(
        lines,
        vec![
            "the session ended as a technical failure.".to_string(),
            "no evaluation was produced; the interview can be rescheduled.".to_string(),
        ]
    );
    let joined = lines.join("\n");
    assert!(!joined.contains("Final rating"), "{joined}");
    assert!(!joined.chars().any(|c| c.is_ascii_digit()), "{joined}");
}

struct FakeGrader {
    calls: usize,
    last_statement: String,
    last_other: String,
}

impl OtherGrader for FakeGrader {
    fn grade(
        &mut self,
        question_statement: &str,
        other_text: &str,
    ) -> Result<ApproachGrade, RunnerError> {
        self.calls += 1;
        self.last_statement = question_statement.to_string();
        self.last_other = other_text.to_string();
        Ok(
            ApproachGrade::new(ApproachQuality::Workable, 1234, "plausible plan".into())
                .expect("approach grade"),
        )
    }
}

struct OkCodingRunner {
    calls: usize,
}

impl CodingRunner for OkCodingRunner {
    fn run_step(&mut self, _input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        self.calls += 1;
        Ok(CodingStepOutput {
            code: "print('fixed')".into(),
            ready: true,
            summary: "done".into(),
        })
    }
}

#[test]
fn other_grading_unavailable_records_no_selection() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_other_text("   ");
    let mut grader = UnavailableOtherGrader;
    let mut runner = UnavailableCodingRunner;
    let error = work
        .select_other_graded(&mut session, &mut grader, &mut runner, 10)
        .expect_err("blank other");
    assert!(format!("{error}").contains("empty other text"), "{error}");
    assert_eq!(session.current_slot().expect("slot").approach, None);
    work.set_other_text("my own plan");
    work.select_other_graded(&mut session, &mut grader, &mut runner, 11)
        .expect("unavailable is not an error");
    assert_eq!(session.current_slot().expect("slot").approach, None);
    assert_eq!(
        session.current_slot().expect("slot").locked_expected_tokens,
        None
    );
    assert_eq!(work.phase(), WorkPhase::RunnerUnavailable);
    assert_eq!(work.other_text(), "my own plan");
}

#[test]
fn other_grading_with_fake_grade_selects() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_statement("e0", "Find two numbers that sum to target.");
    work.set_other_text("  sliding window  ");
    let mut grader = FakeGrader {
        calls: 0,
        last_statement: String::new(),
        last_other: String::new(),
    };
    let mut runner = OkCodingRunner { calls: 0 };
    work.select_other_graded(&mut session, &mut grader, &mut runner, 10)
        .expect("graded selection");
    assert_eq!(grader.calls, 1);
    assert_eq!(
        grader.last_statement,
        "Find two numbers that sum to target."
    );
    assert!(grader.last_other.contains("sliding window"));
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Other("sliding window".into()))
    );
    assert_eq!(
        session.current_slot().expect("slot").locked_expected_tokens,
        Some(1234)
    );
    assert_eq!(work.other_text(), "");
    assert_eq!(work.phase(), WorkPhase::CodingReady);
    assert_eq!(runner.calls, 1);
}
