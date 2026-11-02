use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{EventKind, Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::control::{
    final_ack_bytes, heartbeat_bytes, CandidateSnapshot, Intent, IntentKind, SnapshotFinal,
    SnapshotOption, SnapshotQuestion, SnapshotTest,
};
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_runtime::{
    CandidateRuntime, CandidateRuntimeError, CandidateRuntimePhase, CandidateTransport,
    CONNECTION_LOST,
};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, PollOutcome,
};

type PF<'a, T> = PipelineFuture<'a, T>;
const T0: u64 = 1_730_000_000_000;
#[rustfmt::skip]
fn q(s: &str, d: Difficulty) -> QuestionRef { QuestionRef { title_slug: s.into(), title: s.into(), difficulty: d, acceptance_pct: None } }
#[rustfmt::skip]
fn qs(p: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> { (0..n).map(|i| q(&format!("{p}{i}"), d)).collect() }
#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pairproof-expert-rt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d
}

struct FakeFetcher {
    hang: bool,
}
#[rustfmt::skip]
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let s = slug.to_string(); let hang = self.hang;
        Box::pin(async move { if hang { std::future::pending::<()>().await; } Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) })
    }
}
#[derive(Default)]
struct FakeGenerator;
#[rustfmt::skip]
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> {
        Box::pin(async move {
            let opt = |t: &str, quality: ApproachQuality, tok: u64| pairproof::scoring::ApproachOption { title: t.into(), summary: format!("{t} summary"), quality, expected_tokens: tok };
            ApproachSet::new(vec![opt("Brute force", ApproachQuality::Weak, 111), opt("Hash map", ApproachQuality::Best, 424242), opt("Sort first", ApproachQuality::Workable, 333), opt("Two pointers", ApproachQuality::Workable, 444)]).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> {
        Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) })
    }
}
#[derive(Default)]
struct FakeCoding {
    calls: AtomicU32,
}
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> {
        Box::pin(async move { self.calls.fetch_add(1, Ordering::SeqCst); s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) })
    }
}
#[derive(Default)]
struct FakeSandbox;
#[rustfmt::skip]
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, _i: usize, _l: Language, _c: &'a str) -> PF<'a, ()> { Box::pin(async move { Ok(()) }) }
    fn read_solution<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, String> { Box::pin(async move { Ok(String::new()) }) }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> { Box::pin(async move { Ok(RunnabilityReport { runnable: true, exit_code: Some(0), stdout: String::new(), stderr: String::new(), truncated: false }) }) }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> { Box::pin(async move { Ok(TestOutcome { success: true, exit_code: Some(0), stdout: "ok".into(), stderr: String::new(), stdout_truncated: false, stderr_truncated: false }) }) }
}
#[derive(Default)]
struct FakeChat;
#[rustfmt::skip]
impl ChatTransport for FakeChat {
    fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> {
        Box::pin(async move { Ok("Think about a map.".into()) })
    }
}
struct FakeGrader;
#[rustfmt::skip]
impl QuestionGrader for FakeGrader {
    fn grade_all<'a>(&'a self, _inputs: &'a [pairproof::llm::grading::QuestionScoringInput]) -> GradeFuture<'a> {
        Box::pin(async move { Vec::new() })
    }
}

#[rustfmt::skip]
async fn make_runtime(name: &str, hang: bool) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher { hang }), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding::default()), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, control, runtime)
}

#[rustfmt::skip] fn write_hello(control: &std::path::Path, session_id: &str) { let body = serde_json::json!({ "v": 1, "session_id": session_id }).to_string(); std::fs::write(control.join("inbox").join("hello.json"), body).unwrap(); }
#[rustfmt::skip] fn write_ack(control: &std::path::Path, session_id: &str) { std::fs::write(control.join("inbox").join("final-ack.json"), final_ack_bytes(session_id)).unwrap(); }
#[rustfmt::skip] fn write_heartbeat(control: &std::path::Path, session_id: &str, at_ms: u64) { std::fs::write(control.join("inbox").join("heartbeat.json"), heartbeat_bytes(session_id, at_ms)).unwrap(); }
#[rustfmt::skip] fn append_intent(control: &std::path::Path, intent: &Intent) { let mut file = std::fs::OpenOptions::new().create(true).append(true).open(control.join(INTENT_REL)).unwrap(); file.write_all(format!("{}\n", intent.to_line().unwrap()).as_bytes()).unwrap(); }

