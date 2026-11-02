use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::tui::app::{AppEvent, AppScreen, ExpertApp};
use pairproof::tui::candidate::{
    placeholder_message, CandidateApp, CandidateConnectError, CandidateScreen, ConnectionVerifier,
};
use pairproof::tui::dispatch_role;
use pairproof::tui::shared::{destination_for, parse_role, Role, RoleDestination};

const HOST: &str = "secret-host.example.com";
const USERNAME: &str = "pp-abcd1234";
const FINGERPRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct OkVerifier;

impl ConnectionVerifier for OkVerifier {
    fn verify(
        &self,
        _connection: &ConnectionString,
        _now_ms: u64,
    ) -> Result<(), CandidateConnectError> {
        Ok(())
    }
}

struct RejectVerifier;

impl ConnectionVerifier for RejectVerifier {
    fn verify(
        &self,
        _connection: &ConnectionString,
        _now_ms: u64,
    ) -> Result<(), CandidateConnectError> {
        Err(CandidateConnectError::Rejected)
    }
}

struct Valid {
    encoded: String,
    secret_hex: String,
}

fn valid(expires_at_ms: u64) -> Valid {
    let secret = Secret::from_bytes([0xab; 16]);
    let secret_hex = secret.expose_hex();
    let value = ConnectionString::new(
        Endpoint::new(HOST, 2222).expect("endpoint"),
        EphemeralUsername::parse(USERNAME).expect("username"),
        secret,
        expires_at_ms,
        Fingerprint::parse(FINGERPRINT).expect("fingerprint"),
    );
    Valid {
        encoded: value.encode(),
        secret_hex,
    }
}

fn assert_hidden(haystack: &str, secret_hex: &str) {
    assert!(!haystack.contains(secret_hex), "secret leaked: {haystack}");
    assert!(!haystack.contains(HOST), "host leaked: {haystack}");
    assert!(!haystack.contains(USERNAME), "username leaked: {haystack}");
    assert!(
        !haystack.contains(FINGERPRINT),
        "fingerprint leaked: {haystack}"
    );
}

fn connected_app() -> (CandidateApp, Valid) {
    let value = valid(10_000);
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&value.encoded);
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    (app, value)
}

fn question(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
        acceptance_pct: None,
    }
}

