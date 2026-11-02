use pairproof::core::{
    ApproachChoice, CoreError, EventKind, Language, QuestionRef, QuestionStatus, Session,
    TestSummary,
};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::scoring::{ApproachOption, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_work::*;

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

fn new_session() -> Session {
    let easy = qs("e", 5, Difficulty::Easy);
    Session::new("s1".into(), Language::Python, easy, 0).expect("session")
}

fn started() -> Session {
    let mut session = new_session();
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn at_expert() -> Session {
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
    for i in 0..5u64 {
        session.finish_question(10 + i).expect("finish easy");
    }
    session
}

fn approach_set() -> ApproachSet {
    ApproachSet::new(vec![
        ApproachOption {
            title: "Hash table lookup".into(),
            summary: "Store seen values in a map".into(),
            quality: ApproachQuality::Workable,
            expected_tokens: 1111,
        },
        ApproachOption {
            title: "Sorting scan".into(),
            summary: "Sort then scan neighbours".into(),
            quality: ApproachQuality::Weak,
            expected_tokens: 2222,
        },
        ApproachOption {
            title: "Two pointers".into(),
            summary: "Move pointers from both ends".into(),
            quality: ApproachQuality::Best,
            expected_tokens: 3333,
        },
        ApproachOption {
            title: "Binary search".into(),
            summary: "Search halves of the range".into(),
            quality: ApproachQuality::Workable,
            expected_tokens: 4444,
        },
    ])
    .expect("approach set")
}

#[derive(Default)]
struct OkCodingRunner {
    calls: usize,
    last_input: Option<CodingStepInput>,
}

impl CodingRunner for OkCodingRunner {
    fn run_step(&mut self, input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        self.calls += 1;
        self.last_input = Some(input.clone());
        Ok(CodingStepOutput {
            code: "print('fixed')".into(),
            ready: true,
            summary: "done".into(),
        })
    }
}

struct FailedCodingRunner;

impl CodingRunner for FailedCodingRunner {
    fn run_step(&mut self, _input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        Err(RunnerError::Failed)
    }
}

struct CountingTestRunner {
    calls: usize,
    result: (u32, u32),
}

impl TestRunner for CountingTestRunner {
    fn run_tests(&mut self, _code: &str, _language: Language) -> Result<(u32, u32), RunnerError> {
        self.calls += 1;
        Ok(self.result)
    }
}

struct FailedTestRunner;

impl TestRunner for FailedTestRunner {
    fn run_tests(&mut self, _code: &str, _language: Language) -> Result<(u32, u32), RunnerError> {
        Err(RunnerError::Failed)
    }
}

fn has_event(session: &Session, kind: EventKind) -> bool {
    session.events().iter().any(|event| event.kind == kind)
}

#[test]
fn continue_dispatches_real_core_action() {
    let mut session = started();
    let mut work = CandidateWork::new();
    let mut runner = OkCodingRunner::default();
    work.continue_work(&mut session, &mut runner, 10)
        .expect("continue");
    assert!(has_event(&session, EventKind::Continued { index: 0 }));
    assert_eq!(runner.calls, 1);
    assert_eq!(work.phase(), WorkPhase::CodingReady);
    let mut failed_session = started();
    let mut failed_work = CandidateWork::new();
    failed_work
        .continue_work(&mut failed_session, &mut FailedCodingRunner, 10)
        .expect("core action stands");
    assert!(has_event(
        &failed_session,
        EventKind::Continued { index: 0 }
    ));
    assert_eq!(failed_work.phase(), WorkPhase::RunnerFailed);
    assert_eq!(failed_work.current_code(), "");
}

#[test]
fn change_approach_clears_current_selection_as_core_requires() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_approaches(approach_set());
    let mut runner = OkCodingRunner::default();
    work.select_preset(&mut session, 0, &mut runner, 10)
        .expect("select");
    work.change_approach(&mut session, 11).expect("change");
    let slot = session.current_slot().expect("slot");
    assert_eq!(slot.approach, None);
    assert_eq!(slot.locked_expected_tokens, None);
    assert!(has_event(&session, EventKind::ApproachChanged { index: 0 }));
    assert_eq!(work.current_code(), "print('fixed')");
    assert_eq!(work.phase(), WorkPhase::Idle);
    assert_eq!(work_view(&session, &work).approaches.len(), 4);
    work.select_preset(&mut session, 2, &mut runner, 12)
        .expect("reselect");
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Preset(2))
    );
}