struct FakeGatedFetcher {
    entered: Arc<AtomicBool>,
    gate: Arc<tokio::sync::Notify>,
}
#[rustfmt::skip]
impl StatementFetcher for FakeGatedFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let s = slug.to_string(); let entered = self.entered.clone(); let gate = self.gate.clone();
        Box::pin(async move { entered.store(true, Ordering::SeqCst); gate.notified().await; Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) })
    }
}

const HOST: &str = "secret-host.example.com";
const USERNAME: &str = "pp-abcd1234";
const FINGERPRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Default)]
struct FakeTransport {
    ops: Vec<String>,
    snapshots: VecDeque<Option<CandidateSnapshot>>,
    sent: Vec<Intent>,
    read_err: Option<CandidateRuntimeError>,
    heartbeats: Vec<(String, u64)>,
    heartbeat_fails: bool,
}

impl CandidateTransport for FakeTransport {
    #[rustfmt::skip]
    fn authenticate(&mut self, _c: &ConnectionString) -> Result<(), CandidateRuntimeError> { self.ops.push("authenticate".to_string()); Ok(()) }
    #[rustfmt::skip]
    fn discover_session_id(&mut self) -> Result<String, CandidateRuntimeError> { self.ops.push("discover_session".to_string()); Ok("s1".to_string()) }
    #[rustfmt::skip]
    fn write_hello(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> { self.ops.push("write_hello".to_string()); Ok(()) }
    #[rustfmt::skip]
    fn start_sync(&mut self, _s: &str, _a: &std::path::Path) -> Result<(), CandidateRuntimeError> { self.ops.push("start_sync".to_string()); Ok(()) }
    #[rustfmt::skip]
    fn send_intent(&mut self, intent: &Intent) -> Result<(), CandidateRuntimeError> { self.ops.push("send_intent".to_string()); self.sent.push(intent.clone()); Ok(()) }
    #[rustfmt::skip]
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> { self.ops.push("read_snapshot".to_string()); if let Some(err) = &self.read_err { return Err(err.clone()); } Ok(self.snapshots.pop_front().unwrap_or(None)) }
    #[rustfmt::skip]
    fn write_heartbeat(&mut self, session_id: &str, at_ms: u64) -> Result<(), CandidateRuntimeError> { self.ops.push("write_heartbeat".to_string()); self.heartbeats.push((session_id.to_string(), at_ms)); if self.heartbeat_fails { Err(CandidateRuntimeError::Transport) } else { Ok(()) } }
    #[rustfmt::skip]
    fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> { self.ops.push("flush_sync".to_string()); Ok(()) }
    #[rustfmt::skip]
    fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> { self.ops.push("terminate_sync".to_string()); Ok(()) }
}

#[rustfmt::skip] fn connection() -> ConnectionString { ConnectionString::new(Endpoint::new(HOST, 2222).expect("endpoint"), EphemeralUsername::parse(USERNAME).expect("username"), Secret::from_bytes([0xab; 16]), 4_102_444_800_000, Fingerprint::parse(FINGERPRINT).expect("fingerprint")) }

#[rustfmt::skip]
fn snapshot(session_id: &str, phase: &str, final_breakdown: Option<SnapshotFinal>) -> CandidateSnapshot {
    CandidateSnapshot { v: 1, session_id: session_id.to_string(), phase: phase.to_string(), sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion { index: 0, title: "Two Sum".to_string(), slug: "two-sum".to_string(), statement: Some("Find two numbers that sum to target.".to_string()) }),
        options: vec![SnapshotOption { title: "Hash map".to_string(), summary: "One pass lookup".to_string() }], selected: Some("preset 0".to_string()),
        latest_test: Some(SnapshotTest { passed: 3, failed: 1, at_ms: 10 }), chat_replies: vec!["Try a hash map.".to_string()], final_breakdown }
}

