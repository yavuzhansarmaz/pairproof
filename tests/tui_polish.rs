use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::ConnectionGate;
use pairproof::tui::app::{screen_lines, AppEvent, AppScreen, ExpertApp};
use pairproof::tui::candidate::{CandidateApp, CandidateConnectError, CandidateScreen};
use pairproof::tui::candidate_chat::ChatController;
use pairproof::tui::candidate_run::{
    candidate_screen_lines, handle_candidate_key, CandidateFocus, CandidateRunState,
};
use pairproof::tui::candidate_runtime::CandidateRuntimeError;
use pairproof::tui::candidate_work::CandidateWork;
use pairproof::tui::help::{
    candidate_connect_message, candidate_error_message, candidate_help_lines,
    candidate_help_lines_for, candidate_is_typing, countdown_line, disclosure_lines,
    expert_disclosure_action, expert_error_message, expert_event_action, expert_help_lines,
    expert_key_action, help_overlay_dispatches, ExpertKeyAction,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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

fn begun() -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}

struct OkVerifier;

impl pairproof::tui::candidate::ConnectionVerifier for OkVerifier {
    fn verify(
        &self,
        _c: &pairproof::connection::ConnectionString,
        _now_ms: u64,
    ) -> Result<(), CandidateConnectError> {
        Ok(())
    }
}

fn encoded_connection() -> String {
    pairproof::connection::ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        pairproof::connection::Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    )
    .encode()
}

fn solving_state(session: Session) -> CandidateRunState {
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&encoded_connection());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    app.attach_session(session);
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    CandidateRunState::with_parts(app, CandidateWork::new(), ChatController::default())
}

#[test]
fn help_screen_lists_role_specific_keybindings() {
    let role = expert_help_lines(AppScreen::RoleSelect).join("\n");
    assert!(role.contains("e expert"), "{role}");
    assert!(role.contains("c candidate"), "{role}");
    assert!(role.contains("esc quit"), "{role}");
    assert!(!role.contains("q quit"), "{role}");
    let monitor = expert_help_lines(AppScreen::Monitor).join("\n");
    assert!(monitor.contains("R reissue"), "{monitor}");
    assert!(monitor.contains("e end the session"), "{monitor}");
    let wait = expert_help_lines(AppScreen::ConnectionWait).join("\n");
    assert!(wait.contains("Waiting for the candidate"), "{wait}");
    let solving = candidate_help_lines(CandidateScreen::Solving).join("\n");
    assert!(solving.contains("1-4 choose an approach"), "{solving}");
    assert!(solving.contains("Tab switch pane"), "{solving}");
    assert!(solving.contains("esc quit"), "{solving}");
    assert!(!solving.contains("q quit"), "{solving}");
    assert!(role.contains("pairproof update"), "{role}");
    assert!(role.contains("pairproof uninstall"), "{role}");
    assert!(
        role.contains("pairproof uninstall --keep-data remove binary, keep data"),
        "{role}"
    );
    assert!(
        !role.contains("checks for updates when it starts"),
        "{role}"
    );
    assert!(!role.contains("d Meta disclosure"), "{role}");
    assert!(role.contains("d read the disclosure"), "{role}");
    assert!(role.contains("b close help"), "{role}");
    assert!(!role.contains("Press ? to close help"), "{role}");
    let disclosure = disclosure_lines().join("\n");
    assert!(disclosure.contains("Meta disclosure"), "{disclosure}");
    assert!(disclosure.contains("50,000 characters"), "{disclosure}");
    assert!(
        disclosure.contains("remembered on this machine"),
        "{disclosure}"
    );
    let connect = candidate_help_lines(CandidateScreen::Connect).join("\n");
    assert!(connect.contains("Enter connect"), "{connect}");
    let other =
        candidate_help_lines_for(CandidateScreen::Solving, CandidateFocus::Other).join("\n");
    assert!(other.contains("Enter confirm the approach"), "{other}");
    let chat = candidate_help_lines_for(CandidateScreen::Solving, CandidateFocus::Chat).join("\n");
    assert!(chat.contains("Enter send the message"), "{chat}");
}

