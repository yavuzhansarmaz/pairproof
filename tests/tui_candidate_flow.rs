use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{EventKind, Language, QuestionRef, QuestionStatus, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::chat::ChatState;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::scoring::{
    final_rating, ApproachOption, ApproachQuality, ApproachSet, GradeOutcome, QuestionGrade,
};
use pairproof::sync::detect_bulk_insert;
use pairproof::tui::candidate::{CandidateApp, CandidateConnectError, CandidateScreen};
use pairproof::tui::candidate_chat::{ChatController, ChatService, ChatServiceError};
use pairproof::tui::candidate_final::{candidate_final, CandidateFinal};
use pairproof::tui::candidate_run::{
    candidate_screen_lines, handle_candidate_key, handle_candidate_paste, CandidateFocus,
    CandidateRunState, CandidateTtyGuard,
};
use pairproof::tui::candidate_sync::solution_snapshot;
use pairproof::tui::candidate_work::{
    work_lines, CandidateWork, CodingRunner, RunnerError, TestRunner,
};
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}
struct OkVerifier;
impl pairproof::tui::candidate::ConnectionVerifier for OkVerifier {
    fn verify(&self, _c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        Ok(())
    }
}
fn encoded_connection() -> String {
    ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    )
    .encode()
}
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
fn begun(expert: bool) -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    if expert {
        s.set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            1,
        )
        .expect("expert questions");
    }
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}
fn started() -> Session {
    begun(false)
}
fn expert_started() -> Session {
    begun(true)
}
fn solving_state(session: Session) -> CandidateRunState {
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&encoded_connection());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    app.attach_session(session);
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    CandidateRunState::with_parts(app, CandidateWork::new(), ChatController::default())
}
fn opt(t: &str, s: &str, q: ApproachQuality, n: u64) -> ApproachOption {
    ApproachOption {
        title: t.into(),
        summary: s.into(),
        quality: q,
        expected_tokens: n,
    }
}
fn approach_set() -> ApproachSet {
    let w = ApproachQuality::Workable;
    ApproachSet::new(vec![
        opt("Hash table lookup", "Store seen values", w, 1111),
        opt(
            "Sorting scan",
            "Sort then scan",
            ApproachQuality::Weak,
            2222,
        ),
        opt(
            "Two pointers",
            "Move pointers inward",
            ApproachQuality::Best,
            3333,
        ),
        opt("Binary search", "Search halves", w, 4444),
    ])
    .expect("approach set")
}
struct OkCodingRunner;
impl CodingRunner for OkCodingRunner {
    fn run_step(&mut self, _input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        Ok(CodingStepOutput {
            code: "print('fixed')".into(),
            ready: true,
            summary: "done".into(),
        })
    }
}
struct FixedTestRunner;
impl TestRunner for FixedTestRunner {
    fn run_tests(&mut self, _code: &str, _language: Language) -> Result<(u32, u32), RunnerError> {
        Ok((3, 1))
    }
}
struct FakeChatService;
impl ChatService for FakeChatService {
    fn complete(&mut self, _messages: &[Message]) -> Result<String, ChatServiceError> {
        Ok("Consider invariants.".into())
    }
}
fn has_event(session: &Session, kind: EventKind) -> bool {
    session.events().iter().any(|event| event.kind == kind)
}

#[test]
fn connect_q_types_instead_of_quitting() {
    let mut state = CandidateRunState::new();
    handle_candidate_key(&mut state, key(KeyCode::Char('q')));
    assert_eq!(state.app.screen(), CandidateScreen::Connect);
    assert_eq!(state.app.input_len(), 1);
}

#[test]
fn ctrl_c_quits_from_connect() {
    let mut state = CandidateRunState::new();
    handle_candidate_key(&mut state, ctrl(KeyCode::Char('c')));
    assert_eq!(state.app.screen(), CandidateScreen::Exited);
    assert_eq!(state.app.input_len(), 0);
}

