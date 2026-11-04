use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use pairproof::core::{Language, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::chat::ChatState;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};

use crate::common_mod;
use common_mod::qs;

type PF<'a, T> = PipelineFuture<'a, T>;

#[rustfmt::skip]
fn started() -> Orchestrator {
    let mut o = Orchestrator::create("s1".into(), Language::Python, qs("e", 5, Difficulty::Easy), 0).expect("orch");
    o.mark_waiting_for_candidate(1).expect("wait"); o.attach_candidate(2).expect("attach"); o.start(3).expect("start"); o
}

#[derive(Clone)]
struct Script {
    chunks: Vec<String>,
    fail: bool,
}
#[rustfmt::skip]
fn script(chunks: &[&str], fail: bool) -> Script { Script { chunks: chunks.iter().map(|c| c.to_string()).collect(), fail } }

#[derive(Default)]
struct StreamingChat {
    scripts: Mutex<VecDeque<Script>>,
    calls: AtomicU32,
}
#[rustfmt::skip]
impl StreamingChat { fn new(scripts: Vec<Script>) -> Self { Self { scripts: Mutex::new(scripts.into()), calls: AtomicU32::new(0) } } }
#[rustfmt::skip]
impl ChatTransport for StreamingChat {
    fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> { Box::pin(async move { Ok("fallback".to_string()) }) }
    fn chat_streaming<'a>(&'a self, _s: &'a mut Session, _st: &'a ChatState, _h: &'a [Message], _m: &'a str, _at: u64, on_chunk: &'a mut (dyn FnMut(&str) + Send)) -> PF<'a, String> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self.scripts.lock().expect("scripts").pop_front().unwrap_or_else(|| script(&[], true));
            for chunk in &next.chunks { on_chunk(chunk); tokio::task::yield_now().await; }
            tokio::task::yield_now().await; tokio::task::yield_now().await;
            if next.fail { return Err(PipelineError::ChatUnavailable); }
            Ok(next.chunks.concat())
        })
    }
}

struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, _s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { Err(PipelineError::ChatUnavailable) }) }
}
struct FakeSandbox;
#[rustfmt::skip]
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, _i: usize, _l: Language, _c: &'a str) -> PF<'a, ()> { Box::pin(async move { Ok(()) }) }
    fn read_solution<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, String> { Box::pin(async move { Ok(String::new()) }) }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> { Box::pin(async move { Ok(RunnabilityReport { runnable: true, exit_code: Some(0), stdout: String::new(), stderr: String::new(), truncated: false }) }) }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> { Box::pin(async move { Ok(TestOutcome { success: true, exit_code: Some(0), stdout: "ok".into(), stderr: String::new(), stdout_truncated: false, stderr_truncated: false }) }) }
}
struct FakeGenerator;
#[rustfmt::skip]
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> { Box::pin(async move { Err(PipelineError::ChatUnavailable) }) }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> { Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 424242, "hidden".into()).map_err(|e| PipelineError::Llm(e.to_string())) }) }
}

#[derive(Default)]
struct CapturingIo {
    intents: Mutex<Vec<u8>>,
    snapshots: Mutex<Vec<Vec<u8>>>,
    processed: Mutex<Option<ProcessedState>>,
}
#[rustfmt::skip]
impl CapturingIo {
    fn append(&self, intent: &Intent) { let mut line = CandidateControlClient::intent_line(intent).expect("line"); line.push('\n'); self.intents.lock().expect("intents").extend_from_slice(line.as_bytes()); }
    fn partials(&self) -> Vec<Option<String>> { self.snapshots().iter().map(|s| s.chat_partial.clone()).collect() }
    fn snapshots(&self) -> Vec<CandidateSnapshot> { self.snapshots.lock().expect("snaps").iter().map(|b| serde_json::from_slice(b).expect("snapshot")).collect() }
}
#[rustfmt::skip]
impl ControlIo for CapturingIo {
    fn setup_dirs<'a>(&'a self, _u: &'a str) -> ControlFuture<'a, ()> { Box::pin(async move { Ok(()) }) }
    fn read_intents<'a>(&'a self) -> ControlFuture<'a, Vec<u8>> { Box::pin(async move { Ok(self.intents.lock().expect("intents").clone()) }) }
    fn write_snapshot_atomic<'a>(&'a self, bytes: &'a [u8]) -> ControlFuture<'a, ()> { Box::pin(async move { self.snapshots.lock().expect("snaps").push(bytes.to_vec()); Ok(()) }) }
    fn load_processed<'a>(&'a self) -> ControlFuture<'a, Option<ProcessedState>> { Box::pin(async move { Ok(self.processed.lock().expect("processed").clone()) }) }
    fn store_processed<'a>(&'a self, state: &'a ProcessedState) -> ControlFuture<'a, ()> { Box::pin(async move { *self.processed.lock().expect("processed") = Some(state.clone()); Ok(()) }) }
}

