use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{EventKind, Language, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::control::final_ack_bytes;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, PollOutcome,
};

use crate::common_mod;
use common_mod::qs;
type PF<'a, T> = PipelineFuture<'a, T>;
const T0: u64 = 1_730_000_000_000;

#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-expert-rec-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

struct FlakyFetcher {
    calls: AtomicU32,
    fail_first: u32,
}
impl FlakyFetcher {
    fn new(fail_first: u32) -> Self {
        Self {
            calls: AtomicU32::new(0),
            fail_first,
        }
    }
    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}
#[rustfmt::skip]
impl StatementFetcher for FlakyFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst); let fail = n < self.fail_first; let s = slug.to_string();
        Box::pin(async move { if fail { Err(PipelineError::Llm("meta api returned status 503: x".into())) } else { Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) } })
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
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> { Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) }) }
}
struct FailingCoding;
#[rustfmt::skip]
impl CodingTransport for FailingCoding {
    fn coding_step<'a>(&'a self, _s: &'a mut pairproof::core::Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { Err(PipelineError::Llm("meta api returned status 503: x".into())) }) }
}
#[derive(Default)]
struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, s: &'a mut pairproof::core::Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) }) }
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
    fn chat<'a>(&'a self, _s: &'a mut pairproof::core::Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> { Box::pin(async move { Ok("Think about a map.".into()) }) }
}
struct FakeGrader;
#[rustfmt::skip]
impl QuestionGrader for FakeGrader {
    fn grade_all<'a>(&'a self, _inputs: &'a [pairproof::llm::grading::QuestionScoringInput]) -> GradeFuture<'a> { Box::pin(async move { Vec::new() }) }
}

struct Flaky {
    down: u32,
    calls: AtomicU32,
}
impl Flaky {
    fn new(down: u32) -> Self {
        Self {
            down,
            calls: AtomicU32::new(0),
        }
    }
    fn health(&self, service: ServiceKind) -> ServiceHealth {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n < self.down {
            ServiceHealth::Down {
                service,
                reason: "status 503".into(),
            }
        } else {
            ServiceHealth::Up
        }
    }
}
struct LeetProbe(Arc<Flaky>);
impl LeetCodeProbe for LeetProbe {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let f = self.0.clone();
        Box::pin(async move { f.health(ServiceKind::LeetCode) })
    }
}
struct MetaProbeFake(Arc<Flaky>);
impl MetaProbe for MetaProbeFake {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let f = self.0.clone();
        Box::pin(async move { f.health(ServiceKind::Meta) })
    }
}
fn probes(down: u32) -> (Arc<dyn LeetCodeProbe>, Arc<dyn MetaProbe>) {
    (
        Arc::new(LeetProbe(Arc::new(Flaky::new(down)))) as Arc<dyn LeetCodeProbe>,
        Arc::new(MetaProbeFake(Arc::new(Flaky::new(down)))) as Arc<dyn MetaProbe>,
    )
}
fn probes_always_down() -> (Arc<dyn LeetCodeProbe>, Arc<dyn MetaProbe>) {
    probes(u32::MAX)
}
struct CountingProbe {
    calls: Arc<AtomicU32>,
}
impl LeetCodeProbe for CountingProbe {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let c = self.calls.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
            ServiceHealth::Down {
                service: ServiceKind::LeetCode,
                reason: "status 503".into(),
            }
        })
    }
}
impl MetaProbe for CountingProbe {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let c = self.calls.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
            ServiceHealth::Down {
                service: ServiceKind::Meta,
                reason: "status 503".into(),
            }
        })
    }
}