#[test]
fn request_tests_shows_only_real_test_outcome() {
    let mut session = started();
    let mut work = CandidateWork::new();
    assert!(work_lines(&session, &work)
        .join("\n")
        .contains("no test run yet"));
    let mut runner = CountingTestRunner {
        calls: 0,
        result: (3, 1),
    };
    work.request_tests(&mut session, &mut runner, 10)
        .expect("tests");
    assert_eq!(runner.calls, 1);
    assert!(has_event(&session, EventKind::TestsRequested { index: 0 }));
    assert!(has_event(
        &session,
        EventKind::TestRunCompleted {
            index: 0,
            passed: 3,
            failed: 1
        }
    ));
    assert_eq!(
        session.current_slot().expect("slot").latest_test,
        Some(TestSummary {
            passed: 3,
            failed: 1,
            at_ms: 10
        })
    );
    assert_eq!(
        work_view(&session, &work).latest_test.as_deref(),
        Some("3 passed, 1 failed")
    );
    let mut unavailable_session = started();
    let mut unavailable_work = CandidateWork::new();
    let mut unavailable = UnavailableTestRunner;
    unavailable_work
        .request_tests(&mut unavailable_session, &mut unavailable, 10)
        .expect("core request stands");
    assert!(has_event(
        &unavailable_session,
        EventKind::TestsRequested { index: 0 }
    ));
    assert!(!unavailable_session
        .events()
        .iter()
        .any(|event| matches!(event.kind, EventKind::TestRunCompleted { .. })));
    assert_eq!(
        unavailable_session
            .current_slot()
            .expect("slot")
            .latest_test,
        None
    );
    let view = work_view(&unavailable_session, &unavailable_work);
    assert!(view.tests_unavailable);
    let lines = work_lines(&unavailable_session, &unavailable_work).join("\n");
    assert!(lines.contains("tests are not available yet"), "{lines}");
    assert!(lines.contains("no test run yet"), "{lines}");
    let mut failed_session = started();
    let mut failed_work = CandidateWork::new();
    failed_work
        .request_tests(&mut failed_session, &mut FailedTestRunner, 10)
        .expect("core request stands");
    assert_eq!(
        failed_session.current_slot().expect("slot").latest_test,
        None
    );
    assert!(!failed_session
        .events()
        .iter()
        .any(|event| matches!(event.kind, EventKind::TestRunCompleted { .. })));
}

#[test]
fn finish_question_displays_latest_recorded_test_summary() {
    let mut session = started();
    let mut work = CandidateWork::new();
    let mut runner = CountingTestRunner {
        calls: 0,
        result: (4, 2),
    };
    work.request_tests(&mut session, &mut runner, 10)
        .expect("tests");
    let preview = work.finish_preview(&session);
    assert_eq!(
        preview,
        Some(TestSummary {
            passed: 4,
            failed: 2,
            at_ms: 10
        })
    );
    assert_eq!(finish_preview(&session), preview);
    let returned = work.finish_question(&mut session, 20).expect("finish");
    assert_eq!(returned, preview);
    assert_eq!(
        session.slot(0).expect("slot").status,
        QuestionStatus::Finished
    );
    assert_eq!(session.slot(0).expect("slot").latest_test, preview);
    assert_eq!(session.current_index(), Some(1));
    assert_eq!(work.finish_preview(&session), None);
}

#[test]
fn skip_dispatches_core_action() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.skip_question(&mut session, 10).expect("skip");
    assert!(has_event(&session, EventKind::QuestionSkipped { index: 0 }));
    assert_eq!(
        session.slot(0).expect("slot").status,
        QuestionStatus::Skipped
    );
    assert_eq!(session.current_index(), Some(1));
}

#[test]
fn candidate_cannot_resolve_expert_only_requestion() {
    let mut session = at_expert();
    let mut work = CandidateWork::new();
    work.set_approaches(approach_set());
    assert!(can_request_requestion(&session));
    assert!(work.can_request_requestion(&session));
    work.request_requestion(&mut session, 20).expect("request");
    assert!(has_event(
        &session,
        EventKind::ReQuestionRequested { index: 5 }
    ));
    assert!(!work.can_request_requestion(&session));
    let lines = work_lines(&session, &work).join("\n");
    assert!(
        lines.contains("re-question requested; waiting for the expert."),
        "{lines}"
    );
    let mut runner = OkCodingRunner::default();
    assert_eq!(
        work.continue_work(&mut session, &mut runner, 21),
        Err(CandidateWorkError::Core(CoreError::ReQuestionPending))
    );
    assert_eq!(
        work.select_preset(&mut session, 0, &mut runner, 21),
        Err(CandidateWorkError::Core(CoreError::ReQuestionPending))
    );
    assert_eq!(runner.calls, 0);
    let mut test_runner = CountingTestRunner {
        calls: 0,
        result: (1, 0),
    };
    assert_eq!(
        work.request_tests(&mut session, &mut test_runner, 21),
        Err(CandidateWorkError::Core(CoreError::ReQuestionPending))
    );
    assert_eq!(
        work.finish_question(&mut session, 21),
        Err(CandidateWorkError::Core(CoreError::ReQuestionPending))
    );
    assert!(session.current_slot().expect("slot").re_question_pending);
}

#[test]
fn duplicate_action_does_not_apply_twice() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_approaches(approach_set());
    let mut runner = OkCodingRunner::default();
    work.select_preset(&mut session, 0, &mut runner, 10)
        .expect("select");
    let code_before = work.current_code().to_string();
    assert_eq!(
        work.select_preset(&mut session, 1, &mut runner, 11),
        Err(CandidateWorkError::Core(CoreError::ApproachAlreadySelected))
    );
    assert_eq!(runner.calls, 1);
    assert_eq!(work.current_code(), code_before);
    assert_eq!(work.phase(), WorkPhase::CodingReady);
    assert_eq!(
        session
            .events()
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ApproachSelected { .. }))
            .count(),
        1
    );
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Preset(0))
    );
}