struct Fakes {
    coding: FakeCoding,
    sandbox: FakeSandbox,
    generator: FakeGenerator,
    chat: StreamingChat,
}
#[rustfmt::skip]
impl Fakes {
    fn transports(&self) -> ControlTransports<'_> { ControlTransports { coding: &self.coding, sandbox: &self.sandbox, generator: &self.generator, chat: &self.chat } }
    fn new(scripts: Vec<Script>) -> Self { Self { coding: FakeCoding, sandbox: FakeSandbox, generator: FakeGenerator, chat: StreamingChat::new(scripts) } }
}

#[rustfmt::skip]
async fn run_chat(io: &CapturingIo, ex: &mut ControlExecutor, orch: &mut Orchestrator, pipe: &mut QuestionPipeline, fakes: &Fakes, seq: u64, chat_id: &str) -> Vec<ApplyReport> {
    io.append(&Intent::new("s1", seq, IntentKind::ChatRequest { chat_id: chat_id.into(), text: "hint?".into() }));
    ex.set_snapshot_base(build_snapshot(orch, pipe, SessionSyncPhase::Active, ex.chat_replies(), None));
    ex.drain(io, orch, pipe, &fakes.transports(), 10 + seq).await.expect("drain")
}
#[rustfmt::skip]
async fn setup(scripts: Vec<Script>) -> (CapturingIo, ControlExecutor, Orchestrator, QuestionPipeline, Fakes) {
    let io = CapturingIo::default(); let ex = ControlExecutor::restore("s1", &io).await.expect("executor");
    (io, ex, started(), QuestionPipeline::new(), Fakes::new(scripts))
}

#[tokio::test]
async fn coach_displays_first_chunk_without_waiting_for_completion() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hel", "lo"], false)]).await;
    let reports = run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    assert_eq!(reports[0].outcome, ApplyOutcome::Applied);
    assert!(
        io.partials().contains(&Some("Hel".to_string())),
        "partials: {:?}",
        io.partials()
    );
}

#[tokio::test]
async fn coach_stream_appends_chunks_incrementally() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hel", "lo"], false)]).await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    let partials: Vec<String> = io.partials().into_iter().flatten().collect();
    assert_eq!(
        partials,
        vec!["Hel".to_string(), "Hello".to_string()],
        "{partials:?}"
    );
}

#[tokio::test]
async fn coach_stream_finalizes_response_once() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hel", "lo"], false)]).await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    assert_eq!(ex.chat_replies(), ["Hello".to_string()]);
    assert_eq!(ex.chat_partial(), None);
    let again = ex
        .drain(&io, &mut orch, &mut pipe, &fakes.transports(), 20)
        .await
        .expect("redrain");
    assert!(
        again.iter().any(|r| r.outcome == ApplyOutcome::Duplicate),
        "{again:?}"
    );
    assert_eq!(ex.chat_replies().len(), 1);
}

#[tokio::test]
async fn coach_stream_handles_empty_response() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&[], true)]).await;
    let reports = run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    assert_eq!(reports[0].outcome, ApplyOutcome::Rejected);
    assert!(ex.chat_replies().is_empty());
    assert_eq!(ex.chat_partial(), None);
    assert!(io.partials().iter().all(|p| p.is_none()));
    let again = ex
        .drain(&io, &mut orch, &mut pipe, &fakes.transports(), 20)
        .await
        .expect("redrain");
    assert!(
        again.iter().any(|r| r.outcome == ApplyOutcome::Duplicate),
        "{again:?}"
    );
}

#[tokio::test]
async fn coach_stream_preserves_partial_output_on_error() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hel"], true)]).await;
    let reports = run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    assert_eq!(reports[0].outcome, ApplyOutcome::Rejected);
    assert_eq!(
        ex.chat_replies(),
        ["Hel [response interrupted]".to_string()]
    );
    assert_eq!(ex.chat_partial(), None);
    // Pipeline level: history stays unchanged on a streaming error.
    let mut orch2 = started();
    let pipe2 = QuestionPipeline::new();
    let chat2 = StreamingChat::new(vec![script(&["Hel"], true)]);
    let mut history: Vec<Message> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let err = pipe2
        .submit_chat_streaming(&mut orch2, &chat2, &mut history, "hint?", 20, &mut |d| {
            seen.push(d.to_string())
        })
        .await
        .expect_err("stream fails");
    assert_eq!(err, PipelineError::ChatUnavailable);
    assert!(history.is_empty());
    assert_eq!(seen, vec!["Hel".to_string()]);
}

#[tokio::test]
async fn coach_stream_cancellation_stops_pending_updates() {
    // Hermetic proxy for cancellation: the fake emits one chunk then errors,
    // so no later partial snapshot may appear after the error.
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hel"], true)]).await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    let partials: Vec<String> = io.partials().into_iter().flatten().collect();
    assert_eq!(partials, vec!["Hel".to_string()]);
    assert_eq!(ex.chat_partial(), None);
    assert_eq!(
        ex.chat_replies(),
        ["Hel [response interrupted]".to_string()]
    );
}