#[test]
fn rules_esc_quits_q_does_not() {
    let app = CandidateApp::with_verifier(OkVerifier);
    let mut state =
        CandidateRunState::with_parts(app, CandidateWork::new(), ChatController::default());
    state.app.push_text(&encoded_connection());
    handle_candidate_key(&mut state, key(KeyCode::Enter));
    assert_eq!(state.app.screen(), CandidateScreen::Rules);
    handle_candidate_key(&mut state, key(KeyCode::Char('q')));
    assert_eq!(state.app.screen(), CandidateScreen::Rules);
    handle_candidate_key(&mut state, key(KeyCode::Esc));
    assert_eq!(state.app.screen(), CandidateScreen::Exited);
}

#[test]
fn solving_keys_dispatch_with_unavailable_runners_honestly() {
    let mut state = solving_state(started());
    assert_eq!(state.app.screen(), CandidateScreen::Solving);
    assert_eq!(state.focus, CandidateFocus::Work);
    handle_candidate_key(&mut state, key(KeyCode::Char('s')));
    {
        let session = state.app.session().expect("session");
        assert_eq!(
            session.slot(0).expect("slot").status,
            QuestionStatus::Skipped
        );
        assert_eq!(session.current_index(), Some(1));
    }
    handle_candidate_key(&mut state, key(KeyCode::Char('t')));
    {
        let session = state.app.session().expect("session");
        assert!(has_event(session, EventKind::TestsRequested { index: 1 }));
        assert!(!session
            .events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::TestRunCompleted { .. })));
        assert_eq!(session.current_slot().expect("slot").latest_test, None);
    }
    let lines = candidate_screen_lines(&state).join("\n");
    assert!(lines.contains("tests are not available yet"), "{lines}");
    for _ in 0..3 {
        handle_candidate_key(&mut state, key(KeyCode::Tab));
    }
    assert_eq!(state.focus, CandidateFocus::Chat);
    handle_candidate_key(&mut state, key(KeyCode::Char('h')));
    handle_candidate_key(&mut state, key(KeyCode::Char('i')));
    assert_eq!(state.chat.input_text(), "hi");
    handle_candidate_key(&mut state, key(KeyCode::Enter));
    assert!(state.chat.history().is_empty());
    let lines = candidate_screen_lines(&state).join("\n");
    assert!(lines.contains("chat is not available yet"), "{lines}");
    handle_candidate_paste(&mut state, "bulk pasted text");
    assert!(state.paste_notice);
    assert_eq!(state.chat.input_text(), "hi");
    let lines = candidate_screen_lines(&state).join("\n");
    assert!(
        lines.contains("pasting is blocked here; type instead."),
        "{lines}"
    );
}

#[test]
fn candidate_controller_happy_path_uses_deterministic_fakes() {
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&encoded_connection());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    // Drive the session through its own public API and the work/chat
    // controllers directly; CandidateApp only observes via attach/session.
    let mut session = expert_started();
    app.attach_session(session.clone());
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    assert!(app.can_solve());
    let mut work = CandidateWork::new();
    work.set_approaches(approach_set());
    work.set_statement("e0", "Find two numbers that sum to target.");
    let mut coding = OkCodingRunner;
    work.select_preset(&mut session, 0, &mut coding, 10)
        .expect("preset");
    assert_eq!(work.current_code(), "print('fixed')");
    work.continue_work(&mut session, &mut coding, 11)
        .expect("continue");
    let mut tests = FixedTestRunner;
    work.request_tests(&mut session, &mut tests, 12)
        .expect("tests");
    let latest = session.current_slot().expect("slot").latest_test;
    assert_eq!(latest.map(|t| (t.passed, t.failed)), Some((3, 1)));
    let mut chat = ChatController::new(FakeChatService);
    chat.type_text("What should I consider?");
    let statement = work.statement_for("e0").expect("statement").to_string();
    let chat_state = ChatState::from_session(&session, &statement, work.current_code());
    let reply = chat.submit(&mut session, &chat_state, 13).expect("chat");
    assert_eq!(reply, "Consider invariants.");
    assert_eq!(chat.history().len(), 2);
    let returned = work.finish_question(&mut session, 14).expect("finish");
    assert_eq!(returned.map(|t| (t.passed, t.failed)), Some((3, 1)));
    assert_eq!(
        session.slot(0).expect("slot").status,
        QuestionStatus::Finished
    );
    for i in 1..10u64 {
        if i == 1 {
            work.skip_question(&mut session, 20 + i).expect("skip");
        } else {
            work.finish_question(&mut session, 20 + i)
                .expect("finish rest");
        }
    }
    assert_eq!(session.slots().len(), 10);
    assert_eq!(session.status(), SessionStatus::Scoring);
    session.mark_completed(100).expect("completed");
    assert_eq!(session.status(), SessionStatus::Completed);
    app.attach_session(session.clone());
    assert_eq!(app.screen(), CandidateScreen::Final);
    let session = &session;
    let grades = vec![GradeOutcome::Graded(
        QuestionGrade::new(0, 9.0, "rationale 0".into(), 0.2).expect("grade"),
    )];
    let expected = final_rating(session, &BTreeMap::new(), &grades).expect("expected rating");
    let result = candidate_final(session, &BTreeMap::new(), &grades).expect("candidate final");
    let CandidateFinal::Breakdown(view) = &result else {
        panic!("expected breakdown")
    };
    assert_eq!(view.rating, expected.rating);
    assert_eq!(view.solved_count, expected.solved_count);
}

