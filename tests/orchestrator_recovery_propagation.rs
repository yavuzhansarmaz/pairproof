use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::Language;
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::control::{CandidateSnapshot, Intent};
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_runtime::{
    CandidateRuntime, CandidateRuntimeError, CandidateTransport,
};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, PollOutcome,
};

mod common_mod;
use common_mod::qs;
type PF<'a, T> = PipelineFuture<'a, T>;
const T0: u64 = 1_730_000_000_000;

#[rustfmt::skip]
fn started() -> Orchestrator { let mut o = Orchestrator::create("s1".into(), Language::Python, qs("e", 5, Difficulty::Easy), 0).unwrap(); o.mark_waiting_for_candidate(1).unwrap(); o.attach_candidate(2).unwrap(); o.start(3).unwrap(); o }
#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-rec-prop-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
#[rustfmt::skip]
fn active_recovery(generation: u64, remaining_ms: u64) -> SnapshotRecovery { SnapshotRecovery::from_service(true, Some(ServiceKind::Meta), remaining_ms, generation, false) }
#[rustfmt::skip]
fn snap_with(recovery: Option<SnapshotRecovery>) -> CandidateSnapshot { build_snapshot_with_recovery(&started(), &QuestionPipeline::new(), SessionSyncPhase::Active, &[], None, recovery) }
#[rustfmt::skip]
fn roundtrip(snap: &CandidateSnapshot) -> CandidateSnapshot { parse_snapshot(&serde_json::to_vec(snap).unwrap(), "s1").unwrap() }

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

struct FakeFetcher;
#[rustfmt::skip]
impl StatementFetcher for FakeFetcher { fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> { let s = slug.to_string(); Box::pin(async move { Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) }) } }
#[derive(Default)]
struct FakeGenerator;
#[rustfmt::skip]
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> { Box::pin(async move { let opt = |t: &str, quality: ApproachQuality, tok: u64| pairproof::scoring::ApproachOption { title: t.into(), summary: format!("{t} summary"), quality, expected_tokens: tok }; ApproachSet::new(vec![opt("Hash map", ApproachQuality::Best, 4242)]).map_err(|e| PipelineError::Llm(e.to_string())) }) }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> { Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) }) }
}
#[derive(Default)]
struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding { fn coding_step<'a>(&'a self, s: &'a mut pairproof::core::Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) }) } }
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
impl ChatTransport for FakeChat { fn chat<'a>(&'a self, _s: &'a mut pairproof::core::Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> { Box::pin(async move { Ok("Think about a map.".into()) }) } }
struct FakeGrader;
#[rustfmt::skip]
impl QuestionGrader for FakeGrader { fn grade_all<'a>(&'a self, _i: &'a [pairproof::llm::grading::QuestionScoringInput]) -> GradeFuture<'a> { Box::pin(async move { Vec::new() }) } }
struct DownProbes {
    calls: AtomicU32,
}
struct LeetFake(Arc<DownProbes>);
#[rustfmt::skip]
impl LeetCodeProbe for LeetFake { fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> { let f = self.0.clone(); Box::pin(async move { f.calls.fetch_add(1, Ordering::SeqCst); ServiceHealth::Down { service: ServiceKind::LeetCode, reason: "status 503".into() } }) } }
struct MetaFake(Arc<DownProbes>);
#[rustfmt::skip]
impl MetaProbe for MetaFake { fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> { let f = self.0.clone(); Box::pin(async move { f.calls.fetch_add(1, Ordering::SeqCst); ServiceHealth::Down { service: ServiceKind::Meta, reason: "status 503".into() } }) } }
#[rustfmt::skip]
async fn make_expert(name: &str) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts, topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted };
    (root, control.clone(), ExpertRuntime::new(spec, deps, T0).await.unwrap())
}
#[rustfmt::skip]
fn write_hello(control: &std::path::Path) { std::fs::write(control.join("inbox").join("hello.json"), serde_json::json!({ "v": 1, "session_id": "s1", "app_version": env!("CARGO_PKG_VERSION") }).to_string()).unwrap(); }
#[rustfmt::skip]
async fn connect_expert(runtime: &mut ExpertRuntime, control: &std::path::Path) { write_hello(control); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); }
#[rustfmt::skip]
fn read_published(control: &std::path::Path) -> CandidateSnapshot { parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap() }

