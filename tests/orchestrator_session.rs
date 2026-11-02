use pairproof::core::{CoreError, EventKind, Language, QuestionRef, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::{Clock, ManualClock, Orchestrator, OrchestratorError, SystemClock};

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
        acceptance_pct: None,
    }
}

fn qs(prefix: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{prefix}{i}"), d)).collect()
}

fn easy() -> Vec<QuestionRef> {
    qs("e", 5, Difficulty::Easy)
}

fn medium() -> Vec<QuestionRef> {
    qs("m", 3, Difficulty::Medium)
}

fn hard() -> Vec<QuestionRef> {
    qs("h", 2, Difficulty::Hard)
}

fn created() -> Orchestrator {
    Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create")
}

fn started() -> Orchestrator {
    let mut o = created();
    o.mark_waiting_for_candidate(1).expect("waiting");
    o.attach_candidate(2).expect("attach");
    o.start(3).expect("start");
    o
}

fn with_expert_started() -> Orchestrator {
    let mut o = created();
    o.set_expert_questions(medium(), hard(), 1).expect("expert");
    o.mark_waiting_for_candidate(2).expect("waiting");
    o.attach_candidate(3).expect("attach");
    o.start(4).expect("start");
    o
}

fn at_expert() -> Orchestrator {
    let mut o = with_expert_started();
    for i in 0..5u64 {
        o.finish_question(10 + i).expect("finish easy");
    }
    o
}

#[test]
fn orchestrator_creates_session_with_initial_easy_questions() {
    let o = created();
    assert_eq!(o.session_id(), "s1");
    assert_eq!(o.status(), SessionStatus::Setup);
    assert_eq!(o.session().slots().len(), 5);
    assert_eq!(o.current_index(), None);
    assert_eq!(o.current_question(), None);
    assert!(!o.is_connected());
    assert!(!o.is_terminal());
    assert_eq!(o.elapsed_ms(100), 0);
    assert_eq!(o.per_question_ms(100), None);
    assert_eq!(o.session().events().len(), 1);
    assert_eq!(o.session().events()[0].kind, EventKind::SessionCreated);
    for (i, slot) in o.session().slots().iter().enumerate() {
        assert_eq!(slot.index, i);
        assert_eq!(slot.question.title_slug, format!("e{i}"));
        assert_eq!(slot.question.difficulty, Difficulty::Easy);
    }
    // Failure paths: wrong count, wrong difficulty, duplicate, empty id.
    assert_eq!(
        Orchestrator::create(
            "s1".into(),
            Language::Python,
            qs("e", 4, Difficulty::Easy),
            0
        )
        .expect_err("count err"),
        OrchestratorError::Core(CoreError::InvalidQuestionCount {
            expected: 5,
            got: 4
        })
    );
    let mut wrong = easy();
    wrong[0] = q("e0", Difficulty::Medium);
    assert!(matches!(
        Orchestrator::create("s1".into(), Language::Python, wrong, 0),
        Err(OrchestratorError::Core(CoreError::WrongDifficulty { .. }))
    ));
    let mut dup = easy();
    dup[1] = q("e0", Difficulty::Easy);
    assert_eq!(
        Orchestrator::create("s1".into(), Language::Python, dup, 0).expect_err("dup err"),
        OrchestratorError::Core(CoreError::DuplicateSlug("e0".into()))
    );
    assert_eq!(
        Orchestrator::create("  ".into(), Language::Python, easy(), 0).expect_err("id err"),
        OrchestratorError::Core(CoreError::EmptySessionId)
    );
    // Clock projections are deterministic.
    let clock = ManualClock::new(10);
    assert_eq!(clock.now_ms(), 10);
    clock.advance(15);
    assert_eq!(clock.now_ms(), 25);
    clock.set(100);
    assert_eq!(clock.now_ms(), 100);
    assert!(SystemClock.now_ms() > 0);
}

