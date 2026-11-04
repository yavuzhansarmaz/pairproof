use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::control::{CandidateSnapshot, SnapshotOption, SnapshotQuestion};
use pairproof::tui::candidate::{CandidateApp, CandidateConnectError, CandidateScreen};
use pairproof::tui::candidate_chat::ChatController;
use pairproof::tui::candidate_run::{handle_candidate_key, CandidateFocus, CandidateRunState};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::candidate_work::CandidateWork;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
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

fn qs(prefix: &str, n: usize) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| QuestionRef {
            title_slug: format!("{prefix}{i}"),
            title: format!("Title {prefix}{i}"),
            difficulty: Difficulty::Easy,
            acceptance_pct: None,
        })
        .collect()
}

fn solving_state() -> CandidateRunState {
    let mut session = Session::new("s1".into(), Language::Python, qs("e", 5), 0).expect("session");
    session.mark_waiting_for_candidate(2).expect("waiting");
    session.candidate_connected(3).expect("connected");
    session.start(4).expect("start");
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&encoded_connection());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    app.attach_session(session);
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    CandidateRunState::with_parts(app, CandidateWork::new(), ChatController::default())
}

#[test]
fn coach_stream_does_not_block_question_navigation() {
    let snapshot = CandidateSnapshot {
        v: 1,
        session_id: "s1".to_string(),
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion {
            index: 0,
            title: "Two Sum".to_string(),
            slug: "two-sum".to_string(),
            statement: Some("Find two numbers.".to_string()),
        }),
        options: vec![
            SnapshotOption {
                title: "Brute force".to_string(),
                summary: "Try every pair".to_string(),
            },
            SnapshotOption {
                title: "Hash map".to_string(),
                summary: "One pass lookup".to_string(),
            },
            SnapshotOption {
                title: "Sort first".to_string(),
                summary: "Sort then scan".to_string(),
            },
            SnapshotOption {
                title: "Two pointers".to_string(),
                summary: "Converge from ends".to_string(),
            },
        ],
        nav: Vec::new(),
        selected: None,
        latest_test: None,
        chat_replies: Vec::new(),
        final_breakdown: None,
        chat_partial: Some("Hel".to_string()),
        recovery: None,
    };
    let view = CandidateView::from_snapshot(Some(&snapshot));
    let questions = view.question_lines().join("\n");
    for title in ["Brute force", "Hash map", "Sort first", "Two pointers"] {
        assert!(questions.contains(title), "{questions}");
    }
    assert!(
        view.chat_lines().iter().any(|l| l == "> Hel"),
        "{:?}",
        view.chat_lines()
    );
    assert!(view.lines().iter().any(|l| l == "> Hel"));
    let mut state = solving_state();
    assert_eq!(state.focus, CandidateFocus::Work);
    let before = state.focus;
    handle_candidate_key(&mut state, key(KeyCode::Tab));
    assert_ne!(state.focus, before);
    assert_eq!(state.focus, CandidateFocus::Chat);
}

#[test]
fn coach_whitespace_only_partial_renders_no_blank_line() {
    let mut view = CandidateView::from_snapshot(None);
    view.chat_partial = Some("   ".to_string());
    let lines = view.chat_lines();
    assert!(!lines.iter().any(|l| l.trim() == ">"), "{lines:?}");
    assert_eq!(lines.len(), 2, "{lines:?}");
    view.chat_partial = Some(" hi".to_string());
    let lines = view.chat_lines();
    assert!(lines.iter().any(|l| l.contains("hi")), "{lines:?}");
}
