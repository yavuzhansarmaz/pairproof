use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::control::final_ack_bytes;
use pairproof::orchestration::secret_push::{DockerSecretPush, SecretPush};
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, SandboxHandle, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
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
    let d = std::env::temp_dir().join(format!("pairproof-secret-refresh-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d
}

/// Deterministic pusher: records pushes, fails the first `failures_left` pushes.
struct FakePusher {
    pushed: Mutex<Vec<Secret>>,
    failures_left: AtomicUsize,
}
#[rustfmt::skip]
impl FakePusher {
    fn new() -> Self { Self { pushed: Mutex::new(Vec::new()), failures_left: AtomicUsize::new(0) } }
    fn failing(n: usize) -> Self { Self { pushed: Mutex::new(Vec::new()), failures_left: AtomicUsize::new(n) } }
    fn push_count(&self) -> usize { self.pushed.lock().unwrap().len() }
    fn last(&self) -> Option<Secret> { self.pushed.lock().unwrap().last().cloned() }
}
#[rustfmt::skip]
impl SecretPush for FakePusher {
    fn push<'a>(&'a self, secret: &'a Secret) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        let secret = secret.clone();
        Box::pin(async move {
            if self.failures_left.load(Ordering::SeqCst) > 0 { self.failures_left.fetch_sub(1, Ordering::SeqCst); return Err("fake push failed".into()); }
            self.pushed.lock().unwrap().push(secret); Ok(())
        })
    }
}

struct FakeFetcher;
#[rustfmt::skip]
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> { let s = slug.to_string(); Box::pin(async move { Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) }) }
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
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> { Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) }) }
}
#[derive(Default)]
struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) }) }
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
    fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> { Box::pin(async move { Ok("Think about a map.".into()) }) }
}
struct FakeGrader;
#[rustfmt::skip]
impl QuestionGrader for FakeGrader {
    fn grade_all<'a>(&'a self, _inputs: &'a [pairproof::llm::grading::QuestionScoringInput]) -> GradeFuture<'a> { Box::pin(async move { Vec::new() }) }
}

#[rustfmt::skip]
async fn make_runtime(name: &str) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, control, runtime)
}

#[rustfmt::skip] fn write_hello(control: &Path, session_id: &str) { let body = serde_json::json!({ "v": 1, "session_id": session_id, "app_version": env!("CARGO_PKG_VERSION") }).to_string(); std::fs::write(control.join("inbox").join("hello.json"), body).unwrap(); }
#[rustfmt::skip] fn write_ack(control: &Path, session_id: &str) { std::fs::write(control.join("inbox").join("final-ack.json"), final_ack_bytes(session_id)).unwrap(); }
#[rustfmt::skip] fn arm(rt: &mut ExpertRuntime) -> Arc<FakePusher> { let p = Arc::new(FakePusher::new()); rt.set_secret_pusher(p.clone() as Arc<dyn SecretPush>); p }
#[rustfmt::skip] fn arm_failing(rt: &mut ExpertRuntime, n: usize) -> Arc<FakePusher> { let p = Arc::new(FakePusher::failing(n)); rt.set_secret_pusher(p.clone() as Arc<dyn SecretPush>); p }
#[rustfmt::skip]
async fn connect(rt: &mut ExpertRuntime, control: &Path) { write_hello(control, "s1"); assert_eq!(rt.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); }
#[rustfmt::skip]
async fn disconnect(rt: &mut ExpertRuntime) { rt.set_heartbeat_timeout(1_000); rt.drain_intents(T0 + 2_500).await.unwrap(); assert!(rt.is_disconnected()); }

#[tokio::test]
#[rustfmt::skip]
async fn refresh_pushes_on_window_elapse_pre_lock() {
    let (_root, _control, mut rt) = make_runtime("push-on-elapse").await;
    let pusher = arm(&mut rt);
    assert!(rt.refresh_pre_lock_secret(T0 + 1_000).await.unwrap());
    assert_eq!(pusher.push_count(), 1);
    let first = rt.current_string(T0 + 1_000).unwrap();
    assert_eq!(pusher.last().as_ref(), Some(first.secret()));
    assert!(rt.refresh_pre_lock_secret(T0 + 61_000).await.unwrap());
    assert_eq!(pusher.push_count(), 2);
    let second = rt.current_string(T0 + 61_000).unwrap();
    assert_eq!(pusher.last().as_ref(), Some(second.secret()));
    assert_ne!(first.secret(), second.secret());
}

#[tokio::test]
#[rustfmt::skip]
async fn refresh_noop_when_window_fresh() {
    let (_root, _control, mut rt) = make_runtime("noop-fresh").await;
    let pusher = arm(&mut rt);
    assert!(rt.refresh_pre_lock_secret(T0 + 1_000).await.unwrap());
    assert!(!rt.refresh_pre_lock_secret(T0 + 2_000).await.unwrap());
    assert!(!rt.refresh_pre_lock_secret(T0 + 3_000).await.unwrap());
    assert_eq!(pusher.push_count(), 1);
}

