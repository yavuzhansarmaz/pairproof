use std::sync::{Arc, Mutex};

use pairproof::core::{ApproachChoice, CoreError, Language, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::approaches::parse_approach_set;
use pairproof::llm::chat::{assemble_chat_messages, ChatState};
use pairproof::llm::Message;
use pairproof::orchestration::{
    ApproachGenerator, ChatTransport, FetchedStatement, Orchestrator, PipelineError,
    PipelineFuture, QuestionPipeline, StatementFetcher,
};
use pairproof::scoring::{ApproachGrade, ApproachSet};
use rand::rngs::StdRng;
use rand::SeedableRng;

use crate::common_mod;
use common_mod::qs;

fn started() -> Orchestrator {
    let mut o = Orchestrator::create(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .unwrap();
    o.mark_waiting_for_candidate(1).unwrap();
    o.attach_candidate(2).unwrap();
    o.start(3).unwrap();
    o
}

const SET_JSON: &str = r#"{"approaches":[
 {"title":"Brute force","summary":"Try every pair","quality":"weak","expected_tokens":111},
 {"title":"Hash map","summary":"One pass lookup","quality":"best","expected_tokens":424242},
 {"title":"Sort first","summary":"Sort then scan","quality":"workable","expected_tokens":333},
 {"title":"Two pointers","summary":"Converge from ends","quality":"workable","expected_tokens":444}]}"#;

struct FakeFetcher;
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(
        &'a self,
        slug: &'a str,
        _l: Language,
    ) -> PipelineFuture<'a, FetchedStatement> {
        let slug = slug.to_string();
        Box::pin(async move {
            Ok(FetchedStatement {
                statement: format!("Statement for {slug}: find two numbers."),
                starter_code: "# starter".into(),
            })
        })
    }
}

struct FakeGenerator;
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PipelineFuture<'a, ApproachSet> {
        Box::pin(async move {
            parse_approach_set(SET_JSON, &mut StdRng::seed_from_u64(7))
                .map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PipelineFuture<'a, ApproachGrade> {
        Box::pin(async move { Err(PipelineError::ChatUnavailable) })
    }
}

/// Production-shaped chat fake: assembles messages exactly like the real
/// Phase 4 chat turn and returns a canned light-model reply.
#[derive(Default)]
struct FakeChat {
    seen: Mutex<Vec<Vec<Message>>>,
    delay_ms: u64,
    fail: bool,
}
impl ChatTransport for FakeChat {
    fn chat<'a>(
        &'a self,
        _session: &'a mut Session,
        state: &'a ChatState,
        history: &'a [Message],
        user_message: &'a str,
        _at_ms: u64,
    ) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            if self.fail {
                return Err(PipelineError::ChatUnavailable);
            }
            let messages = assemble_chat_messages(state, history, user_message);
            self.seen.lock().unwrap().push(messages);
            Ok("Think about what a map gives you.".into())
        })
    }
}

async fn pipe_with_selection(orch: &mut Orchestrator) -> QuestionPipeline {
    let mut pipe = QuestionPipeline::new();
    pipe.load_current_statement(orch, &FakeFetcher)
        .await
        .unwrap();
    let views = pipe
        .generate_current_approaches(orch, &FakeGenerator)
        .await
        .unwrap();
    let preset = views.iter().position(|v| v.title == "Hash map").unwrap() as u8;
    orch.select_approach(ApproachChoice::Preset(preset), 424242, 10)
        .unwrap();
    orch.add_tokens(9876).unwrap();
    pipe
}

#[tokio::test]
async fn candidate_chat_uses_real_meta_transport() {
    let mut orch = started();
    let pipe = pipe_with_selection(&mut orch).await;
    let chat = FakeChat::default();
    let mut history: Vec<Message> = Vec::new();
    let reply = pipe
        .submit_chat(
            &mut orch,
            &chat,
            &mut history,
            "What should I consider?",
            20,
        )
        .await
        .expect("chat reply");
    assert_eq!(reply, "Think about what a map gives you.");
    // History is extended only on success, user then assistant.
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].role, "user");
    assert_eq!(history[0].content, "What should I consider?");
    assert_eq!(history[1].role, "assistant");
    assert_eq!(history[1].content, reply);
    // The transport saw Phase 4 assembled messages for the current slot.
    let joined = {
        let seen = chat.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        seen[0]
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(joined.contains("find two numbers"), "{joined}");
    assert!(joined.contains("preset option"), "{joined}");
    assert!(!pipe.is_chat_busy());
    // A failed turn leaves history untouched.
    let failing = FakeChat {
        fail: true,
        ..Default::default()
    };
    let err = pipe
        .submit_chat(&mut orch, &failing, &mut history, "again?", 21)
        .await
        .expect_err("unavailable");
    assert_eq!(err, PipelineError::ChatUnavailable);
    assert_eq!(history.len(), 2);
}