#[test]
fn orchestrator_enforces_core_transition_order() {
    let mut o = created();
    // Start and attach before waiting are illegal.
    assert!(matches!(
        o.start(1),
        Err(OrchestratorError::Core(CoreError::InvalidState(_)))
    ));
    assert!(matches!(
        o.attach_candidate(1),
        Err(OrchestratorError::Core(CoreError::InvalidState(_)))
    ));
    assert_eq!(o.status(), SessionStatus::Setup);
    o.mark_waiting_for_candidate(1).expect("waiting");
    assert_eq!(o.status(), SessionStatus::WaitingForCandidate);
    // Repeat waiting is illegal.
    assert!(o.mark_waiting_for_candidate(2).is_err());
    // Start without a candidate is rejected with the core error.
    assert_eq!(
        o.start(2).expect_err("start err"),
        OrchestratorError::Core(CoreError::CandidateNotConnected)
    );
    o.attach_candidate(2).expect("attach");
    assert!(o.is_connected());
    o.start(3).expect("start");
    assert_eq!(o.status(), SessionStatus::InProgress);
    assert_eq!(o.current_index(), Some(0));
    assert_eq!(
        o.current_question().map(|q| q.title_slug.as_str()),
        Some("e0")
    );
    // Second start and second waiting are illegal.
    assert!(o.start(4).is_err());
    assert!(o.mark_waiting_for_candidate(4).is_err());
    // Projections derive from the started slot.
    assert_eq!(o.elapsed_ms(3), 0);
    assert_eq!(o.elapsed_ms(103), 100);
    assert_eq!(o.per_question_ms(103), Some(100));
    assert_eq!(o.per_question_ms(2), Some(0));
}

#[test]
fn orchestrator_rejects_duplicate_candidate_attach() {
    let mut o = created();
    o.mark_waiting_for_candidate(1).expect("waiting");
    o.attach_candidate(2).expect("attach");
    let n = o.session().events().len();
    assert_eq!(
        o.attach_candidate(3).expect_err("dup attach"),
        OrchestratorError::AlreadyAttached
    );
    assert_eq!(o.session().events().len(), n);
    assert_eq!(
        o.session()
            .events()
            .iter()
            .filter(|e| e.kind == EventKind::CandidateConnected)
            .count(),
        1
    );
    assert!(o.is_connected());
    // Reconnect while still connected is also AlreadyAttached.
    assert_eq!(
        o.reconnect_candidate(3).expect_err("dup reconnect"),
        OrchestratorError::AlreadyAttached
    );
    assert_eq!(o.session().events().len(), n);
    // Disconnect then reconnect resumes without a second Connected event.
    o.start(4).expect("start");
    o.disconnect_candidate(10).expect("disconnect");
    assert!(!o.is_connected());
    assert_eq!(
        o.disconnect_candidate(11).expect_err("dup disconnect"),
        OrchestratorError::Core(CoreError::CandidateNotConnected)
    );
    o.reconnect_candidate(60).expect("reconnect");
    assert!(o.is_connected());
    assert_eq!(o.current_index(), Some(0));
    assert!(o
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateReconnected { downtime_ms: 50 }));
    assert_eq!(
        o.session()
            .events()
            .iter()
            .filter(|e| e.kind == EventKind::CandidateConnected)
            .count(),
        1
    );
    // Terminal sessions surface the core Terminal error, not AlreadyAttached.
    o.end_by_expert(70).expect("end");
    assert_eq!(
        o.attach_candidate(71).expect_err("terminal attach"),
        OrchestratorError::Core(CoreError::Terminal)
    );
}