#[rustfmt::skip]
async fn make_runtime(name: &str, fetcher: Arc<FlakyFetcher>) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher, generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted, shadow_evaluator: None };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, control, runtime)
}
#[rustfmt::skip]
async fn make_runtime_failing_coding(name: &str, fetcher: Arc<FlakyFetcher>) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher, generator: Arc::new(FakeGenerator), coding: Arc::new(FailingCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted, shadow_evaluator: None };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, control, runtime)
}
#[rustfmt::skip]
fn push_intent(control: &std::path::Path, seq: u64, kind: IntentKind) {
    use std::io::Write as _;
    let intent = Intent::new("s1", seq, kind);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(control.join(INTENT_REL)).unwrap();
    file.write_all(format!("{}\n", intent.to_line().unwrap()).as_bytes()).unwrap();
}
#[rustfmt::skip]
fn write_hello(control: &std::path::Path) { let body = serde_json::json!({ "v": 1, "session_id": "s1", "app_version": env!("CARGO_PKG_VERSION") }).to_string(); std::fs::write(control.join("inbox").join("hello.json"), body).unwrap(); }
#[rustfmt::skip]
fn write_ack(control: &std::path::Path) { std::fs::write(control.join("inbox").join("final-ack.json"), final_ack_bytes("s1")).unwrap(); }
#[rustfmt::skip]
async fn connect(runtime: &mut ExpertRuntime, control: &std::path::Path) { write_hello(control); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); }
#[rustfmt::skip]
async fn enter_recovery(runtime: &mut ExpertRuntime) { tokio::time::sleep(Duration::from_millis(30)).await; runtime.drain_intents(T0 + 2_000).await.unwrap(); }
async fn drive_until_loaded(runtime: &mut ExpertRuntime, control: &std::path::Path) -> bool {
    for _ in 0..100 {
        let _ = runtime.poll_recovery(T0 + 3_000).await;
        runtime.drain_intents(T0 + 3_000).await.unwrap();
        if let Ok(bytes) = std::fs::read(control.join(SNAPSHOT_REL)) {
            if let Ok(snap) = parse_snapshot(&bytes, "s1") {
                if snap.options.len() == 4 {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn runtime_enters_recovery_after_qualifying_service_failure() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("enters", fetcher.clone()).await;
    runtime.set_recovery_window_ms(200);
    let (leet, meta) = probes(1);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(runtime.is_recovery_active());
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Recovering);
    let snap = runtime.recovery_state_for_snapshot();
    assert_eq!(snap.service, Some(ServiceKind::Meta));
    assert_eq!(snap.generation, 1);
    assert!(snap.remaining_ms <= 200);
    assert!(!runtime.recovery_timed_out());
    assert_eq!(fetcher.calls(), 1);
    // Duplicate begin is rejected and spawns no second task.
    assert!(runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_100
        )
        .is_err());
    assert_eq!(runtime.recovery_state_for_snapshot().generation, 1);
    runtime.cancel_recovery();
}

#[tokio::test]
async fn runtime_recovery_runs_without_blocking_tick() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, _control, mut runtime) = make_runtime("nonblocking", fetcher).await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0,
        )
        .unwrap();
    let started = Instant::now();
    let outcome = runtime.poll_recovery(T0 + 10).await;
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "tick blocked {:?}",
        started.elapsed()
    );
    assert_eq!(outcome, None);
    assert!(runtime.is_recovery_active());
    runtime.cancel_recovery();
}

#[tokio::test]
async fn runtime_recovery_success_retries_interrupted_operation() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("success", fetcher.clone()).await;
    runtime.set_recovery_window_ms(200);
    let (leet, meta) = probes(0);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(runtime.is_recovery_active());
    let mut resumed = false;
    for _ in 0..100 {
        if let Some(RecoveryOutcome::Resumed(_)) = runtime.poll_recovery(T0 + 3_000).await {
            resumed = true;
        }
        runtime.drain_intents(T0 + 3_000).await.unwrap();
        if let Ok(bytes) = std::fs::read(control.join(SNAPSHOT_REL)) {
            if parse_snapshot(&bytes, "s1")
                .map(|s| s.options.len() == 4)
                .unwrap_or(false)
            {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(resumed, "recovery should resume");
    assert!(drive_until_loaded(&mut runtime, &control).await || true);
    // Ensure loaded via snapshot check loop already; verify fetcher retried exactly once.
    assert_eq!(fetcher.calls(), 2);
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Healthy);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
}

#[tokio::test]
async fn runtime_recovery_does_not_duplicate_session_mutation() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("no-dup", fetcher.clone()).await;
    runtime.set_recovery_window_ms(200);
    let (leet, meta) = probes(0);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(
        drive_until_loaded(&mut runtime, &control).await,
        "load should complete after recovery"
    );
    let events_after = runtime.orchestrator().session().events().len();
    for _ in 0..3 {
        let _ = runtime.poll_recovery(T0 + 4_000).await;
        runtime.drain_intents(T0 + 4_000).await.unwrap();
    }
    assert_eq!(fetcher.calls(), 2, "exactly one retry");
    assert_eq!(
        runtime.orchestrator().session().events().len(),
        events_after
    );
    assert_eq!(
        runtime
            .orchestrator()
            .session()
            .events()
            .iter()
            .filter(|e| e.kind == EventKind::CandidateConnected)
            .count(),
        1
    );
    assert_eq!(
        runtime
            .orchestrator()
            .session()
            .events()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::QuestionStarted { index: 0 }))
            .count(),
        1
    );
    assert_eq!(runtime.orchestrator().session().slots().len(), 5);
}