#[test]
fn candidate_help_does_not_expose_expert_controls() {
    let mut all = Vec::new();
    for screen in [
        CandidateScreen::Connect,
        CandidateScreen::Rules,
        CandidateScreen::Waiting,
        CandidateScreen::Solving,
        CandidateScreen::Final,
        CandidateScreen::Exited,
    ] {
        all.extend(candidate_help_lines(screen));
        for focus in [
            CandidateFocus::Work,
            CandidateFocus::Other,
            CandidateFocus::Redirect,
            CandidateFocus::Chat,
        ] {
            all.extend(candidate_help_lines_for(screen, focus));
        }
    }
    let joined = all.join("\n");
    let lower = joined.to_lowercase();
    for banned in [
        "reissue",
        "monitor",
        "kill",
        "ssh",
        "end the session",
        "secret",
        "fingerprint",
        "model_api_key",
        "/home",
    ] {
        assert!(!lower.contains(banned), "{banned} leaked in {joined}");
    }
    assert!(!joined.contains("R reissue"), "{joined}");
}

#[test]
fn error_messages_are_actionable_and_secret_safe() {
    let expert = expert_error_message("Selection failed", "\u{1b}[31mbroken\u{7}");
    assert!(expert.contains("Selection failed"), "{expert}");
    assert!(expert.contains("try again"), "{expert}");
    assert!(!expert.contains('\u{1b}'), "{expert}");
    assert!(!expert.contains('\u{7}'), "{expert}");
    let blank = expert_error_message("End failed", "  ");
    assert!(blank.contains("End failed"), "{blank}");
    assert!(blank.contains("try again"), "{blank}");
    for err in [
        CandidateRuntimeError::SyncFailed,
        CandidateRuntimeError::Transport,
        CandidateRuntimeError::WrongSession,
        CandidateRuntimeError::NotConnected,
        CandidateRuntimeError::Ended,
    ] {
        let msg = candidate_error_message(&err);
        assert!(
            msg.contains("ask the expert") || msg.contains("try again"),
            "{msg}"
        );
        assert!(!msg.contains('\u{1b}'), "{msg}");
    }
}

#[test]
fn candidate_errors_remain_generic() {
    assert_eq!(
        candidate_error_message(&CandidateRuntimeError::WrongSession),
        "Connection problem \u{2014} ask the expert"
    );
    assert_ne!(
        candidate_error_message(&CandidateRuntimeError::WrongSession),
        CandidateRuntimeError::WrongSession.to_string()
    );
    assert_eq!(
        candidate_connect_message(&CandidateConnectError::Expired),
        "Connection string expired; ask the expert for the current string"
    );
    let mut messages = vec![
        candidate_error_message(&CandidateRuntimeError::SyncFailed),
        candidate_error_message(&CandidateRuntimeError::Transport),
        candidate_error_message(&CandidateRuntimeError::NotConnected),
        candidate_error_message(&CandidateRuntimeError::Ended),
        candidate_connect_message(&CandidateConnectError::Malformed),
        candidate_connect_message(&CandidateConnectError::Rejected),
        candidate_connect_message(&CandidateConnectError::Unavailable),
    ];
    messages.push(candidate_connect_message(&CandidateConnectError::Expired));
    for msg in messages {
        let lower = msg.to_lowercase();
        for banned in [
            "ssh",
            "another session",
            "secret",
            "fingerprint",
            "endpoint",
            "model_api_key",
            "/home",
            "10.0.0",
        ] {
            assert!(!lower.contains(banned), "{banned} leaked in {msg}");
        }
    }
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text("canary-value-123");
    assert!(app.submit_connection(1_000).is_err());
    let lines = app.screen_lines().join("\n");
    assert!(lines.contains("Connection string is malformed"), "{lines}");
    assert!(!lines.contains("canary-value-123"), "{lines}");
}

