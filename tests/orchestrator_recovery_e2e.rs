use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{EventKind, Language, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::control::final_ack_bytes;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_runtime::{
    CandidateRuntime, CandidateRuntimeError, CandidateTransport,
};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, PollOutcome,
};

use crate::common_mod;
use common_mod::qs;
type PF<'a, T> = PipelineFuture<'a, T>;
const T0: u64 = 1_730_000_000_000;

#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-orch-rec-e2e-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

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

// Controllable probes for the disconnect race: one shared up flag.
struct BoolState {
    up: AtomicBool,
}
struct LeetBool(Arc<BoolState>);
impl LeetCodeProbe for LeetBool {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let f = self.0.clone();
        Box::pin(async move {
            if f.up.load(Ordering::SeqCst) {
                ServiceHealth::Up
            } else {
                ServiceHealth::Down {
                    service: ServiceKind::LeetCode,
                    reason: "status 503".into(),
                }
            }
        })
    }
}
struct MetaBool(Arc<BoolState>);
impl MetaProbe for MetaBool {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let f = self.0.clone();
        Box::pin(async move {
            if f.up.load(Ordering::SeqCst) {
                ServiceHealth::Up
            } else {
                ServiceHealth::Down {
                    service: ServiceKind::Meta,
                    reason: "status 503".into(),
                }
            }
        })
    }
}
fn bool_probes(up: bool) -> (Arc<BoolState>, Arc<dyn LeetCodeProbe>, Arc<dyn MetaProbe>) {
    let state = Arc::new(BoolState {
        up: AtomicBool::new(up),
    });
    (
        state.clone(),
        Arc::new(LeetBool(state.clone())) as Arc<dyn LeetCodeProbe>,
        Arc::new(MetaBool(state)) as Arc<dyn MetaProbe>,
    )
}