#[tokio::test]
async fn coach_stream_does_not_duplicate_messages() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hello"], false)]).await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    ex.drain(&io, &mut orch, &mut pipe, &fakes.transports(), 20)
        .await
        .expect("redrain");
    assert_eq!(fakes.chat.calls.load(Ordering::SeqCst), 1);
    assert_eq!(ex.chat_replies().len(), 1);
}

#[tokio::test]
async fn coach_stream_preserves_chat_history() {
    let scripts = vec![script(&["First"], false), script(&["Second"], false)];
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(scripts).await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 2, "c2").await;
    assert_eq!(
        ex.chat_replies(),
        ["First".to_string(), "Second".to_string()]
    );
    // Pipeline level: two successful chats append user+assistant pairs.
    let mut orch2 = started();
    let pipe2 = QuestionPipeline::new();
    let chat2 = StreamingChat::new(vec![script(&["First"], false), script(&["Second"], false)]);
    let mut history: Vec<Message> = Vec::new();
    for msg in ["one", "two"] {
        pipe2
            .submit_chat_streaming(&mut orch2, &chat2, &mut history, msg, 20, &mut |_| {})
            .await
            .expect("chat");
    }
    assert_eq!(history.len(), 4);
    assert_eq!(history[1].content, "First");
    assert_eq!(history[3].content, "Second");
}

#[tokio::test]
async fn two_chats_in_one_drain_keep_first_reply_visible_during_second_stream() {
    let scripts = vec![script(&["First"], false), script(&["Sec", "ond"], false)];
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(scripts).await;
    io.append(&Intent::new(
        "s1",
        1,
        IntentKind::ChatRequest {
            chat_id: "c1".into(),
            text: "one".into(),
        },
    ));
    io.append(&Intent::new(
        "s1",
        2,
        IntentKind::ChatRequest {
            chat_id: "c2".into(),
            text: "two".into(),
        },
    ));
    ex.set_snapshot_base(build_snapshot(
        &orch,
        &pipe,
        SessionSyncPhase::Active,
        ex.chat_replies(),
        None,
    ));
    let reports = ex
        .drain(&io, &mut orch, &mut pipe, &fakes.transports(), 10)
        .await
        .expect("drain");
    assert_eq!(
        reports
            .iter()
            .filter(|r| r.outcome == ApplyOutcome::Applied)
            .count(),
        2,
        "{reports:?}"
    );
    assert_eq!(
        ex.chat_replies(),
        ["First".to_string(), "Second".to_string()]
    );
    let during_second = io
        .snapshots()
        .into_iter()
        .find(|s| s.chat_partial.as_deref() == Some("Sec"))
        .expect("partial snapshot during second stream");
    assert_eq!(during_second.chat_replies, ["First".to_string()]);
    assert_eq!(during_second.chat_partial.as_deref(), Some("Sec"));
}

#[tokio::test]
async fn coach_stream_coalesces_publication_under_sustained_stream() {
    let chunks: Vec<String> = (0..5_000).map(|_| "x".to_string()).collect();
    let full = chunks.concat();
    let stress = Script {
        chunks,
        fail: false,
    };
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![stress]).await;
    let reports = run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    assert_eq!(reports[0].outcome, ApplyOutcome::Applied);
    assert_eq!(
        ex.chat_replies().first().map(String::as_str),
        Some(full.as_str())
    );
    assert_eq!(ex.chat_replies().len(), 1);
    assert_eq!(ex.chat_replies()[0].len(), 5_000);
    let partials: Vec<String> = io.partials().into_iter().flatten().collect();
    assert!(
        !partials.is_empty(),
        "expected at least one partial snapshot"
    );
    assert!(
        partials.len() < 50,
        "bounded publication expected, got {} partial snapshots",
        partials.len()
    );
    let mut previous_len = 0;
    for partial in &partials {
        assert!(
            full.starts_with(partial.as_str()),
            "partial is not a prefix"
        );
        assert!(partial.len() >= previous_len, "partial lengths decreased");
        previous_len = partial.len();
    }
    assert_eq!(ex.chat_partial(), None);
}

#[tokio::test]
async fn coach_stream_does_not_expose_hidden_runtime_state() {
    let (io, mut ex, mut orch, mut pipe, fakes) = setup(vec![script(&["Hello"], false)]).await;
    run_chat(&io, &mut ex, &mut orch, &mut pipe, &fakes, 1, "c1").await;
    let final_snap = build_snapshot(
        &orch,
        &pipe,
        SessionSyncPhase::Active,
        ex.chat_replies(),
        None,
    );
    let json = serde_json::to_string(&final_snap).expect("json");
    for hidden in [
        "expected_tokens",
        "ApproachQuality",
        "quality",
        "424242",
        "Best",
        "Workable",
    ] {
        assert!(!json.contains(hidden), "leaked {hidden}: {json}");
    }
    for snap in io.snapshots() {
        let text = serde_json::to_string(&snap).expect("snap json");
        assert!(!text.contains("424242"), "{text}");
    }
}