fn started_session() -> Session {
    let easy = (0..5)
        .map(|i| question(&format!("easy-{i}"), Difficulty::Easy))
        .collect();
    let mut session = Session::new("s1".to_string(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn waiting_for_expert_session() -> Session {
    let mut session = started_session();
    for i in 0..5u64 {
        session.finish_question(10 + i).expect("finish");
    }
    assert_eq!(session.status(), SessionStatus::WaitingForExpert);
    session
}

#[test]
fn candidate_role_enters_candidate_flow() {
    assert_eq!(parse_role("candidate"), Some(Role::Candidate));
    assert_eq!(destination_for(Role::Candidate), RoleDestination::Candidate);
    assert_eq!(dispatch_role(Role::Candidate), RoleDestination::Candidate);
    assert!(!placeholder_message().contains("not implemented"));
    assert!(!placeholder_message().contains("Phase 9"));
    let app = CandidateApp::new();
    assert_eq!(app.screen(), CandidateScreen::Connect);
    assert!(!app.is_connected());
    assert!(!app.can_solve());
}

#[test]
fn candidate_route_does_not_change_expert_flow() {
    assert_eq!(parse_role("expert"), Some(Role::Expert));
    assert_eq!(destination_for(Role::Expert), RoleDestination::Expert);
    assert_eq!(dispatch_role(Role::Expert), RoleDestination::Expert);
    let mut app = ExpertApp::new();
    assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    assert_eq!(app.screen(), AppScreen::Preflight);
}

#[test]
fn candidate_rejects_malformed_connection_string_safely() {
    let value = valid(10_000);
    let raw = format!(
        "bogus:{HOST}:{USERNAME}:{}:10000:{FINGERPRINT}",
        value.secret_hex
    );
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&raw);
    let err = app.submit_connection(1_000).expect_err("malformed");
    assert_eq!(err, CandidateConnectError::Malformed);
    assert_eq!(app.screen(), CandidateScreen::Connect);
    assert!(!app.is_connected());
    assert_hidden(&err.to_string(), &value.secret_hex);
    assert_hidden(&app.screen_lines().join("\n"), &value.secret_hex);
    assert_hidden(&format!("{app:?}"), &value.secret_hex);
    assert!(!err.to_string().contains(&raw));
}

#[test]
fn candidate_expired_string_requests_current_string() {
    let value = valid(5_000);
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&value.encoded);
    let err = app.submit_connection(5_000).expect_err("expired");
    assert_eq!(err, CandidateConnectError::Expired);
    let message = err.to_string();
    assert!(message.contains("expired"));
    assert!(message.contains("ask the expert for the current string"));
    assert_eq!(app.screen(), CandidateScreen::Connect);
    assert!(!app.is_connected());
    assert_hidden(&message, &value.secret_hex);
}

#[test]
fn candidate_connection_secret_never_renders_or_debugs() {
    let value = valid(10_000);
    let mut app = CandidateApp::with_verifier(OkVerifier);
    for c in value.encoded.chars() {
        app.push_char(c);
    }
    assert_eq!(app.input_len(), value.encoded.len());
    let lines = app.screen_lines().join("\n");
    assert_hidden(&lines, &value.secret_hex);
    assert!(!lines.contains(&value.encoded));
    assert!(lines.contains('\u{2022}'));
    assert_hidden(&format!("{app:?}"), &value.secret_hex);
    assert!(!format!("{app:?}").contains(&value.encoded));
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    assert_eq!(app.input_len(), 0);
    assert_hidden(&app.screen_lines().join("\n"), &value.secret_hex);
    assert_hidden(&format!("{app:?}"), &value.secret_hex);
}

#[test]
fn candidate_not_connected_until_verified() {
    let value = valid(10_000);
    let mut app = CandidateApp::new();
    app.push_text(&value.encoded);
    let err = app.submit_connection(1_000).expect_err("unavailable");
    assert_eq!(err, CandidateConnectError::Unavailable);
    assert!(err.to_string().contains("not available yet"));
    assert!(!err.to_string().contains("Phase"));
    assert!(!app.is_connected());
    assert_eq!(app.screen(), CandidateScreen::Connect);

    app.set_verifier(RejectVerifier);
    let err = app.submit_connection(1_000).expect_err("rejected");
    assert_eq!(err, CandidateConnectError::Rejected);
    assert!(!app.is_connected());
    assert_eq!(app.screen(), CandidateScreen::Connect);

    app.set_verifier(OkVerifier);
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    assert!(app.is_connected());
    assert_eq!(app.input_len(), 0);
}

#[test]
fn candidate_new_unavailable_is_explicit_legacy_default() {
    let value = valid(10_000);
    let mut app = CandidateApp::new_unavailable();
    assert_eq!(app.screen(), CandidateScreen::Connect);
    app.push_text(&value.encoded);
    let err = app.submit_connection(1_000).expect_err("unavailable");
    assert_eq!(err, CandidateConnectError::Unavailable);
    assert!(!app.is_connected());
    let mut legacy = CandidateApp::new();
    legacy.push_text(&value.encoded);
    assert_eq!(
        legacy.submit_connection(1_000),
        Err(CandidateConnectError::Unavailable)
    );
}

#[test]
fn candidate_rules_require_explicit_acceptance() {
    let mut fresh = CandidateApp::new();
    assert!(fresh.accept_rules().is_err());
    assert!(!fresh.rules_accepted());

    let (mut app, _value) = connected_app();
    assert_eq!(app.screen(), CandidateScreen::Rules);
    assert!(!app.rules_accepted());
    assert!(!app.can_solve());
    let rules = app.rules_lines().join("\n");
    assert!(rules.contains("single workspace only"));
    assert!(rules.contains("no external resources"));
    assert!(rules.contains("bulk pastes are flagged"));
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Waiting));
    assert!(app.rules_accepted());
    assert!(!app.can_solve());
}