#[derive(Default)]
struct FakeTransport {
    snapshots: VecDeque<Option<CandidateSnapshot>>,
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
    fn start_sync(&mut self, _s: &str, _a: &std::path::Path) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn send_intent(&mut self, _i: &Intent) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn read_snapshot(&mut self) -> Result<Option<CandidateSnapshot>, CandidateRuntimeError> {
        Ok(self.snapshots.pop_front().unwrap_or(None))
    }
    fn flush_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
    fn terminate_sync(&mut self, _s: &str) -> Result<(), CandidateRuntimeError> {
        Ok(())
    }
}
#[rustfmt::skip]
fn connection() -> ConnectionString { ConnectionString::new(Endpoint::new("127.0.0.1", 2222).unwrap(), EphemeralUsername::parse("pp-abcd1234").unwrap(), Secret::from_bytes([0xab; 16]), 4_102_444_800_000, Fingerprint::parse(&"a".repeat(64)).unwrap()) }
fn connected_candidate(
    name: &str,
    snaps: Vec<CandidateSnapshot>,
) -> (PathBuf, CandidateRuntime<FakeTransport>) {
    let dir = tmp(name);
    let fake = FakeTransport {
        snapshots: snaps.into_iter().map(Some).collect(),
    };
    let mut rt = CandidateRuntime::new("s1", dir.clone(), fake);
    rt.connect(&connection()).expect("connect");
    (dir, rt)
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
fn write_hello(control: &std::path::Path) { let body = serde_json::json!({ "v": 1, "session_id": "s1", "app_version": env!("CARGO_PKG_VERSION") }).to_string(); std::fs::write(control.join("inbox").join("hello.json"), body).unwrap(); }
#[rustfmt::skip]
fn write_ack(control: &std::path::Path) { std::fs::write(control.join("inbox").join("final-ack.json"), final_ack_bytes("s1")).unwrap(); }
#[rustfmt::skip]
async fn connect(runtime: &mut ExpertRuntime, control: &std::path::Path) { write_hello(control); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); }
#[rustfmt::skip]
async fn enter_recovery(runtime: &mut ExpertRuntime) { tokio::time::sleep(Duration::from_millis(30)).await; runtime.drain_intents(T0 + 2_000).await.unwrap(); }
#[rustfmt::skip]
fn read_snapshot(control: &std::path::Path) -> CandidateSnapshot { parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap() }
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
async fn e2e_recovery_success_resumes_original_operation() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("e2e-success", fetcher.clone()).await;
    runtime.set_recovery_window_ms(2_000);
    let (leet, meta) = probes(0);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(
        runtime.is_recovery_active(),
        "load failure should enter recovery"
    );
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Recovering);
    assert_eq!(fetcher.calls(), 1);
    assert!(runtime.timer_paused());
    // Timer is frozen while recovering.
    assert_eq!(
        runtime.adjusted_now_ms(T0 + 5_000),
        runtime.adjusted_now_ms(T0 + 9_000)
    );
    let events_before = runtime.orchestrator().session().events().len();
    let slots_before = runtime.orchestrator().session().slots().len();
    let mut resumed = false;
    for _ in 0..100 {
        if let Some(RecoveryOutcome::Resumed(_)) = runtime.poll_recovery(T0 + 3_000).await {
            resumed = true;
        }
        runtime.drain_intents(T0 + 3_000).await.unwrap();
        if let Ok(Ok(s)) =
            std::fs::read(control.join(SNAPSHOT_REL)).map(|b| parse_snapshot(&b, "s1"))
        {
            if s.options.len() == 4 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(resumed, "recovery should resume");
    assert!(
        drive_until_loaded(&mut runtime, &control).await
            || read_snapshot(&control).options.len() == 4
    );
    assert_eq!(fetcher.calls(), 2, "exactly one retry of the original load");
    // Exactly-once: no duplicate session mutation from the retry.
    assert_eq!(
        runtime.orchestrator().session().events().len(),
        events_before
    );
    assert_eq!(runtime.orchestrator().session().slots().len(), slots_before);
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
    // Extra polls/drains after success must not retry again.
    for _ in 0..3 {
        let _ = runtime.poll_recovery(T0 + 4_000).await;
        runtime.drain_intents(T0 + 4_000).await.unwrap();
    }
    assert_eq!(fetcher.calls(), 2);
    assert_eq!(
        runtime.orchestrator().session().events().len(),
        events_before
    );
    // Timer resumed and advances again.
    assert!(!runtime.timer_paused());
    assert!(runtime.adjusted_now_ms(T0 + 10_000) > runtime.adjusted_now_ms(T0 + 4_000));
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Healthy);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
    // Published candidate snapshot: statement + options, no active recovery.
    let snap = read_snapshot(&control);
    assert_eq!(snap.phase, "InProgress");
    assert_eq!(snap.options.len(), 4);
    assert!(
        snap.current.as_ref().is_some_and(|c| c
            .statement
            .as_deref()
            .is_some_and(|s| s.contains("Statement for e0"))),
        "{snap:?}"
    );
    assert!(
        snap.recovery.is_none()
            || snap
                .recovery
                .as_ref()
                .is_some_and(|r| !r.active && !r.technical_failure),
        "{snap:?}"
    );
    // Candidate-side consumption of the same published snapshot.
    let (dir, mut candidate) = connected_candidate("e2e-success-candidate", vec![snap]);
    candidate.poll_snapshot_at(10_000).unwrap();
    let view = candidate.view();
    assert_eq!(view.phase, "InProgress");
    assert_eq!(view.options.len(), 4);
    assert!(view
        .statement
        .as_deref()
        .is_some_and(|s| s.contains("Statement for e0")));
    assert!(view.recovery.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn e2e_recovery_timeout_technical_failure_no_score_cleanup_once() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("e2e-timeout", fetcher.clone()).await;
    runtime.set_recovery_window_ms(80);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    enter_recovery(&mut runtime).await;
    assert!(runtime.is_recovery_active());
    let slots_before = runtime.orchestrator().session().slots().len();
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
    let snap = read_snapshot(&control);
    assert_eq!(snap.phase, "TechnicalFailure");
    assert!(
        snap.recovery
            .as_ref()
            .is_some_and(|r| r.technical_failure && !r.active),
        "{snap:?}"
    );
    assert_eq!(snap.final_breakdown, None);
    // Candidate view of the failure snapshot.
    let (dir, mut candidate) = connected_candidate("e2e-timeout-candidate", vec![snap.clone()]);
    candidate.poll_snapshot_at(10_000).unwrap();
    assert!(candidate
        .view()
        .lines()
        .join("\n")
        .contains("technical failure"));
    let _ = std::fs::remove_dir_all(dir);
    // Progress preserved across expiry.
    let events_before_end = runtime.orchestrator().session().events().len();
    assert_eq!(runtime.orchestrator().session().slots().len(), slots_before);
    assert_eq!(
        runtime
            .orchestrator()
            .session()
            .events()
            .iter()
            .filter(|e| e.kind == EventKind::TechnicalFailure)
            .count(),
        1
    );
    write_ack(&control);
    let first = runtime
        .end_session_with_ack_deadline(T0 + 6_000, 500)
        .await
        .unwrap();
    assert!(
        !first.attempted.is_empty(),
        "cleanup must be attempted: {first:?}"
    );
    assert_eq!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
    let snap_after = read_snapshot(&control);
    assert_eq!(snap_after.phase, "TechnicalFailure");
    assert_eq!(snap_after.final_breakdown, None, "no fabricated score");
    assert_eq!(
        runtime.orchestrator().session().slots().len(),
        slots_before,
        "slots preserved after cleanup"
    );
    assert_eq!(
        runtime.orchestrator().session().events().len(),
        events_before_end,
        "events preserved after cleanup"
    );
    // Second end: no duplicate cleanup. ExpertRuntime returns the cached
    // report (re-attempts nothing); CleanupSequence itself would report
    // already_done. Accept either signal, but never a doubled attempt list.
    let second = runtime
        .end_session_with_ack_deadline(T0 + 7_000, 500)
        .await
        .unwrap();
    assert!(
        second.already_done || second.attempted == first.attempted,
        "second end must not duplicate cleanup: first={first:?} second={second:?}"
    );
    assert_eq!(second.attempted.len(), first.attempted.len());
    if let Some(report) = runtime.cleanup_report() {
        assert!(
            report.already_done || report.attempted == first.attempted,
            "{report:?}"
        );
    }
    assert_eq!(runtime.orchestrator().session().slots().len(), slots_before);
    assert_eq!(
        runtime.orchestrator().session().events().len(),
        events_before_end
    );
}

#[tokio::test]
async fn race_expert_end_during_recovery() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("race-end", fetcher.clone()).await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
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
    assert_ne!(runtime.orchestrator().status(), SessionStatus::Completed);
    assert_eq!(
        runtime.orchestrator().status(),
        SessionStatus::EndedByExpert
    );
    assert!(!runtime.is_recovery_active());
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(runtime.pending_count(), 0);
    assert_eq!(runtime.poll_recovery(T0 + 4_000).await, None);
}