#[tokio::test]
#[rustfmt::skip]
async fn refresh_noop_after_connect() {
    let (_root, control, mut rt) = make_runtime("noop-connected").await;
    let pusher = arm(&mut rt);
    connect(&mut rt, &control).await;
    assert!(!rt.refresh_pre_lock_secret(T0 + 2_000).await.unwrap());
    assert!(!rt.refresh_pre_lock_secret(T0 + 70_000).await.unwrap());
    assert_eq!(pusher.push_count(), 0);
}

#[tokio::test]
#[rustfmt::skip]
async fn refresh_noop_after_end() {
    let (_root, control, mut rt) = make_runtime("noop-ended").await;
    let pusher = arm(&mut rt);
    connect(&mut rt, &control).await;
    write_ack(&control, "s1");
    rt.end_session_with_ack_deadline(T0 + 2_000, 5_000).await.unwrap();
    assert_eq!(rt.gate_phase(), GatePhase::Ended);
    assert!(!rt.refresh_pre_lock_secret(T0 + 70_000).await.unwrap());
    assert_eq!(pusher.push_count(), 0);
}

#[tokio::test]
#[rustfmt::skip]
async fn refresh_noop_without_pusher() {
    let (_root, _control, mut rt) = make_runtime("noop-no-pusher").await;
    assert!(!rt.refresh_pre_lock_secret(T0 + 70_000).await.unwrap());
    assert!(rt.current_string(T0 + 70_000).is_ok());
}

#[tokio::test]
#[rustfmt::skip]
async fn refresh_push_failure_retries_next_tick() {
    let (_root, _control, mut rt) = make_runtime("failure-retry").await;
    let pusher = arm_failing(&mut rt, 1);
    let err = rt.refresh_pre_lock_secret(T0 + 1_000).await.unwrap_err();
    assert_eq!(err.to_string(), "secret refresh push failed");
    assert_eq!(pusher.push_count(), 0);
    let display = rt.current_string(T0 + 1_000).unwrap();
    assert!(!err.to_string().contains(&display.secret().expose_hex()));
    assert!(rt.refresh_pre_lock_secret(T0 + 2_000).await.unwrap());
    assert_eq!(pusher.push_count(), 1);
    assert_eq!(pusher.last().as_ref(), Some(display.secret()));
}

#[tokio::test]
#[rustfmt::skip]
async fn refresh_covers_disconnected_monitor() {
    let (_root, control, mut rt) = make_runtime("disconnected-monitor").await;
    let pusher = arm(&mut rt);
    connect(&mut rt, &control).await;
    disconnect(&mut rt).await;
    let fresh = rt.reissue(T0 + 3_000).await.unwrap();
    assert!(rt.is_disconnected());
    assert!(rt.refresh_pre_lock_secret(T0 + 64_000).await.unwrap());
    assert_eq!(pusher.push_count(), 1);
    assert!(rt.is_disconnected());
    let display = rt.current_string(T0 + 64_000).unwrap();
    assert_eq!(pusher.last().as_ref(), Some(display.secret()));
    assert_ne!(fresh.secret(), display.secret());
}

#[tokio::test]
#[rustfmt::skip]
async fn reissue_then_refresh_stays_aligned() {
    let (_root, control, mut rt) = make_runtime("reissue-aligned").await;
    let pusher = arm(&mut rt);
    connect(&mut rt, &control).await;
    disconnect(&mut rt).await;
    rt.reissue(T0 + 3_000).await.unwrap();
    assert!(!rt.refresh_pre_lock_secret(T0 + 3_500).await.unwrap());
    assert_eq!(pusher.push_count(), 0);
    assert!(rt.refresh_pre_lock_secret(T0 + 64_000).await.unwrap());
    assert_eq!(pusher.push_count(), 1);
}

#[tokio::test]
#[rustfmt::skip]
async fn pusher_debug_leaks_no_secret() {
    let endpoint = Endpoint::new("203.0.113.10", 22022).unwrap();
    let handle = SandboxHandle::ids("s1", &endpoint).unwrap();
    let pusher = DockerSecretPush::new(handle, EphemeralUsername::parse("pp-a1b2c3d4").unwrap());
    let debug = format!("{pusher:?}");
    assert_eq!(debug, "DockerSecretPush { .. }");
    let (_root, _control, mut rt) = make_runtime("debug-no-leak").await;
    let display = rt.current_string(T0 + 1_000).unwrap();
    assert!(!debug.contains(&display.secret().expose_hex()));
    let fixed = Secret::from_bytes([0x5a; 16]);
    assert!(!debug.contains(&fixed.expose_hex()));
}