#[rustfmt::skip] fn workspace(name: &str) -> PathBuf { let dir = std::env::temp_dir().join(format!("pairproof-candidate-runtime-{name}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).expect("workspace"); dir }
#[rustfmt::skip] fn runtime(name: &str, fake: FakeTransport) -> (PathBuf, CandidateRuntime<FakeTransport>) { let dir = workspace(name); let value = CandidateRuntime::new("s1", dir.clone(), fake); (dir, value) }
#[rustfmt::skip] fn connected(name: &str, fake: FakeTransport) -> (PathBuf, CandidateRuntime<FakeTransport>) { let (dir, mut value) = runtime(name, fake); value.connect(&connection()).expect("connect"); (dir, value) }

#[tokio::test]
#[rustfmt::skip]
async fn expert_detects_candidate_disconnect_on_heartbeat_timeout() {
    let (_root, control, mut runtime) = make_runtime("hb-timeout", false).await;
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    runtime.set_heartbeat_timeout(1_000);
    write_heartbeat(&control, "s1", T0 + 1_500);
    runtime.drain_intents(T0 + 1_800).await.unwrap();
    assert!(runtime.orchestrator().is_connected()); assert!(!runtime.is_disconnected());
    write_heartbeat(&control, "s1", T0 + 1_500);
    runtime.drain_intents(T0 + 2_900).await.unwrap();
    assert!(!runtime.orchestrator().is_connected()); assert!(runtime.is_disconnected());
    assert_eq!(runtime.gate_phase(), GatePhase::ReissueNeeded);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
    assert_eq!(runtime.orchestrator().current_index(), Some(0));
    assert!(runtime.cleanup_report().is_none(), "disconnect must not clean up");
    assert!(runtime.orchestrator().session().events().iter().any(|e| e.kind == EventKind::CandidateDisconnected));
}

#[tokio::test]
#[rustfmt::skip]
async fn expert_reissue_recovers_disconnected_candidate() {
    let (_root, control, mut runtime) = make_runtime("hb-reissue", false).await;
    assert_eq!(runtime.reissue(T0 + 500).await.unwrap_err().to_string(), "No disconnect to recover");
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    assert_eq!(runtime.reissue(T0 + 1_500).await.unwrap_err().to_string(), "No disconnect to recover");
    runtime.set_heartbeat_timeout(1_000);
    runtime.drain_intents(T0 + 2_500).await.unwrap();
    assert!(runtime.is_disconnected());
    assert_eq!(runtime.poll_candidate(T0 + 2_600).await, PollOutcome::Waiting);
    assert!(runtime.is_disconnected());
    let fresh = runtime.reissue(T0 + 3_000).await.expect("reissue");
    assert!(!fresh.encode().is_empty());
    assert_eq!(runtime.gate_phase(), GatePhase::Awaiting);
    assert_eq!(runtime.reissue_string().map(|s| s.encode()), Some(fresh.encode()));
    // Real candidate rewrites hello on every reconnect (10P consumes hello on accept).
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 3_500).await, PollOutcome::Connected);
    assert!(!runtime.is_disconnected()); assert!(runtime.orchestrator().is_connected());
    assert!(runtime.reissue_string().is_none(), "reissue display is consumed on reconnect");
}

#[tokio::test]
#[rustfmt::skip]
async fn reconnect_resumes_same_question_live_state() {
    let (_root, control, mut runtime) = make_runtime("hb-resume", false).await;
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    append_intent(&control, &Intent::new("s1", 1, IntentKind::Finish));
    runtime.drain_intents(T0 + 1_500).await.unwrap();
    assert_eq!(runtime.orchestrator().current_index(), Some(1));
    let slots_before = runtime.orchestrator().session().slots().to_vec();
    runtime.set_heartbeat_timeout(1_000);
    runtime.drain_intents(T0 + 3_000).await.unwrap();
    assert!(runtime.is_disconnected());
    runtime.reissue(T0 + 3_500).await.expect("reissue");
    // Real candidate rewrites hello on every reconnect (10P consumes hello on accept).
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 4_000).await, PollOutcome::Connected);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
    assert_eq!(runtime.orchestrator().current_index(), Some(1));
    assert_eq!(runtime.orchestrator().session().slots(), slots_before.as_slice());
    assert!(runtime.orchestrator().session().events().iter().any(|e| matches!(e.kind, EventKind::CandidateReconnected { .. })));
    assert_eq!(runtime.monitor_view(T0 + 4_000).current_index, Some(1));
}