#[tokio::test]
async fn race_candidate_disconnect_during_recovery() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("race-disconnect", fetcher.clone()).await;
    runtime.set_recovery_window_ms(5_000);
    // Probes up: the recovery task queues Resumed before the disconnect is
    // observed, so the poll after disconnect must still complete recovery.
    let (_state, leet, meta) = bool_probes(true);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(runtime.is_recovery_active());
    // Candidate disconnects during recovery (heartbeat timeout path, the
    // harness-supported disconnect; ExpertRuntime exposes no orch mut).
    runtime.set_heartbeat_timeout(1_000);
    let disconnected = runtime.check_heartbeat(T0 + 10_000).await;
    assert!(disconnected, "heartbeat timeout should disconnect");
    assert!(runtime.is_disconnected());
    assert!(!runtime.orchestrator().is_connected());
    assert_eq!(
        runtime.orchestrator().status(),
        SessionStatus::InProgress,
        "disconnect must not terminate the session"
    );
    assert!(!runtime.orchestrator().status().is_terminal());
    assert!(
        runtime.is_recovery_active(),
        "disconnect must not cancel recovery"
    );
    // Recovery still completable after the disconnect.
    let mut resumed = false;
    for _ in 0..100 {
        if let Some(RecoveryOutcome::Resumed(_)) = runtime.poll_recovery(T0 + 11_000).await {
            resumed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        resumed,
        "recovery should complete after candidate disconnect"
    );
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
    assert_ne!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
    runtime.drain_intents(T0 + 11_000).await.unwrap();
    let snap = read_snapshot(&control);
    assert_eq!(snap.phase, "InProgress");
    assert!(snap.recovery.is_none() || snap.recovery.as_ref().is_some_and(|r| !r.active));
}

