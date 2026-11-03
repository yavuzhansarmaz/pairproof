use pairproof::core::{ApproachChoice, Language, Session, TestSummary};
use pairproof::llm::chat::{
    assemble_chat_messages, build_live_state_block, detect_boundary_violation,
    log_boundary_violation_if_present, run_chat, ChatState, CHAT_SYSTEM_PROMPT,
    MAX_HISTORY_MESSAGES, MAX_SOLUTION_CHARS,
};
use pairproof::llm::{LlmError, Message, MetaClient};

mod common_mod;
use common_mod::q_easy as q;

fn started() -> Session {
    let easy = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn plain_state() -> ChatState {
    ChatState {
        question_statement: "Find two numbers that sum to target.".into(),
        approach_text: Some("preset option 1".into()),
        stage: "active, approach selected".into(),
        solution_code: "def solve(): pass".into(),
        latest_test: Some(TestSummary {
            passed: 2,
            failed: 1,
            at_ms: 50,
        }),
        recent_actions: vec!["redirected: use a heap".into()],
    }
}

#[test]
fn hidden_token_data_never_enters_chat_messages() {
    let mut session = started();
    session
        .select_approach(ApproachChoice::Preset(1), 424242, 10)
        .expect("select");
    session.add_tokens(9876).expect("tokens");
    session.record_paste_flag(5000, 11).expect("paste");
    let state = ChatState::from_session(
        &session,
        "Find two numbers that sum to target.",
        "def solve(): pass",
    );
    let messages = assemble_chat_messages(&state, &[], "What should I consider?");
    let joined = messages
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("424242"), "{joined}");
    assert!(!joined.contains("9876"), "{joined}");
    assert_eq!(state.approach_text.as_deref(), Some("preset option 1"));
}

#[test]
fn live_state_block_is_labelled_as_data() {
    let block = build_live_state_block(&plain_state());
    let first_line = block.lines().next().unwrap_or_default();
    assert!(first_line.contains("data"), "{block}");
    assert!(block.contains("Find two numbers"), "{block}");
    assert!(block.contains("preset option 1"), "{block}");
    assert!(block.contains("active, approach selected"), "{block}");
    assert!(block.contains("passed 2, failed 1"), "{block}");
    assert!(block.contains("redirected: use a heap"), "{block}");
    let mut bare = plain_state();
    bare.approach_text = None;
    bare.latest_test = None;
    bare.recent_actions.clear();
    let block = build_live_state_block(&bare);
    assert!(block.contains("none selected"), "{block}");
}

#[test]
fn solution_truncation_respects_char_bound_and_marker() {
    let mut state = plain_state();
    state.solution_code = "é".repeat(5000);
    let block = build_live_state_block(&state);
    assert_eq!(block.matches('é').count(), MAX_SOLUTION_CHARS);
    assert!(block.contains("[truncated]"), "marker present");
    state.solution_code = "x = 1".into();
    let block = build_live_state_block(&state);
    assert!(block.contains("x = 1"), "{block}");
    assert!(!block.contains("[truncated]"), "{block}");
}

#[test]
fn history_is_trimmed_to_last_six() {
    let history: Vec<Message> = (0..10)
        .map(|i| {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            Message::new(role, &format!("h{i}"))
        })
        .collect();
    let messages = assemble_chat_messages(&plain_state(), &history, "next question");
    assert_eq!(messages.len(), 2 + MAX_HISTORY_MESSAGES + 1);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[0].content, CHAT_SYSTEM_PROMPT);
    assert_eq!(messages[1].role, "system");
    for (offset, kept) in messages[2..8].iter().enumerate() {
        let original = &history[4 + offset];
        assert_eq!(kept.role, original.role);
        assert_eq!(kept.content, original.content);
    }
    assert_eq!(messages[2].content, "h4");
    assert_eq!(messages[7].content, "h9");
}

#[test]
fn user_message_is_verbatim_and_last() {
    let text = "  keep   my\n spacing \t exactly  ";
    let messages = assemble_chat_messages(&plain_state(), &[], text);
    let last = messages.last().expect("last message");
    assert_eq!(last.role, "user");
    assert_eq!(last.content, text);
}

#[test]
fn boundary_violation_detected_on_code_fence_only() {
    assert!(detect_boundary_violation("```python\nprint(1)\n```"));
    assert!(detect_boundary_violation("here:\n```\ncode\n```"));
    assert!(!detect_boundary_violation(
        "Think about what a map gives you."
    ));
    assert!(!detect_boundary_violation("Consider the value `x` here."));
    assert!(!detect_boundary_violation(""));
}

#[test]
fn boundary_violation_is_logged_as_event() {
    let mut session = started();
    let clean = log_boundary_violation_if_present(&mut session, "Try a hint first.", 20)
        .expect("clean logs");
    assert!(!clean);
    assert_eq!(session.chat_boundary_violations(), 0);
    let fenced =
        log_boundary_violation_if_present(&mut session, "```\nfull code\n```", 21).expect("logs");
    assert!(fenced);
    assert_eq!(session.chat_boundary_violations(), 1);
    let again =
        log_boundary_violation_if_present(&mut session, "No code here.", 22).expect("clean logs");
    assert!(!again);
    assert_eq!(session.chat_boundary_violations(), 1);
}

#[test]
fn from_session_reflects_current_slot_state() {
    let mut session = started();
    session
        .select_approach(ApproachChoice::Preset(2), 100, 10)
        .expect("select");
    session.redirect("use a heap", 11).expect("redirect");
    session.record_test_result(3, 1, 12).expect("tests");
    let state = ChatState::from_session(&session, "Q statement", "code here");
    assert_eq!(state.approach_text.as_deref(), Some("preset option 2"));
    assert_eq!(state.question_statement, "Q statement");
    assert_eq!(state.solution_code, "code here");
    let latest = state.latest_test.expect("latest test");
    assert_eq!((latest.passed, latest.failed), (3, 1));
    let actions = state.recent_actions.join("\n");
    assert!(actions.contains("preset option 2"), "{actions}");
    assert!(actions.contains("use a heap"), "{actions}");
    assert!(actions.contains("passed 3, failed 1"), "{actions}");
    session.finish_question(13).expect("finish");
    let next = ChatState::from_session(&session, "Next Q", "");
    assert_eq!(next.approach_text, None);
    assert_eq!(next.latest_test, None);
    assert!(next.recent_actions.is_empty(), "{:?}", next.recent_actions);
}

#[tokio::test]
async fn run_chat_unreachable_api_is_typed_error() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let mut session = started();
    let state = ChatState::from_session(&session, "Q statement", "code");
    let err = run_chat(&client, &mut session, &state, &[], "hint please", 30).await;
    assert!(matches!(err, Err(LlmError::Api(_))), "{err:?}");
    assert_eq!(session.current_slot().expect("slot").actual_tokens, 0);
    assert_eq!(session.chat_boundary_violations(), 0);
}

#[test]
fn system_prompt_forbids_full_solution() {
    let prompt = CHAT_SYSTEM_PROMPT.to_lowercase();
    assert!(prompt.contains("never"), "{prompt}");
    assert!(prompt.contains("solution"), "{prompt}");
    assert!(prompt.contains("code"), "{prompt}");
    assert!(prompt.contains("full solution"), "{prompt}");
    assert!(prompt.contains("full"), "{prompt}");
    assert!(prompt.contains("reveal"), "{prompt}");
    assert!(prompt.contains("correct"), "{prompt}");
    assert!(prompt.contains("socratic"), "{prompt}");
    assert!(prompt.contains("data"), "{prompt}");
}