#[test]
fn orchestrator_preserves_waiting_for_expert_state() {
    let mut o = started();
    for i in 0..5u64 {
        o.finish_question(10 + i).expect("finish");
    }
    assert_eq!(o.status(), SessionStatus::WaitingForExpert);
    assert_eq!(o.current_index(), None);
    assert_eq!(o.current_question(), None);
    assert_eq!(o.per_question_ms(100), None);
    // Elapsed is measured from the earliest started question.
    assert_eq!(o.elapsed_ms(103), 100);
    // Expert questions resume the interview on index 5.
    o.set_expert_questions(medium(), hard(), 20)
        .expect("expert set");
    assert_eq!(o.status(), SessionStatus::InProgress);
    assert_eq!(o.current_index(), Some(5));
    assert_eq!(
        o.current_question().map(|q| q.title_slug.as_str()),
        Some("m0")
    );
    assert_eq!(o.session().slots().len(), 10);
    // Setting expert questions twice is rejected by the core.
    assert_eq!(
        o.set_expert_questions(medium(), hard(), 21)
            .expect_err("dup expert"),
        OrchestratorError::Core(CoreError::ExpertAlreadySet)
    );
    // Extra question requires the expert set and is allowed at most once.
    o.add_extra_question(q("x0", Difficulty::Easy), 22)
        .expect("extra");
    assert_eq!(o.session().slots().len(), 11);
    assert_eq!(
        o.add_extra_question(q("x1", Difficulty::Easy), 23)
            .expect_err("dup extra"),
        OrchestratorError::Core(CoreError::ExtraAlreadyAdded)
    );
    // Without an expert set the extra question is rejected.
    let mut bare = started();
    assert_eq!(
        bare.add_extra_question(q("x0", Difficulty::Easy), 5)
            .expect_err("no expert"),
        OrchestratorError::Core(CoreError::ExpertNotSet)
    );
}

#[test]
fn orchestrator_maps_expert_end_to_terminal_state() {
    let mut o = started();
    o.end_by_expert(9).expect("end");
    assert_eq!(o.status(), SessionStatus::EndedByExpert);
    assert!(o.is_terminal());
    assert!(o
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::ExpertEnded));
    // Every further mutation surfaces the core Terminal error.
    assert_eq!(
        o.continue_work(10).expect_err("terminal continue"),
        OrchestratorError::Core(CoreError::Terminal)
    );
    assert_eq!(
        o.end_by_expert(10).expect_err("terminal end"),
        OrchestratorError::Core(CoreError::Terminal)
    );
    assert_eq!(
        o.mark_completed(10).expect_err("terminal complete"),
        OrchestratorError::Core(CoreError::Terminal)
    );
    assert_eq!(
        o.finish_question(10).expect_err("terminal finish"),
        OrchestratorError::Core(CoreError::Terminal)
    );
    // Technical failure is also terminal.
    let mut failed = started();
    failed.mark_technical_failure(9).expect("failure");
    assert_eq!(failed.status(), SessionStatus::TechnicalFailure);
    assert!(failed.is_terminal());
    assert_eq!(
        failed.mark_completed(10).expect_err("terminal"),
        OrchestratorError::Core(CoreError::Terminal)
    );
    // Completing from Scoring reaches the Completed terminal state.
    let mut done = at_expert();
    for i in 0..5u64 {
        done.finish_question(20 + i).expect("finish expert");
    }
    assert_eq!(done.status(), SessionStatus::Scoring);
    assert!(!done.is_terminal());
    done.mark_completed(30).expect("complete");
    assert_eq!(done.status(), SessionStatus::Completed);
    assert!(done.is_terminal());
    // Completing before Scoring is an illegal transition.
    let mut early = started();
    assert!(matches!(
        early.mark_completed(5),
        Err(OrchestratorError::Core(CoreError::InvalidState(_)))
    ));
}

#[test]
fn orchestrator_keeps_core_independent_of_tui() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/orchestration");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("orchestration dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "expected orchestration sources");
    let needle = ["crate", "::", "tui"].concat();
    let bare = ["tui", "::"].concat();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("read orchestration source");
        assert!(
            !src.contains(&needle),
            "{} must not reference the terminal UI layer",
            path.display()
        );
        assert!(
            !src.contains(&bare),
            "{} must not reference the terminal UI layer",
            path.display()
        );
    }
    // The crate root wires orchestration without a UI dependency cycle.
    let lib = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
    )
    .expect("read lib.rs");
    assert!(lib.contains("pub mod orchestration;"));
}