#[tokio::test]
async fn race_duplicate_recovery_trigger() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("race-dup-trigger", fetcher.clone()).await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    // Let the initial load settle so pending_count reflects only recovery.
    assert!(drive_until_loaded(&mut runtime, &control).await);
    assert_eq!(runtime.pending_count(), 0);
    let gen = runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert_eq!(runtime.recovery_state_for_snapshot().generation, gen);
    assert!(
        runtime.pending_count() <= 1,
        "single recovery task, pending={}",
        runtime.pending_count()
    );
    let second = runtime.begin_recovery(
        ServiceKind::Meta,
        InterruptedStep::CodingStep { index: 0 },
        T0 + 2_100,
    );
    assert!(second.is_err(), "duplicate begin must be rejected");
    assert_eq!(
        runtime.recovery_state_for_snapshot().generation,
        gen,
        "generation unchanged after duplicate"
    );
    assert!(
        runtime.pending_count() <= 1,
        "duplicate must not spawn a second task, pending={}",
        runtime.pending_count()
    );
    assert!(runtime.is_recovery_active());
    runtime.cancel_recovery();
    assert_eq!(runtime.poll_recovery(T0 + 3_000).await, None);
}

#[tokio::test]
async fn race_late_success_after_cancellation() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("race-late-success", fetcher.clone()).await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes(0);
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    assert!(drive_until_loaded(&mut runtime, &control).await);
    assert_eq!(fetcher.calls(), 1);
    let events_before = runtime.orchestrator().session().events().len();
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 3_000,
        )
        .unwrap();
    // Let the spawned probe task produce its (now late) success.
    tokio::time::sleep(Duration::from_millis(30)).await;
    runtime.cancel_recovery();
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(
        runtime.poll_recovery(T0 + 4_000).await,
        None,
        "late success must be ignored"
    );
    runtime.drain_intents(T0 + 4_000).await.unwrap();
    assert_eq!(runtime.poll_recovery(T0 + 4_000).await, None);
    assert_ne!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Cancelled);
    assert_eq!(runtime.snapshot_recovery(), None);
    assert_eq!(fetcher.calls(), 1, "no load retry from a stale success");
    assert_eq!(
        runtime.orchestrator().session().events().len(),
        events_before
    );
}

#[tokio::test]
async fn race_duplicate_timeout_event() {
    let fetcher = Arc::new(FlakyFetcher::new(1));
    let (_root, control, mut runtime) = make_runtime("race-dup-timeout", fetcher.clone()).await;
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
    assert_eq!(
        runtime
            .orchestrator()
            .session()
            .events()
            .iter()
            .filter(|e| e.kind == EventKind::TechnicalFailure)
            .count(),
        1
    );
    // Duplicate timeout delivery must be a no-op.
    assert_eq!(runtime.poll_recovery(T0 + 6_000).await, None);
    assert_eq!(runtime.poll_recovery(T0 + 7_000).await, None);
    runtime.drain_intents(T0 + 7_000).await.unwrap();
    assert_eq!(
        runtime
            .orchestrator()
            .session()
            .events()
            .iter()
            .filter(|e| e.kind == EventKind::TechnicalFailure)
            .count(),
        1,
        "exactly one TechnicalFailure event"
    );
    assert_eq!(
        runtime.orchestrator().status(),
        SessionStatus::TechnicalFailure
    );
}

#[tokio::test]
async fn tty_tick_remains_responsive_during_recovery_responsiveness() {
    let fetcher = Arc::new(FlakyFetcher::new(0));
    let (_root, control, mut runtime) = make_runtime("tty-responsive", fetcher.clone()).await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_always_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(runtime.is_recovery_active());
    let started = Instant::now();
    for i in 0..5u64 {
        let now = T0 + 3_000 + i * 100;
        assert_eq!(runtime.poll_recovery(now).await, None);
        runtime.drain_intents(now).await.unwrap();
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "tty tick blocked during recovery: {elapsed:?}"
    );
    assert!(
        runtime.is_recovery_active(),
        "recovery still active after responsive ticks"
    );
    // Heartbeat path remains callable during recovery.
    let disconnected = runtime.check_heartbeat(T0 + 3_500).await;
    assert!(
        !disconnected,
        "heartbeat check should run and not disconnect inside timeout"
    );
    assert!(runtime.is_recovery_active());
    runtime.cancel_recovery();
}
