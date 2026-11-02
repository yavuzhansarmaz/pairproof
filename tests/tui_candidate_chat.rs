use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pairproof::core::{ApproachChoice, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::chat::{ChatState, CHAT_SYSTEM_PROMPT};
use pairproof::llm::Message;
use pairproof::tui::candidate_chat::{
    other_insert, ChatController, ChatError, ChatInput, ChatService, ChatServiceError,
    PasteBlocked, UnavailableChatService,
};
use pairproof::tui::candidate_work::CandidateWork;

fn q(slug: &str) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty: Difficulty::Easy,
        acceptance_pct: None,
    }
}

fn started() -> Session {
    let easy = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn state(session: &Session) -> ChatState {
    ChatState::from_session(
        session,
        "Find two numbers that sum to target.",
        "def solve(): pass",
    )
}

#[derive(Clone)]
struct FakeService {
    captured: Rc<RefCell<Vec<Message>>>,
    calls: Rc<Cell<usize>>,
    reply: Result<String, ChatServiceError>,
}

impl ChatService for FakeService {
    fn complete(&mut self, messages: &[Message]) -> Result<String, ChatServiceError> {
        self.calls.set(self.calls.get() + 1);
        *self.captured.borrow_mut() = messages.to_vec();
        self.reply.clone()
    }
}

type Captured = Rc<RefCell<Vec<Message>>>;
type Calls = Rc<Cell<usize>>;

fn controller(reply: Result<String, ChatServiceError>) -> (ChatController, Captured, Calls) {
    let captured = Rc::new(RefCell::new(Vec::new()));
    let calls = Rc::new(Cell::new(0));
    let service = FakeService {
        captured: captured.clone(),
        calls: calls.clone(),
        reply,
    };
    (ChatController::new(service), captured, calls)
}

#[test]
fn candidate_chat_uses_phase4_interface() {
    let mut session = started();
    let state = state(&session);
    let (mut chat, captured, calls) = controller(Ok("Consider invariants.".to_string()));
    chat.type_text("  hello coach  ");
    let reply = chat
        .submit(&mut session, &state, 100)
        .expect("chat succeeds");
    assert_eq!(reply, "Consider invariants.");
    assert_eq!(calls.get(), 1);
    let messages = captured.borrow();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, CHAT_SYSTEM_PROMPT);
    assert_eq!(messages[1].role, "system");
    assert!(messages[1].content.contains("Live session data"));
    let last = messages.last().expect("last message");
    assert_eq!(last.role, "user");
    assert_eq!(last.content, "  hello coach  ");
    assert_eq!(chat.history().len(), 2);
    assert_eq!(chat.history()[0].role, "user");
    assert_eq!(chat.history()[0].content, "  hello coach  ");
    assert_eq!(chat.history()[1].role, "assistant");
    assert_eq!(chat.history()[1].content, "Consider invariants.");
    assert!(chat.input_text().is_empty());
    assert!(!chat.is_busy());
}

#[test]
fn candidate_chat_context_excludes_hidden_scoring_data() {
    let mut session = started();
    session
        .select_approach(ApproachChoice::Preset(1), 424242, 10)
        .expect("select");
    session.add_tokens(9876).expect("tokens");
    session.record_paste_flag(5000, 11).expect("paste flag");
    let state = state(&session);
    assert_eq!(state.approach_text.as_deref(), Some("preset option 1"));
    let (mut chat, captured, _calls) = controller(Ok("Think about a map.".to_string()));
    chat.type_text("What should I consider?");
    chat.submit(&mut session, &state, 100)
        .expect("chat succeeds");
    let joined = captured
        .borrow()
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("424242"), "{joined}");
    assert!(!joined.contains("9876"), "{joined}");
}

#[test]
fn candidate_chat_displays_service_error_without_sensitive_details() {
    let mut session = started();
    let state = state(&session);
    let (mut chat, _captured, calls) = controller(Err(ChatServiceError::Failed));
    chat.type_text("help please");
    let error = chat
        .submit(&mut session, &state, 100)
        .expect_err("failed chat");
    assert_eq!(error, ChatError::Failed);
    assert_eq!(calls.get(), 1);
    assert_eq!(chat.input_text(), "help please");
    assert!(chat.history().is_empty());
    assert!(!chat.is_busy());
    let lines = chat.chat_lines().join("\n");
    assert!(lines.contains("chat failed; try again."), "{lines}");
    assert!(lines.contains("message: help please"), "{lines}");
    for banned in ["score", "rating", "token", "424242", "secret", "api key"] {
        assert!(
            !lines.to_lowercase().contains(banned),
            "{banned} in {lines}"
        );
    }
    assert!(!error.to_string().to_lowercase().contains("secret"));
    let mut unavailable = ChatController::new(UnavailableChatService);
    unavailable.type_text("hello");
    let error = unavailable
        .submit(&mut session, &state, 101)
        .expect_err("unavailable chat");
    assert_eq!(error, ChatError::Unavailable);
    assert!(unavailable
        .chat_lines()
        .join("\n")
        .contains("chat is not available yet."));
    assert_eq!(unavailable.input_text(), "hello");
}