#[tokio::test]
#[rustfmt::skip]
async fn stale_string_rejected_after_reissue() {
    let (_root, control, mut runtime) = make_runtime("hb-stale", false).await;
    let stale = runtime.current_string(T0 + 500).unwrap();
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    runtime.set_heartbeat_timeout(1_000);
    runtime.drain_intents(T0 + 2_500).await.unwrap();
    assert!(runtime.is_disconnected());
    assert!(runtime.verify_reconnect(&stale, T0 + 2_600).is_err(), "locked window fails closed");
    let fresh = runtime.reissue(T0 + 3_000).await.expect("reissue");
    assert_ne!(stale.secret(), fresh.secret());
    runtime.verify_reconnect(&fresh, T0 + 3_100).expect("fresh string verifies");
    let err = runtime.verify_reconnect(&stale, T0 + 3_100).unwrap_err();
    assert_eq!(err.to_string(), "stale connection string");
    assert!(!err.to_string().contains(&stale.secret().expose_hex()));
}

#[tokio::test]
#[rustfmt::skip]
async fn end_during_inflight_request_produces_valid_bundle() {
    let root = tmp("inflight-end"); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let entered = Arc::new(AtomicBool::new(false)); let gate = Arc::new(tokio::sync::Notify::new());
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeGatedFetcher { entered: entered.clone(), gate: gate.clone() }), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding::default()), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted };
    let mut runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    for _ in 0..1000 { if entered.load(Ordering::SeqCst) { break; } tokio::task::yield_now().await; }
    assert!(entered.load(Ordering::SeqCst), "statement fetch must be in flight");
    assert_eq!(runtime.pending_count(), 1);
    write_ack(&control, "s1");
    let report = runtime.end_session_with_ack_deadline(T0 + 2_000, 5_000).await.unwrap();
    assert_eq!(report.attempted.len(), 5); assert!(report.failed.is_empty(), "{report:?}");
    assert!(runtime.is_killed()); assert_eq!(runtime.gate_phase(), GatePhase::Ended);
    assert!(root.join("artifacts").join("s1").join("report.md").is_file());
    let final_bytes = std::fs::read(control.join(SNAPSHOT_REL)).unwrap();
    let final_snapshot = parse_snapshot(&final_bytes, "s1").unwrap();
    assert!(final_snapshot.final_breakdown.is_some()); assert_eq!(final_snapshot.phase, "EndedByExpert");
    let events_at_end = runtime.orchestrator().session().events().len();
    gate.notify_one();
    tokio::task::yield_now().await;
    assert!(runtime.drain_intents(T0 + 3_000).await.unwrap().is_empty());
    assert_eq!(runtime.poll_candidate(T0 + 3_000).await, PollOutcome::AlreadyConnected);
    assert_eq!(std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), final_bytes, "no post-end writes");
    assert_eq!(runtime.orchestrator().session().events().len(), events_at_end, "no post-end events");
}

