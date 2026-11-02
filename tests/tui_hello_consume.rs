use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{EventKind, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::app::{AppEvent, AppScreen, ExpertApp};
use pairproof::tui::expert_runtime::{
    ConsumeFuture, ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, HelloFuture, HelloSource,
    LocalHelloSource, PollOutcome,
};

type PF<'a, T> = PipelineFuture<'a, T>;
const T0: u64 = 1_730_000_000_000;
#[rustfmt::skip]
fn q(s: &str, d: Difficulty) -> QuestionRef { QuestionRef { title_slug: s.into(), title: s.into(), difficulty: d, acceptance_pct: None } }
#[rustfmt::skip]
fn qs(p: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> { (0..n).map(|i| q(&format!("{p}{i}"), d)).collect() }
#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pairproof-hello-consume-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d
}

struct FakeFetcher;
#[rustfmt::skip]
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let s = slug.to_string();
        Box::pin(async move { Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) })
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
struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> {
        Box::pin(async move { s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) })
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

/// Counts consumes and fails them on demand; reads delegate locally.
struct FailingHelloSource {
    inner: LocalHelloSource,
    fail: Arc<AtomicBool>,
    consumes: Arc<AtomicU32>,
}
#[rustfmt::skip]
impl HelloSource for FailingHelloSource {
    fn read_hello<'a>(&'a self) -> HelloFuture<'a> { self.inner.read_hello() }
    fn consume_hello<'a>(&'a self) -> ConsumeFuture<'a> { Box::pin(async move { self.consumes.fetch_add(1, Ordering::SeqCst); if self.fail.load(Ordering::SeqCst) { return Err("hello consume failed".to_string()); } self.inner.consume_hello().await }) }
}

#[rustfmt::skip]
async fn build(control: PathBuf, artifacts: PathBuf, hello: Box<dyn HelloSource>) -> ExpertRuntime {
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello, fetcher: Arc::new(FakeFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted };
    ExpertRuntime::new(spec, deps, T0).await.unwrap()
}
#[rustfmt::skip]
async fn make_runtime(name: &str) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control");
    let rt = build(control.clone(), root.join("artifacts"), Box::new(LocalHelloSource::new(control.clone()))).await; (root, control, rt)
}
#[rustfmt::skip]
async fn make_failing_runtime(name: &str, fail: Arc<AtomicBool>, consumes: Arc<AtomicU32>) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control");
    let hello = FailingHelloSource { inner: LocalHelloSource::new(control.clone()), fail, consumes };
    let rt = build(control.clone(), root.join("artifacts"), Box::new(hello)).await; (root, control, rt)
}

#[rustfmt::skip] fn write_hello(control: &Path, session_id: &str) { let body = serde_json::json!({ "v": 1, "session_id": session_id, "app_version": env!("CARGO_PKG_VERSION") }).to_string(); std::fs::write(control.join("inbox").join("hello.json"), body).unwrap(); }
#[rustfmt::skip] fn hello_exists(control: &Path) -> bool { control.join("inbox").join("hello.json").exists() }
#[rustfmt::skip] fn connects(rt: &ExpertRuntime) -> usize { rt.orchestrator().session().events().iter().filter(|e| e.kind == EventKind::CandidateConnected).count() }
#[rustfmt::skip] fn reconnects(rt: &ExpertRuntime) -> usize { rt.orchestrator().session().events().iter().filter(|e| matches!(e.kind, EventKind::CandidateReconnected { .. })).count() }
#[rustfmt::skip]
async fn disconnect(rt: &mut ExpertRuntime) { rt.set_heartbeat_timeout(1_000); rt.drain_intents(T0 + 2_500).await.unwrap(); assert!(rt.is_disconnected()); }

#[tokio::test]
#[rustfmt::skip]
async fn hello_consumed_on_first_accept() {
    let consumes = Arc::new(AtomicU32::new(0));
    let (_root, control, mut rt) = make_failing_runtime("first", Arc::new(AtomicBool::new(false)), consumes.clone()).await;
    assert!(!hello_exists(&control));
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    assert!(!hello_exists(&control), "first accept must consume the hello");
    assert_eq!(consumes.load(Ordering::SeqCst), 1, "first connect consumes exactly once");
    assert_eq!(rt.poll_candidate(T0 + 2_000).await, PollOutcome::AlreadyConnected);
    assert_eq!(consumes.load(Ordering::SeqCst), 1, "short-circuit must not consume again");
}

#[tokio::test]
#[rustfmt::skip]
async fn stale_hello_never_reaccepted() {
    let (_root, control, mut rt) = make_runtime("stale").await;
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    disconnect(&mut rt).await;
    assert_eq!(rt.poll_candidate(T0 + 2_600).await, PollOutcome::Waiting);
    assert!(rt.is_disconnected());
    assert_eq!(connects(&rt), 1); assert_eq!(reconnects(&rt), 0);
}