#[test]
fn solution_editor_sync_is_not_subject_to_pairproof_paste_block() {
    let alpha: PathBuf =
        std::env::temp_dir().join(format!("pairproof-flow-{}-editor", std::process::id()));
    let _ = std::fs::remove_dir_all(&alpha);
    let dir = alpha.join("questions/01");
    std::fs::create_dir_all(&dir).expect("dirs");
    let path = dir.join("solution.py");
    std::fs::write(&path, b"print(1)\n").expect("initial write");
    let previous = solution_snapshot(&alpha, 0, Language::Python).expect("previous snapshot");
    let grown = "a".repeat(3000);
    std::fs::write(&path, grown.as_bytes()).expect("editor growth");
    assert_eq!(std::fs::read(&path).expect("read back").len(), 3000);
    let current = solution_snapshot(&alpha, 0, Language::Python).expect("current snapshot");
    let flag = detect_bulk_insert(&previous, &current).expect("bulk insert detected");
    assert!(flag.inserted_bytes >= 2048, "{flag:?}");
    let mut chat = ChatController::default();
    let error = chat.paste_input(&grown).expect_err("chat paste blocked");
    assert_eq!(error.to_string(), "pasting is blocked here; type instead.");
    assert_eq!(chat.input_text(), "");
    let _ = std::fs::remove_dir_all(&alpha);
}

#[test]
fn candidate_never_renders_expert_only_diagnostics() {
    let mut state = solving_state(started());
    state
        .work
        .set_statement("e0", "Find two numbers that sum to target.");
    state.work.set_approaches(approach_set());
    let session = state.app.session().expect("session");
    let mut collected = state.app.screen_lines();
    collected.extend(work_lines(session, &state.work));
    collected.extend(state.chat.chat_lines());
    collected.extend(candidate_screen_lines(&state));
    let joined = collected.join("\n");
    for banned in [
        "ssh",
        "/home",
        "10.0.0",
        "MODEL_API_KEY",
        "pairproof-sync",
        "secret-host",
    ] {
        assert!(!joined.contains(banned), "{banned} leaked in {joined}");
    }
}

#[test]
fn candidate_terminal_is_restored_after_error_and_quit() {
    let mut escaped = CandidateRunState::new();
    handle_candidate_key(&mut escaped, key(KeyCode::Esc));
    assert_eq!(escaped.app.screen(), CandidateScreen::Exited);
    let mut controlled = CandidateRunState::new();
    handle_candidate_key(&mut controlled, ctrl(KeyCode::Char('c')));
    assert_eq!(controlled.app.screen(), CandidateScreen::Exited);
    let guard = CandidateTtyGuard;
    drop(guard);
    // candidate_loop is intentionally not called: tests have no TTY.
}

#[test]
fn now_ms_is_positive_epoch_millis() {
    let first = pairproof::tui::candidate_run::now_ms();
    assert!(first > 1_700_000_000_000, "{first}");
    let second = pairproof::tui::candidate_run::now_ms();
    assert!(second >= first, "{second} < {first}");
}
