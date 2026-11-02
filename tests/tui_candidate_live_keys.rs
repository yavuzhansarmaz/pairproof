use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::orchestration::control::{CandidateSnapshot, Intent, IntentKind};
use pairproof::tui::candidate::CandidateScreen;
use pairproof::tui::candidate_run::{CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::{
    handle_live_key, CandidateRuntime, CandidateRuntimeError, CandidateTransport,
};

#[derive(Default)]
struct FakeTransport {
    sent: Vec<Intent>,
}

impl CandidateTransport for FakeTransport {
    fn authenticate(&mut self, _c: &ConnectionString) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> {
        Ok("s1".to_string())
    }
    fn write_hello(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn start_sync(&mut self, _s: &str, _a: &Path) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> {
        self.sent.push(intent.clone());
        Ok(())
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        Ok(None)
    }
    fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
}

fn connected_runtime() -> CandidateRuntime<FakeTransport> {
    let conn = ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    );
    let dir: PathBuf =
        std::env::temp_dir().join(format!("pairproof-live-key-{}", std::process::id()));
    let mut runtime = CandidateRuntime::new("s1", dir, FakeTransport::default());
    runtime.connect(&conn).expect("connect");
    runtime
}

fn key(code: K) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn question_mark_types_in_chat_focus() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Chat;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('?')), &mut status);
    assert_eq!(state.chat.input_text(), "?");
    assert!(!state.show_help);
}

#[test]
fn question_mark_toggles_help_in_work_focus() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('?')), &mut status);
    assert!(state.show_help);
}

#[test]
fn a_sends_change_approach_when_connected() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('a')), &mut status);
    assert_eq!(runtime.transport().sent.len(), 1);
    assert_eq!(runtime.transport().sent[0].kind, IntentKind::ChangeApproach);
    assert_eq!(status, None);
}

#[test]
fn empty_chat_enter_sends_nothing() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Chat;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Enter), &mut status);
    assert!(runtime.transport().sent.is_empty());
    assert!(
        status
            .as_ref()
            .is_some_and(|s| s.contains("type a message")),
        "{status:?}"
    );
}

#[test]
fn q_does_not_quit_live_or_offline() {
    let mut state = CandidateRunState::new();
    state.focus = CandidateFocus::Work;
    let mut runtime = connected_runtime();
    let mut status = None;
    handle_live_key(&mut state, &mut runtime, key(K::Char('q')), &mut status);
    assert_ne!(state.app.screen(), CandidateScreen::Exited);
    assert!(runtime.transport().sent.is_empty());
}

#[test]
fn shared_work_key_classifier_maps_both_paths() {
    use pairproof::tui::candidate_run::{next_focus, work_key_action, WorkKeyAction};
    assert_eq!(work_key_action('1'), Some(WorkKeyAction::SelectPreset(0)));
    assert_eq!(work_key_action('4'), Some(WorkKeyAction::SelectPreset(3)));
    assert_eq!(work_key_action('5'), Some(WorkKeyAction::OpenOther));
    assert_eq!(work_key_action('o'), Some(WorkKeyAction::OpenOther));
    assert_eq!(work_key_action('c'), Some(WorkKeyAction::Continue));
    assert_eq!(work_key_action('d'), Some(WorkKeyAction::OpenRedirect));
    assert_eq!(work_key_action('a'), Some(WorkKeyAction::ChangeApproach));
    assert_eq!(work_key_action('t'), Some(WorkKeyAction::RequestTests));
    assert_eq!(work_key_action('f'), Some(WorkKeyAction::Finish));
    assert_eq!(work_key_action('s'), Some(WorkKeyAction::Skip));
    assert_eq!(work_key_action('r'), Some(WorkKeyAction::RequestReQuestion));
    assert_eq!(work_key_action('x'), None);
    assert_eq!(next_focus(CandidateFocus::Work), CandidateFocus::Other);
    assert_eq!(next_focus(CandidateFocus::Other), CandidateFocus::Redirect);
    assert_eq!(next_focus(CandidateFocus::Redirect), CandidateFocus::Chat);
    assert_eq!(next_focus(CandidateFocus::Chat), CandidateFocus::Work);
}