#[tokio::test]
#[rustfmt::skip]
async fn reissue_survives_tick_after_consume() {
    let (_root, control, mut rt) = make_runtime("reissue-survives").await;
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    disconnect(&mut rt).await;
    let fresh = rt.reissue(T0 + 3_000).await.expect("reissue");
    assert_eq!(rt.poll_candidate(T0 + 3_500).await, PollOutcome::Waiting);
    assert!(rt.is_disconnected());
    assert_eq!(rt.reissue_string().map(|s| s.encode()), Some(fresh.encode()), "tick must not wipe the reissue");
}

#[tokio::test]
#[rustfmt::skip]
async fn legit_reconnect_consumes_fresh_hello() {
    let (_root, control, mut rt) = make_runtime("legit").await;
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    disconnect(&mut rt).await;
    rt.reissue(T0 + 3_000).await.expect("reissue");
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 3_500).await, PollOutcome::Connected);
    assert!(!hello_exists(&control), "re-accept must consume the fresh hello");
    assert!(rt.reissue_string().is_none());
    assert!(!rt.is_disconnected());
}

#[tokio::test]
#[rustfmt::skip]
async fn consume_failure_blocks_reaccept() {
    let fail = Arc::new(AtomicBool::new(true)); let consumes = Arc::new(AtomicU32::new(0));
    let (_root, control, mut rt) = make_failing_runtime("wedge", fail.clone(), consumes.clone()).await;
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected, "accept stands despite consume failure");
    assert!(hello_exists(&control), "failed consume must leave the stale file");
    disconnect(&mut rt).await;
    assert_eq!(rt.poll_candidate(T0 + 2_600).await, PollOutcome::Waiting, "pending consume failure blocks re-accept");
    assert!(rt.is_disconnected());
    assert_eq!(connects(&rt), 1); assert_eq!(reconnects(&rt), 0);
    assert_eq!(consumes.load(Ordering::SeqCst), 2);
    fail.store(false, Ordering::SeqCst);
    assert_eq!(rt.poll_candidate(T0 + 2_700).await, PollOutcome::Waiting, "healed retry consumes the stale file, then reads fresh");
    assert!(!hello_exists(&control));
    assert!(rt.is_disconnected());
    rt.reissue(T0 + 3_000).await.expect("reissue");
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 3_500).await, PollOutcome::Connected);
    assert!(!hello_exists(&control)); assert!(rt.reissue_string().is_none());
}

#[test]
#[rustfmt::skip]
fn disconnected_flag_clears_on_reconnect() {
    let mut app = ExpertApp::new();
    assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    assert_eq!(app.handle(AppEvent::DisclosureAcknowledged), Ok(AppScreen::Preflight));
    assert_eq!(app.handle(AppEvent::PreflightCompleted), Ok(AppScreen::Setup));
    assert_eq!(app.handle(AppEvent::DisclosureAcknowledged), Ok(AppScreen::Setup));
    app.set_has_proposal(true);
    assert_eq!(app.handle(AppEvent::ProposalReady), Ok(AppScreen::Proposal));
    assert_eq!(app.handle(AppEvent::ProposalConfirmed), Ok(AppScreen::ConnectionWait));
    app.set_connected(true);
    assert_eq!(app.handle(AppEvent::ConnectionLocked), Ok(AppScreen::SelectMedium));
    assert_eq!(app.handle(AppEvent::MediumSelected), Ok(AppScreen::SelectHard));
    app.set_expert_selection_done(true);
    assert_eq!(app.handle(AppEvent::ExpertSelectionDone), Ok(AppScreen::Monitor));
    assert!(!app.is_disconnected());
    assert_eq!(app.handle(AppEvent::AccidentalDisconnect), Ok(AppScreen::Monitor));
    assert!(app.is_disconnected());
    assert_eq!(app.handle(AppEvent::CandidateReconnected), Ok(AppScreen::Monitor));
    assert!(!app.is_disconnected());
    assert_eq!(app.handle(AppEvent::CandidateReconnected), Ok(AppScreen::Monitor));
    assert!(!app.is_disconnected());
    let mut fresh = ExpertApp::new();
    assert_eq!(fresh.handle(AppEvent::CandidateReconnected).unwrap_err().to_string(), "invalid transition from RoleSelect on event CandidateReconnected");
}

#[tokio::test]
#[rustfmt::skip]
async fn double_accept_without_rewrite_impossible() {
    let (_root, control, mut rt) = make_runtime("double").await;
    write_hello(&control, "s1");
    assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    assert_eq!(rt.poll_candidate(T0 + 2_000).await, PollOutcome::AlreadyConnected);
    assert_eq!(connects(&rt), 1); assert_eq!(reconnects(&rt), 0);
    assert!(!hello_exists(&control));
}