#[test]
fn countdown_rendering_tracks_actual_state() {
    const T0: u64 = 1_000_000;
    let gate = ConnectionGate::new(
        Endpoint::new("127.0.0.1", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
        T0,
    );
    assert_eq!(gate.countdown_secs(T0), 60);
    assert_eq!(countdown_line(gate.countdown_secs(T0)), "Expires in 60s.");
    assert_eq!(
        countdown_line(gate.countdown_secs(T0 + 30_000)),
        "Expires in 30s."
    );
    assert_eq!(
        countdown_line(gate.countdown_secs(T0 + 59_000)),
        "Expires in 1s."
    );
    assert_eq!(
        countdown_line(gate.countdown_secs(T0 + 60_000)),
        "Expires in 0s."
    );
}

#[test]
fn waiting_state_renders_without_blocking() {
    let mut session = begun();
    for i in 0..5u64 {
        session.finish_question(10 + i).expect("finish easy");
    }
    assert_eq!(session.status(), SessionStatus::WaitingForExpert);
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&encoded_connection());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    app.attach_session(session);
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Waiting));
    assert_eq!(
        app.waiting_lines(),
        vec!["Expert is preparing questions".to_string()]
    );
    let lines = candidate_screen_lines(&CandidateRunState::with_parts(
        app,
        CandidateWork::new(),
        ChatController::default(),
    ))
    .join("\n");
    assert!(lines.contains("Expert is preparing questions"), "{lines}");
    let mut state = solving_state(begun());
    state.app.attach_session({
        let mut s = begun();
        for i in 0..5u64 {
            s.finish_question(10 + i).expect("finish easy");
        }
        s
    });
    assert_eq!(state.app.screen(), CandidateScreen::Waiting);
    handle_candidate_key(&mut state, key(KeyCode::Char('q')));
    assert_eq!(state.app.screen(), CandidateScreen::Waiting);
    handle_candidate_key(&mut state, key(KeyCode::Esc));
    assert_eq!(state.app.screen(), CandidateScreen::Exited);
    let mut expert = ExpertApp::new();
    expert.handle(AppEvent::SelectExpert).expect("preflight");
    expert.set_preflight_passed(true);
    expert.set_api_key_ready(true);
    expert
        .handle(AppEvent::DisclosureAcknowledged)
        .expect("disclosure");
    expert.handle(AppEvent::PreflightCompleted).expect("setup");
    expert.set_has_proposal(true);
    expert.handle(AppEvent::ProposalReady).expect("proposal");
    expert
        .handle(AppEvent::ProposalConfirmed)
        .expect("connection wait");
    assert_eq!(expert.screen(), AppScreen::ConnectionWait);
    assert!(screen_lines(&expert)
        .join("\n")
        .contains("Waiting for candidate to connect."));
}

#[test]
fn help_key_ignored_while_typing() {
    let mut typing = CandidateRunState::new();
    handle_candidate_key(&mut typing, key(KeyCode::Char('?')));
    assert_eq!(typing.app.screen(), CandidateScreen::Connect);
    assert_eq!(typing.app.input_len(), 1);
    assert!(!typing.show_help);
    let mut state = solving_state(begun());
    handle_candidate_key(&mut state, key(KeyCode::Char('?')));
    assert!(state.show_help);
    assert_eq!(state.app.screen(), CandidateScreen::Solving);
    assert!(candidate_screen_lines(&state)
        .join("\n")
        .contains("1-4 choose an approach"));
    handle_candidate_key(&mut state, key(KeyCode::Char('?')));
    assert!(state.show_help, "? must not close help any more");
    handle_candidate_key(&mut state, key(KeyCode::Char('b')));
    assert!(!state.show_help);
    for _ in 0..3 {
        handle_candidate_key(&mut state, key(KeyCode::Tab));
    }
    assert_eq!(state.focus, CandidateFocus::Chat);
    handle_candidate_key(&mut state, key(KeyCode::Char('?')));
    assert!(!state.show_help);
    assert_eq!(state.chat.input_text(), "?");
}

#[test]
fn candidate_is_typing_covers_text_fields_and_work_pane() {
    for focus in [
        CandidateFocus::Work,
        CandidateFocus::Other,
        CandidateFocus::Redirect,
        CandidateFocus::Chat,
    ] {
        assert!(candidate_is_typing(CandidateScreen::Connect, focus));
    }
    for focus in [
        CandidateFocus::Other,
        CandidateFocus::Redirect,
        CandidateFocus::Chat,
    ] {
        assert!(candidate_is_typing(CandidateScreen::Solving, focus));
    }
    assert!(!candidate_is_typing(
        CandidateScreen::Solving,
        CandidateFocus::Work
    ));
    for screen in [
        CandidateScreen::Rules,
        CandidateScreen::Waiting,
        CandidateScreen::Final,
        CandidateScreen::Exited,
    ] {
        for focus in [
            CandidateFocus::Work,
            CandidateFocus::Other,
            CandidateFocus::Redirect,
            CandidateFocus::Chat,
        ] {
            assert!(!candidate_is_typing(screen, focus), "{screen:?}/{focus:?}");
        }
    }
}