#[test]
fn candidate_chat_prevents_duplicate_submit_when_busy() {
    let mut session = started();
    let state = state(&session);
    let (mut chat, _captured, calls) = controller(Ok("Reply.".to_string()));
    chat.type_text("hello");
    chat.set_busy(true);
    assert!(chat.is_busy());
    assert!(chat.chat_lines().join("\n").contains("sending..."));
    let error = chat
        .submit(&mut session, &state, 100)
        .expect_err("busy chat");
    assert_eq!(error, ChatError::Busy);
    assert_eq!(calls.get(), 0);
    assert_eq!(chat.input_text(), "hello");
    assert!(chat.history().is_empty());
    chat.set_busy(false);
    chat.submit(&mut session, &state, 101).expect("after busy");
    assert_eq!(calls.get(), 1);
}

#[test]
fn candidate_blocks_paste_in_chat() {
    let mut input = ChatInput::new();
    input.push_typed("hi");
    let error = input
        .insert_paste("bulk pasted text")
        .expect_err("paste blocked");
    assert_eq!(error, PasteBlocked::Blocked);
    assert_eq!(error.to_string(), "pasting is blocked here; type instead.");
    assert_eq!(input.text(), "hi");
    assert!(!input.is_empty());
    let (mut chat, _captured, _calls) = controller(Ok("Reply.".to_string()));
    chat.type_text("hi");
    let error = chat
        .paste_input("bulk pasted text")
        .expect_err("paste blocked");
    assert_eq!(error.to_string(), "pasting is blocked here; type instead.");
    assert_eq!(chat.input_text(), "hi");
    let lines = chat.chat_lines().join("\n");
    assert!(
        lines.contains("pasting is blocked here; type instead."),
        "{lines}"
    );
    chat.type_char('!');
    assert_eq!(chat.input_text(), "hi!");
    assert!(!chat
        .chat_lines()
        .join("\n")
        .contains("pasting is blocked here"));
}

#[test]
fn candidate_blocks_paste_in_other_input() {
    let mut work = CandidateWork::new();
    other_insert(&mut work, "typed plan", false).expect("typed other");
    assert_eq!(work.other_text(), "typed plan");
    let error = other_insert(&mut work, "pasted bulk", true).expect_err("paste blocked");
    assert_eq!(error.to_string(), "pasting is blocked here; type instead.");
    assert_eq!(work.other_text(), "typed plan");
    other_insert(&mut work, " plus", false).expect("more typing");
    assert_eq!(work.other_text(), "typed plan plus");
}

#[test]
fn candidate_allows_normal_typing_in_chat_and_other() {
    let mut input = ChatInput::new();
    input.push_char('a');
    input.push_typed(" b, c! d?");
    input.push_char('\u{1b}');
    assert_eq!(input.text(), "a b, c! d?");
    input.backspace();
    assert_eq!(input.text(), "a b, c! d");
    input.clear();
    assert!(input.is_empty());
    let (mut chat, _captured, _calls) = controller(Ok("Reply.".to_string()));
    chat.type_char('H');
    chat.type_text("i, coach: ready?");
    assert_eq!(chat.input_text(), "Hi, coach: ready?");
    chat.backspace_input();
    assert_eq!(chat.input_text(), "Hi, coach: ready");
    let mut work = CandidateWork::new();
    other_insert(&mut work, "Use DP + memo, O(n)!", false).expect("typed other");
    assert_eq!(work.other_text(), "Use DP + memo, O(n)!");
    work.push_other_char(' ');
    work.push_other_char('x');
    assert_eq!(work.other_text(), "Use DP + memo, O(n)! x");
}

#[test]
fn candidate_chat_boundary_violation_is_logged_not_scored() {
    let mut session = started();
    let state = state(&session);
    let fenced = "```\nprint(1)\n```".to_string();
    let (mut chat, _captured, _calls) = controller(Ok(fenced.clone()));
    chat.type_text("show me");
    let reply = chat
        .submit(&mut session, &state, 100)
        .expect("reply returned");
    assert_eq!(reply, fenced);
    assert_eq!(session.chat_boundary_violations(), 1);
    assert_eq!(chat.history().len(), 2);
    assert_eq!(chat.history()[1].content, fenced);
    assert!(chat.input_text().is_empty());
    let lines = chat.chat_lines().join("\n");
    assert!(!lines.to_lowercase().contains("rating"), "{lines}");
    assert!(!lines.to_lowercase().contains("score"), "{lines}");
}

#[test]
fn chat_last_error_tracks_generic_service_failure() {
    let mut session = started();
    let state = state(&session);
    let mut chat = ChatController::new(UnavailableChatService);
    assert_eq!(chat.last_error(), None);
    chat.type_text("hello coach");
    let error = chat
        .submit(&mut session, &state, 100)
        .expect_err("unavailable chat");
    assert_eq!(error, ChatError::Unavailable);
    assert_eq!(chat.last_error(), Some(ChatServiceError::Unavailable));
    assert_eq!(chat.input_text(), "hello coach");
}