#[tokio::test]
async fn candidate_chat_keeps_phase4_hidden_state_excluded() {
    let mut orch = started();
    let pipe = pipe_with_selection(&mut orch).await;
    let chat = FakeChat::default();
    let mut history: Vec<Message> = Vec::new();
    pipe.submit_chat(&mut orch, &chat, &mut history, "hint please", 20)
        .await
        .unwrap();
    let seen = chat.seen.lock().unwrap();
    let joined = seen[0]
        .iter()
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for hidden in ["424242", "9876", "111", "333", "444"] {
        assert!(!joined.contains(hidden), "leaked {hidden}: {joined}");
    }
    // State itself is the hidden-free Phase 4 projection.
    let state = ChatState::from_session(orch.session(), "stmt", "code");
    assert!(state
        .approach_text
        .as_deref()
        .unwrap()
        .starts_with("preset option"));
}

#[tokio::test]
async fn candidate_chat_rejects_duplicate_inflight_submit() {
    let mut orch_a = started();
    let pipe = Arc::new(pipe_with_selection(&mut orch_a).await);
    let mut orch_b = orch_a.clone();
    let slow = FakeChat {
        delay_ms: 100,
        ..Default::default()
    };
    let fast = FakeChat::default();
    let mut history_a: Vec<Message> = Vec::new();
    let mut history_b: Vec<Message> = Vec::new();
    let first = pipe.submit_chat(&mut orch_a, &slow, &mut history_a, "first", 20);
    let second = pipe.submit_chat(&mut orch_b, &fast, &mut history_b, "second", 20);
    let (first_result, second_result) = tokio::join!(first, second);
    assert_eq!(
        second_result.expect_err("duplicate inflight"),
        PipelineError::ChatBusy
    );
    assert_eq!(history_b.len(), 0);
    assert_eq!(
        first_result.expect("first completes"),
        "Think about what a map gives you."
    );
    assert_eq!(history_a.len(), 2);
    assert!(!pipe.is_chat_busy());
    // After completion a new submit is accepted again.
    let mut history_c: Vec<Message> = Vec::new();
    pipe.submit_chat(&mut orch_a, &fast, &mut history_c, "third", 21)
        .await
        .expect("after busy clears");
    assert_eq!(history_c.len(), 2);
}

/// Chat fake that ends the session mid-call, mirroring
/// `FakeCoding { end_session: true }` in orchestrator_coding.
struct EndingChat;
impl ChatTransport for EndingChat {
    fn chat<'a>(
        &'a self,
        session: &'a mut Session,
        _state: &'a ChatState,
        _history: &'a [Message],
        _user_message: &'a str,
        _at_ms: u64,
    ) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            let _ = session.end_by_expert(9_999);
            Ok("late reply".into())
        })
    }
}

#[tokio::test]
async fn chat_reply_after_session_ends_is_rejected_without_history() {
    let mut orch = started();
    let pipe = pipe_with_selection(&mut orch).await;
    let mut history: Vec<Message> = vec![Message::new("user", "earlier")];
    let before = history.clone();
    let err = pipe
        .submit_chat(&mut orch, &EndingChat, &mut history, "hint please", 20)
        .await
        .expect_err("late chat reply refused");
    assert_eq!(err, PipelineError::Core(CoreError::Terminal.into()));
    assert!(orch.is_terminal());
    assert_eq!(history, before);
    assert!(!pipe.is_chat_busy());
}

struct BoundaryChat;
impl ChatTransport for BoundaryChat {
    fn chat<'a>(
        &'a self,
        session: &'a mut Session,
        _state: &'a ChatState,
        _history: &'a [Message],
        _user_message: &'a str,
        at_ms: u64,
    ) -> PipelineFuture<'a, String> {
        Box::pin(async move {
            let reply = "```python\nprint(1)\n```";
            pairproof::llm::chat::log_boundary_violation_if_present(session, reply, at_ms)
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            Ok(reply.to_string())
        })
    }
}

#[tokio::test]
async fn chat_boundary_violation_is_logged_in_real_session() {
    let mut orch = started();
    let pipe = pipe_with_selection(&mut orch).await;
    let mut history: Vec<Message> = Vec::new();
    assert_eq!(orch.session().chat_boundary_violations(), 0);
    let reply = pipe
        .submit_chat(&mut orch, &BoundaryChat, &mut history, "show code", 20)
        .await
        .expect("chat reply");
    assert!(reply.contains("```"));
    assert_eq!(orch.session().chat_boundary_violations(), 1);
    assert_eq!(history.len(), 2);
}