#[test]
fn expert_event_action_never_dispatches_modified_letter_keys() {
    let ctrl = |code: KeyCode| KeyEvent::new(code, KeyModifiers::CONTROL);
    assert_eq!(
        expert_event_action(ctrl(KeyCode::Char('c')), false),
        ExpertKeyAction::Quit
    );
    // Ctrl-E on Monitor must not end the session, Ctrl-C on
    // RoleSelect must not select the candidate, Ctrl-B must not go back.
    for code in [
        KeyCode::Char('e'),
        KeyCode::Char('b'),
        KeyCode::Char('r'),
        KeyCode::Char('q'),
    ] {
        assert_eq!(
            expert_event_action(ctrl(code), false),
            ExpertKeyAction::Ignore
        );
    }
    assert_eq!(
        expert_event_action(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT), false),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_event_action(key(KeyCode::Char('e')), false),
        ExpertKeyAction::Dispatch
    );
    // Shift-modified keys (R reissue, ?) keep working.
    assert_eq!(
        expert_event_action(
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT),
            false
        ),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_event_action(
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            false
        ),
        ExpertKeyAction::ToggleHelp
    );
}

#[test]
fn preflight_retry_is_requested_only_by_r_on_preflight() {
    use pairproof::tui::expert_tty::preflight_retry_requested;
    assert!(preflight_retry_requested(
        AppScreen::Preflight,
        KeyCode::Char('r')
    ));
    assert!(!preflight_retry_requested(
        AppScreen::Setup,
        KeyCode::Char('r')
    ));
    assert!(!preflight_retry_requested(
        AppScreen::Preflight,
        KeyCode::Enter
    ));
}

#[test]
fn disclosure_ack_persists_to_disk() {
    use pairproof::tui::expert_tty::{load_disclosure_ack, save_disclosure_ack};
    let dir = std::env::temp_dir().join(format!("pairproof-ack-{}", std::process::id()));
    let path = dir.join("disclosure_acknowledged");
    assert!(!load_disclosure_ack(&path));
    save_disclosure_ack(&path).expect("save ack");
    assert!(load_disclosure_ack(&path));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn d_opens_disclosure_only_from_help() {
    assert_eq!(
        expert_key_action(KeyCode::Char('d'), true),
        ExpertKeyAction::ShowDisclosure
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('d'), false),
        ExpertKeyAction::Dispatch
    );
}

#[test]
fn help_overlay_still_dispatches_role_keys_on_main_menu() {
    assert!(help_overlay_dispatches(
        KeyCode::Char('e'),
        AppScreen::RoleSelect
    ));
    assert!(help_overlay_dispatches(
        KeyCode::Char('c'),
        AppScreen::RoleSelect
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Char('e'),
        AppScreen::Monitor
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Enter,
        AppScreen::RoleSelect
    ));
    assert!(!help_overlay_dispatches(
        KeyCode::Char('x'),
        AppScreen::RoleSelect
    ));
}

#[test]
fn disclosure_reader_esc_quits_and_b_returns_to_help() {
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Esc)),
        ExpertKeyAction::Quit
    );
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Char('b'))),
        ExpertKeyAction::ReturnToHelp
    );
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Char('d'))),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_disclosure_action(key(KeyCode::Char('?'))),
        ExpertKeyAction::Ignore
    );
    let disclosure = disclosure_lines().join("\n");
    assert!(
        disclosure.contains("esc quit, b back to help"),
        "{disclosure}"
    );
    assert!(!disclosure.contains("Press d"), "{disclosure}");
}

#[test]
fn expert_key_action_toggles_and_swallows_while_help_open() {
    assert_eq!(
        expert_key_action(KeyCode::Char('?'), false),
        ExpertKeyAction::ToggleHelp
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('?'), true),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('b'), true),
        ExpertKeyAction::ToggleHelp
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('q'), true),
        ExpertKeyAction::Ignore
    );
    assert_eq!(
        expert_key_action(KeyCode::Esc, true),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('b'), false),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_key_action(KeyCode::Char('q'), false),
        ExpertKeyAction::Dispatch
    );
    assert_eq!(
        expert_key_action(KeyCode::Esc, false),
        ExpertKeyAction::Dispatch
    );
}