#[test]
fn recovery_state_propagates_to_candidate() {
    let recovery = active_recovery(2, 45_000);
    let snap = roundtrip(&snap_with(Some(recovery.clone())));
    assert_eq!(snap.v, SNAPSHOT_VERSION);
    assert_eq!(snap.recovery, Some(recovery));
    // Old v1 JSON without the optional field still parses.
    let mut wire = serde_json::to_value(snap_with(None)).unwrap();
    wire.as_object_mut().unwrap().remove("recovery");
    assert_eq!(
        parse_snapshot(&serde_json::to_vec(&wire).unwrap(), "s1")
            .unwrap()
            .recovery,
        None
    );
    assert_eq!(
        build_snapshot(
            &started(),
            &QuestionPipeline::new(),
            SessionSyncPhase::Active,
            &[],
            None
        )
        .recovery,
        None
    );
    let (dir, mut candidate) = connected_candidate("propagates", vec![snap]);
    candidate.poll_snapshot_at(10_000).unwrap();
    let view = candidate.view();
    assert_eq!(view.recovery.as_ref().map(|r| r.generation), Some(2));
    let lines = view.lines().join("\n");
    assert!(
        lines.contains("technical problem - service temporarily unavailable."),
        "{lines}"
    );
    assert!(
        lines.contains("retrying... about 45 seconds remaining."),
        "{lines}"
    );
    assert!(lines.contains("your progress is preserved."), "{lines}");
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn recovery_state_is_consistent_across_roles() {
    let (_root, control, mut expert) = make_expert("consistent").await;
    expert.set_recovery_window_ms(5_000);
    connect_expert(&mut expert, &control).await;
    expert
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    let expert_state = expert.recovery_state_for_snapshot();
    let snapshot_recovery = expert
        .snapshot_recovery()
        .expect("recovering snapshot recovery");
    assert_eq!(snapshot_recovery.generation, expert_state.generation);
    assert_eq!(snapshot_recovery.service.as_deref(), Some("Meta"));
    assert!(snapshot_recovery.active && !snapshot_recovery.technical_failure);
    assert!(
        expert_state
            .remaining_ms
            .abs_diff(snapshot_recovery.remaining_ms)
            <= 2_000,
        "{expert_state:?} vs {snapshot_recovery:?}"
    );
    expert.drain_intents(T0 + 3_000).await.unwrap();
    let published = read_published(&control);
    let published_recovery = published.recovery.clone().expect("published recovery");
    assert_eq!(published_recovery.generation, expert_state.generation);
    assert_eq!(published_recovery.service, snapshot_recovery.service);
    assert_eq!(published_recovery.active, snapshot_recovery.active);
    assert!(
        published_recovery
            .remaining_ms
            .abs_diff(snapshot_recovery.remaining_ms)
            <= 2_000
    );
    let (dir, mut candidate) = connected_candidate("consistent-candidate", vec![published]);
    candidate.poll_snapshot_at(20_000).unwrap();
    assert_eq!(candidate.view().recovery, Some(published_recovery));
    expert.cancel_recovery();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn candidate_recovery_message_is_sanitized() {
    let recovery = SnapshotRecovery {
        active: true,
        service: Some("Meta".to_string()),
        remaining_ms: 12_345,
        generation: 424_242,
        technical_failure: false,
    };
    let snap = snap_with(Some(recovery));
    let json = serde_json::to_string(&snap).unwrap();
    for banned in ["\"status\"", "\"body\"", "secret", "MetaApiError", "reason"] {
        assert!(!json.contains(banned), "{banned} leaked: {json}");
    }
    let (dir, mut candidate) = connected_candidate("sanitized", vec![roundtrip(&snap)]);
    candidate.poll_snapshot_at(10_000).unwrap();
    let lines = candidate.view().lines().join("\n");
    assert!(
        lines.contains("technical problem - service temporarily unavailable."),
        "{lines}"
    );
    assert!(
        lines.contains("retrying... about 13 seconds remaining."),
        "{lines}"
    );
    assert!(lines.contains("your progress is preserved."), "{lines}");
    for banned in ["503", "MetaApiError", "424242", "status", "body", "secret"] {
        assert!(!lines.contains(banned), "{banned} leaked: {lines}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn expert_receives_permitted_recovery_details() {
    let (_root, control, mut expert) = make_expert("expert-details").await;
    expert.set_recovery_window_ms(5_000);
    connect_expert(&mut expert, &control).await;
    expert
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    let state = expert.recovery_state_for_snapshot();
    assert_eq!(state.service, Some(ServiceKind::Meta));
    assert_eq!(state.generation, 1);
    assert_eq!(state.phase, RecoveryPhase::Recovering);
    assert_eq!(
        state.interrupted,
        Some(InterruptedStep::CodingStep { index: 0 })
    );
    let expert_lines = expert.recovery_lines().join("\n");
    assert!(expert_lines.contains("Meta"), "{expert_lines}");
    assert!(expert_lines.contains("seconds remaining"), "{expert_lines}");
    assert!(!expert_lines.contains("ms remaining"), "{expert_lines}");
    assert!(
        !expert_lines.contains(&state.generation.to_string()),
        "generation leaked: {expert_lines}"
    );
    for banned in ["generation", "Some(", "attempt"] {
        assert!(
            !expert_lines.contains(banned),
            "{banned} leaked: {expert_lines}"
        );
    }
    let snapshot_recovery = expert.snapshot_recovery().unwrap();
    assert_eq!(snapshot_recovery.service.as_deref(), Some("Meta"));
    assert_eq!(snapshot_recovery.generation, state.generation);
    assert_eq!(
        SnapshotRecovery::service_name(ServiceKind::LeetCode),
        "LeetCode"
    );
    assert_eq!(snapshot_service_name(ServiceKind::Meta), "Meta");
    expert.cancel_recovery();
}

#[test]
fn stale_recovery_update_is_ignored() {
    let (dir, mut candidate) = connected_candidate(
        "stale",
        vec![
            snap_with(Some(active_recovery(5, 50_000))),
            snap_with(Some(active_recovery(3, 9_999))),
        ],
    );
    candidate.poll_snapshot_at(10_000).unwrap();
    assert_eq!(
        candidate
            .view()
            .recovery
            .as_ref()
            .map(|r| (r.generation, r.remaining_ms)),
        Some((5, 50_000))
    );
    candidate.poll_snapshot_at(20_000).unwrap();
    let view = candidate.view();
    assert_eq!(
        view.recovery
            .as_ref()
            .map(|r| (r.generation, r.remaining_ms)),
        Some((5, 50_000))
    );
    assert_eq!(candidate.last_recovery_generation(), 5);
    assert!(
        view.lines()
            .join("\n")
            .contains("about 50 seconds remaining."),
        "{}",
        view.lines().join("\n")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn older_recovery_generation_is_rejected() {
    let (dir, mut candidate) = connected_candidate(
        "older-gen",
        vec![
            snap_with(Some(active_recovery(4, 40_000))),
            snap_with(Some(active_recovery(3, 30_000))),
            snap_with(Some(active_recovery(5, 20_000))),
        ],
    );
    candidate.poll_snapshot_at(10_000).unwrap();
    assert_eq!(
        candidate.view().recovery.as_ref().map(|r| r.generation),
        Some(4)
    );
    candidate.poll_snapshot_at(20_000).unwrap();
    assert_eq!(
        candidate.view().recovery.as_ref().map(|r| r.generation),
        Some(4),
        "older generation must be rejected"
    );
    candidate.poll_snapshot_at(30_000).unwrap();
    assert_eq!(
        candidate
            .view()
            .recovery
            .as_ref()
            .map(|r| (r.generation, r.remaining_ms)),
        Some((5, 20_000)),
        "newer generation must be accepted"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn recovery_clear_event_reaches_both_roles() {
    let (_root, control, mut expert) = make_expert("clear-expert").await;
    expert.set_recovery_window_ms(5_000);
    connect_expert(&mut expert, &control).await;
    expert
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    expert.drain_intents(T0 + 3_000).await.unwrap();
    let active = read_published(&control);
    assert!(active.recovery.as_ref().is_some_and(|r| r.active));
    expert.cancel_recovery();
    assert_eq!(expert.snapshot_recovery(), None);
    expert.drain_intents(T0 + 4_000).await.unwrap();
    let cleared = read_published(&control);
    assert_eq!(cleared.recovery, None);
    let (dir, mut candidate) = connected_candidate("clear-candidate", vec![active, cleared]);
    candidate.poll_snapshot_at(10_000).unwrap();
    assert!(candidate.view().recovery.is_some());
    assert!(candidate
        .view()
        .lines()
        .join("\n")
        .contains("technical problem"));
    candidate.poll_snapshot_at(20_000).unwrap();
    assert_eq!(candidate.view().recovery, None);
    assert!(!candidate
        .view()
        .lines()
        .join("\n")
        .contains("technical problem"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn technical_failure_reaches_both_roles() {
    // Candidate-safe projection via a failed orchestrator snapshot.
    let mut failed = started();
    failed.mark_technical_failure(60).unwrap();
    let failure_recovery =
        SnapshotRecovery::from_service(false, Some(ServiceKind::Meta), 0, 3, true);
    let snap = build_snapshot_with_recovery(
        &failed,
        &QuestionPipeline::new(),
        SessionSyncPhase::Active,
        &[],
        None,
        Some(failure_recovery.clone()),
    );
    assert_eq!(snap.phase, "TechnicalFailure");
    let parsed = roundtrip(&snap);
    assert_eq!(parsed.recovery, Some(failure_recovery));
    let (dir, mut candidate) = connected_candidate("technical-candidate", vec![parsed]);
    candidate.poll_snapshot_at(10_000).unwrap();
    let lines = candidate.view().lines().join("\n");
    assert!(
        lines.contains("the session ended as a technical failure."),
        "{lines}"
    );
    assert!(lines.contains("no evaluation was produced"), "{lines}");
    let _ = std::fs::remove_dir_all(dir);
    // Expert expiry publishes TechnicalFailure with technical_failure=true.
    let (_root, control, mut expert) = make_expert("technical-expert").await;
    expert.set_recovery_window_ms(80);
    let shared = Arc::new(DownProbes {
        calls: AtomicU32::new(0),
    });
    expert.set_health_probes(
        Arc::new(LeetFake(shared.clone())),
        Arc::new(MetaFake(shared)),
    );
    connect_expert(&mut expert, &control).await;
    expert
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    let mut outcome = None;
    for _ in 0..100 {
        if let Some(o) = expert.poll_recovery(T0 + 5_000).await {
            outcome = Some(o);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(outcome, Some(RecoveryOutcome::TechnicalFailure));
    assert_eq!(expert.recovery_phase(), RecoveryPhase::Failed);
    let published = read_published(&control);
    assert_eq!(published.phase, "TechnicalFailure");
    assert!(
        published
            .recovery
            .as_ref()
            .is_some_and(|r| r.technical_failure && !r.active),
        "{published:?}"
    );
    let (dir2, mut candidate2) = connected_candidate("technical-expert-candidate", vec![published]);
    candidate2.poll_snapshot_at(10_000).unwrap();
    assert!(
        candidate2
            .view()
            .lines()
            .join("\n")
            .contains("technical failure"),
        "{}",
        candidate2.view().lines().join("\n")
    );
    let _ = std::fs::remove_dir_all(dir2);
}