#[test]
fn candidate_actions_blocked_before_consent() {
    let mut app = CandidateApp::new();
    assert!(!app.can_solve());
    assert!(app.accept_rules().is_err());
    app.attach_session(started_session());
    assert_eq!(app.screen(), CandidateScreen::Connect);
    assert!(!app.can_solve());

    let (mut app, _value) = connected_app();
    app.attach_session(started_session());
    assert_eq!(app.screen(), CandidateScreen::Rules);
    assert!(!app.can_solve());
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    assert!(app.can_solve());
}

#[test]
fn waiting_screen_uses_real_waiting_for_expert_state() {
    let (mut app, _value) = connected_app();
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Waiting));
    let session = waiting_for_expert_session();
    assert_eq!(session.status(), SessionStatus::WaitingForExpert);
    app.attach_session(session);
    assert_eq!(app.screen(), CandidateScreen::Waiting);
    assert!(app
        .waiting_lines()
        .iter()
        .any(|line| line == "Expert is preparing questions"));
    assert!(app
        .screen_lines()
        .iter()
        .any(|line| line == "Expert is preparing questions"));
    assert!(!app.can_solve());
}

#[test]
fn candidate_connection_error_hides_expert_diagnostics() {
    let value = valid(10_000);
    let raw = format!(
        "pairproof:v1:{HOST}:2222:pp-bad!user:{}:10000:{FINGERPRINT}",
        value.secret_hex
    );
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&raw);
    let err = app.submit_connection(1_000).expect_err("malformed");
    assert_eq!(err, CandidateConnectError::Malformed);
    assert_hidden(&err.to_string(), &value.secret_hex);
    assert!(!err.to_string().contains("pp-bad!user"));
    assert_hidden(&format!("{app:?}"), &value.secret_hex);
    assert_hidden(&app.screen_lines().join("\n"), &value.secret_hex);

    let mut rejected = CandidateApp::with_verifier(RejectVerifier);
    rejected.push_text(&value.encoded);
    let err = rejected.submit_connection(1_000).expect_err("rejected");
    assert_eq!(err, CandidateConnectError::Rejected);
    assert_hidden(&err.to_string(), &value.secret_hex);
    assert!(!err.to_string().contains("ssh"));
    assert!(!err.to_string().contains("fingerprint mismatch"));
    assert_hidden(&rejected.screen_lines().join("\n"), &value.secret_hex);
    assert_hidden(&format!("{rejected:?}"), &value.secret_hex);
}

#[test]
fn candidate_input_editing_and_quit_from_any_screen() {
    let mut app = CandidateApp::new();
    app.push_char('a');
    app.push_text("bc");
    assert_eq!(app.input_len(), 3);
    assert_eq!(
        app.connect_line(),
        "Connection string: \u{2022}\u{2022}\u{2022} (3 characters)"
    );
    app.backspace();
    assert_eq!(app.input_len(), 2);
    app.clear_input();
    assert_eq!(app.input_len(), 0);
    assert_eq!(app.quit(), CandidateScreen::Exited);
    assert_eq!(app.screen(), CandidateScreen::Exited);

    let (mut app, _value) = connected_app();
    assert_eq!(app.quit(), CandidateScreen::Exited);
}

#[test]
fn session_status_reflects_attached_session() {
    let mut app = CandidateApp::new();
    assert_eq!(app.session_status(), None);
    app.attach_session(started_session());
    assert_eq!(app.session_status(), Some(SessionStatus::InProgress));
    app.attach_session(waiting_for_expert_session());
    assert_eq!(app.session_status(), Some(SessionStatus::WaitingForExpert));
}