#[test]
#[rustfmt::skip]
fn candidate_marks_connection_lost_on_transport_failure() {
    const T0: u64 = 1_730_000_000_000;
    let mut fake = FakeTransport::default(); fake.snapshots.push_back(Some(snapshot("s1", "InProgress", None)));
    let (dir, mut value) = connected("lost", fake);
    value.poll_snapshot_at(T0).expect("poll").expect("snapshot");
    assert!(!value.connection_lost()); assert_eq!(value.status_text(), None);
    assert_eq!(value.transport().heartbeats, vec![("s1".to_string(), T0)]);
    value.poll_snapshot_at(T0 + 1_000).expect("throttled poll");
    assert_eq!(value.transport().heartbeats.len(), 1, "heartbeat throttled within 5s");
    value.transport_mut().read_err = Some(CandidateRuntimeError::Transport);
    let err = value.poll_snapshot_at(T0 + 6_000).expect_err("transport down");
    assert_eq!(err, CandidateRuntimeError::Transport);
    assert_eq!(err.to_string(), "Connection problem \u{2014} ask the expert");
    assert!(value.connection_lost());
    assert_eq!(value.status_text().as_deref(), Some(CONNECTION_LOST));
    assert_eq!(value.transport().heartbeats.len(), 2, "heartbeat still attempted while failing");
    assert_eq!(value.view().question_title.as_deref(), Some("Two Sum"), "last snapshot keeps rendering");
    value.transport_mut().read_err = None;
    value.transport_mut().snapshots.push_back(Some(snapshot("s1", "InProgress", None)));
    value.poll_snapshot_at(T0 + 12_000).expect("poll").expect("snapshot");
    assert!(!value.connection_lost()); assert_eq!(value.status_text(), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[rustfmt::skip]
fn candidate_reconnects_with_fresh_string() {
    const T0: u64 = 1_730_000_000_000;
    let mut fake = FakeTransport::default(); fake.snapshots.push_back(Some(snapshot("s1", "InProgress", None)));
    let (dir, mut value) = connected("re-string", fake);
    value.poll_snapshot_at(T0).expect("poll").expect("snapshot");
    value.transport_mut().read_err = Some(CandidateRuntimeError::Transport);
    assert!(value.poll_snapshot_at(T0 + 1_000).is_err());
    assert!(value.connection_lost());
    value.transport_mut().read_err = None;
    let fresh = ConnectionString::new(Endpoint::new(HOST, 2222).expect("endpoint"), EphemeralUsername::parse(USERNAME).expect("username"), Secret::from_bytes([0xcd; 16]), 4_102_444_800_000, Fingerprint::parse(FINGERPRINT).expect("fingerprint"));
    value.reconnect(&fresh).expect("reconnect");
    assert_eq!(value.phase(), CandidateRuntimePhase::Connected);
    assert!(!value.connection_lost()); assert_eq!(value.session_id(), "s1");
    let ops = &value.transport().ops;
    let term = ops.iter().rposition(|o| o == "terminate_sync").expect("stale sync dropped");
    let auth = ops.iter().rposition(|o| o == "authenticate").expect("fresh auth");
    assert!(term < auth, "terminate precedes fresh connect: {ops:?}");
    assert!(ops.iter().rposition(|o| o == "write_hello").is_some_and(|hello| hello > auth));
    assert!(ops.iter().rposition(|o| o == "start_sync").is_some_and(|sync| sync > auth));
    value.send_intent(IntentKind::Continue).expect("intent after reconnect");
    assert_eq!(value.transport().sent.last().map(|i| i.session_id.as_str()), Some("s1"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
#[rustfmt::skip]
async fn reconnect_resets_sequence_without_losing_id_dedup() {
    let (_root, control, mut runtime) = make_runtime("seq-reset", false).await;
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    let first = Intent::new("s1", 1, IntentKind::Finish);
    append_intent(&control, &first);
    let pre = runtime.drain_intents(T0 + 1_500).await.unwrap();
    assert!(pre.iter().any(|r| r.intent_id.as_deref() == Some(first.id.as_str()) && r.outcome == ApplyOutcome::Applied), "{pre:?}");
    runtime.set_heartbeat_timeout(1_000);
    runtime.drain_intents(T0 + 3_000).await.unwrap();
    assert!(runtime.is_disconnected());
    runtime.reissue(T0 + 3_500).await.expect("reissue");
    // Real candidate rewrites hello on every reconnect (10P consumes hello on accept).
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 4_000).await, PollOutcome::Connected);
    append_intent(&control, &first);
    let fresh = Intent::new("s1", 1, IntentKind::Finish);
    assert_ne!(first.id, fresh.id, "fresh client sequence restarts seq with a new id");
    append_intent(&control, &fresh);
    let reports = runtime.drain_intents(T0 + 4_500).await.unwrap();
    assert!(reports.iter().any(|r| r.intent_id.as_deref() == Some(first.id.as_str()) && r.outcome == ApplyOutcome::Duplicate), "{reports:?}");
    assert!(reports.iter().any(|r| r.intent_id.as_deref() == Some(fresh.id.as_str()) && r.outcome == ApplyOutcome::Applied), "{reports:?}");
    assert!(!reports.iter().any(|r| r.outcome == ApplyOutcome::Rejected), "{reports:?}");
}

#[test] #[rustfmt::skip]
fn candidate_failed_heartbeat_not_retried_until_throttle() {
    let mut fake = FakeTransport { heartbeat_fails: true, ..FakeTransport::default() }; fake.snapshots.push_back(Some(snapshot("s1", "InProgress", None))); let (dir, mut value) = connected("hb-fail", fake);
    value.poll_snapshot_at(T0).expect("snapshot poll must survive heartbeat failure").expect("snapshot"); assert_eq!(value.transport().heartbeats, vec![("s1".to_string(), T0)]); assert!(!value.connection_lost());
    let ops = &value.transport().ops; let pos = |n: &str| ops.iter().position(|o| o == n).expect(n); assert!(pos("read_snapshot") < pos("write_heartbeat"), "snapshot fetched before heartbeat: {ops:?}");
    for delta in [250u64, 1_000, 5_000] { value.poll_snapshot_at(T0 + delta).expect("throttled poll"); assert_eq!(value.transport().heartbeats.len(), 1, "failing heartbeat retried at +{delta}ms"); }
    value.poll_snapshot_at(T0 + 6_000).expect("poll after throttle"); assert_eq!(value.transport().heartbeats, vec![("s1".to_string(), T0), ("s1".to_string(), T0 + 6_000)]); value.poll_snapshot_at(T0 + 6_250).expect("throttled poll"); assert_eq!(value.transport().heartbeats.len(), 2, "second failure also throttled"); let _ = std::fs::remove_dir_all(dir);
}

struct SlowIo {
    inner: LocalControlIo,
    clock: Arc<ManualClock>,
}
#[rustfmt::skip]
impl ControlIo for SlowIo { fn setup_dirs<'a>(&'a self, user: &'a str) -> ControlFuture<'a, ()> { self.inner.setup_dirs(user) } fn read_intents<'a>(&'a self) -> ControlFuture<'a, Vec<u8>> { Box::pin(async move { self.clock.advance(10_000); self.inner.read_intents().await }) } fn write_snapshot_atomic<'a>(&'a self, bytes: &'a [u8]) -> ControlFuture<'a, ()> { self.inner.write_snapshot_atomic(bytes) } fn load_processed<'a>(&'a self) -> ControlFuture<'a, Option<ProcessedState>> { self.inner.load_processed() } fn store_processed<'a>(&'a self, state: &'a ProcessedState) -> ControlFuture<'a, ()> { self.inner.store_processed(state) } }
#[rustfmt::skip]
async fn make_slow_runtime(name: &str, clock: Arc<ManualClock>) -> (PathBuf, ExpertRuntime) { let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts"); let io = SlowIo { inner: LocalControlIo::new(control.clone()), clock }; io.setup_dirs("pp-testuser").await.unwrap(); let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() }; let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher { hang: false }), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding::default()), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts, topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted }; (control, ExpertRuntime::new(spec, deps, T0).await.unwrap()) }

#[tokio::test] #[rustfmt::skip]
async fn expert_drain_heartbeat_uses_fresh_clock_after_long_drain() {
    let clock = Arc::new(ManualClock::new(T0 + 1_500)); let (control, mut runtime) = make_slow_runtime("hb-fresh", clock.clone()).await;
    write_hello(&control, "s1"); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); runtime.set_clock(clock); runtime.set_heartbeat_timeout(1_000);
    // Stale start T0+1500 is 500ms after last seen (inside 1s timeout); the slow read advances the fresh clock 10s, so only a fresh post-drain check disconnects.
    runtime.drain_intents(T0 + 1_500).await.unwrap(); assert!(runtime.is_disconnected(), "fresh post-drain heartbeat check must disconnect"); assert_eq!(runtime.gate_phase(), GatePhase::ReissueNeeded); assert!(runtime.orchestrator().session().events().iter().any(|e| e.kind == EventKind::CandidateDisconnected));
}