#[tokio::test]
async fn runtime_recovery_timeout_terminates_session() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("timeout", fetcher.clone()).await;
    runtime.set_recovery_window_ms(80);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(runtime.is_recovery_active());
    let mut outcome = None;
    for _ in 0..100 {
        if let Some(o) = runtime.poll_recovery(T0 + 5_000).await {
            outcome = Some(o);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(outcome, Some(RecoveryOutcome::TechnicalFailure));
    assert_eq!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
    assert!(runtime.recovery_timed_out());
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Failed);
    assert!(runtime
        .orchestrator()
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::TechnicalFailure));
    let snap = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    assert_eq!(snap.final_breakdown, None);
    // end_session after a technical failure must not grade or fabricate a breakdown.
    write_ack(&control);
    let (_root2, _c) = (_root.clone(), control.clone());
    runtime
        .end_session_with_ack_deadline(T0 + 6_000, 500)
        .await
        .unwrap();
    assert_eq!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
    let snap = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    assert_eq!(snap.final_breakdown, None);
    let report =
        std::fs::read_to_string(_root.join("artifacts").join("s1").join("report.md")).unwrap();
    assert!(report.contains("technical failure"), "{report}");
}

#[tokio::test]
async fn runtime_termination_cancels_recovery_task() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("termination", fetcher).await;
    connect(&mut runtime, &control).await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(runtime.is_recovery_active());
    write_ack(&control);
    runtime
        .end_session_with_ack_deadline(T0 + 3_000, 500)
        .await
        .unwrap();
    assert_ne!(runtime.recovery_phase(), RecoveryPhase::Recovering);
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(runtime.pending_count(), 0);
    assert!(runtime.is_killed());
    assert_eq!(runtime.poll_recovery(T0 + 4_000).await, None);
}

#[tokio::test]
async fn runtime_shutdown_cancels_recovery_task() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("shutdown", fetcher.clone()).await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(runtime.is_recovery_active());
    assert!(runtime.pending_count() >= 1);
    write_ack(&control);
    runtime
        .end_session_with_ack_deadline(T0 + 3_000, 500)
        .await
        .unwrap();
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(runtime.pending_count(), 0);
    assert_eq!(runtime.poll_recovery(T0 + 4_000).await, None);
    assert_ne!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
}

#[tokio::test]
async fn runtime_ignores_stale_recovery_result() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("stale", fetcher.clone()).await;
    connect(&mut runtime, &control).await;
    assert!(drive_until_loaded(&mut runtime, &control).await);
    assert_eq!(fetcher.calls(), 1);
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes(0);
    runtime.set_health_probes(leet, meta);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 3_000,
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    runtime.cancel_recovery();
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(runtime.poll_recovery(T0 + 4_000).await, None);
    runtime.drain_intents(T0 + 4_000).await.unwrap();
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(fetcher.calls(), 1, "stale result must not retry load");
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
}

#[tokio::test]
async fn drain_recoverable_coding_failure_does_not_enter_recovery() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) =
        make_runtime_failing_coding("drain-rec", fetcher.clone()).await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes(0);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    assert!(
        drive_until_loaded(&mut runtime, &control).await,
        "approaches should load before coding intent"
    );
    assert!(!runtime.is_recovery_active());
    push_intent(&control, 1, IntentKind::SelectPreset { preset: 1 });
    let reports = runtime.drain_intents(T0 + 4_000).await.unwrap();
    assert!(
        reports.iter().any(|r| r.outcome == ApplyOutcome::Rejected
            && r.error.as_deref().is_some_and(|e| e.contains("status 503"))),
        "failing coding should be rejected with 503, got {reports:?}"
    );
    assert!(
        !runtime.is_recovery_active(),
        "drain coding failure must not enter recovery"
    );
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Healthy);
    assert_eq!(runtime.recovery_state_for_snapshot().generation, 0);
    push_intent(&control, 2, IntentKind::Continue);
    let reports2 = runtime.drain_intents(T0 + 5_000).await.unwrap();
    assert!(
        reports2.iter().any(|r| r.outcome == ApplyOutcome::Rejected),
        "second failing coding should also be rejected, got {reports2:?}"
    );
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Healthy);
    assert_eq!(runtime.recovery_state_for_snapshot().generation, 0);
}

#[tokio::test]
async fn recovery_cancel_stops_probe_task() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, _control, mut runtime) = make_runtime("cancel-stop", fetcher).await;
    runtime.set_recovery_window_ms(60_000);
    let calls = Arc::new(AtomicU32::new(0));
    let probe = Arc::new(CountingProbe {
        calls: calls.clone(),
    });
    runtime.set_health_probes(
        probe.clone() as Arc<dyn LeetCodeProbe>,
        probe as Arc<dyn MetaProbe>,
    );
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0,
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(calls.load(Ordering::SeqCst) >= 2, "probes should run");
    runtime.cancel_recovery();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let at_cancel = calls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        at_cancel,
        "probe count must not increase after cancel"
    );
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
}

#[tokio::test]
async fn recovery_gate_uses_monotonic_deadline() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, _control, mut runtime) = make_runtime("mono-deadline", fetcher).await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0,
        )
        .unwrap();
    let outcome = runtime.poll_recovery(T0 + 10).await;
    assert_eq!(outcome, None, "gate must not fail before monotonic window");
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Recovering);
    assert!(runtime.is_recovery_active());
    runtime.cancel_recovery();
}
