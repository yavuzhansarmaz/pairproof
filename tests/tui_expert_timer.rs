use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::Language;
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
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-expert-timer-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

struct OkFetcher;
#[rustfmt::skip]
impl StatementFetcher for OkFetcher {
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

struct UpLeet;
impl LeetCodeProbe for UpLeet {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        Box::pin(async move { ServiceHealth::Up })
    }
}
struct UpMeta;
impl MetaProbe for UpMeta {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        Box::pin(async move { ServiceHealth::Up })
    }
}
struct DownLeet;
impl LeetCodeProbe for DownLeet {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        Box::pin(async move {
            ServiceHealth::Down {
                service: ServiceKind::LeetCode,
                reason: "status 503".into(),
            }
        })
    }
}
struct DownMeta;
impl MetaProbe for DownMeta {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        Box::pin(async move {
            ServiceHealth::Down {
                service: ServiceKind::Meta,
                reason: "status 503".into(),
            }
        })
    }
}
fn probes_up() -> (Arc<dyn LeetCodeProbe>, Arc<dyn MetaProbe>) {
    (
        Arc::new(UpLeet) as Arc<dyn LeetCodeProbe>,
        Arc::new(UpMeta) as Arc<dyn MetaProbe>,
    )
}
fn probes_down() -> (Arc<dyn LeetCodeProbe>, Arc<dyn MetaProbe>) {
    (
        Arc::new(DownLeet) as Arc<dyn LeetCodeProbe>,
        Arc::new(DownMeta) as Arc<dyn MetaProbe>,
    )
}

#[rustfmt::skip]
async fn make_runtime(name: &str) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(OkFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted, shadow_evaluator: None };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, control, runtime)
}
#[rustfmt::skip]
fn write_hello(control: &std::path::Path) { let body = serde_json::json!({ "v": 1, "session_id": "s1", "app_version": env!("CARGO_PKG_VERSION") }).to_string(); std::fs::write(control.join("inbox").join("hello.json"), body).unwrap(); }
#[rustfmt::skip]
fn write_ack(control: &std::path::Path) { std::fs::write(control.join("inbox").join("final-ack.json"), final_ack_bytes("s1")).unwrap(); }
#[rustfmt::skip]
async fn connect(runtime: &mut ExpertRuntime, control: &std::path::Path) { write_hello(control); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); }

async fn poll_until_resumed(runtime: &mut ExpertRuntime, now_ms: u64) -> bool {
    for _ in 0..100 {
        if let Some(RecoveryOutcome::Resumed(_)) = runtime.poll_recovery(now_ms).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn interview_timer_pauses_on_recovery() {
    let (_root, control, mut runtime) = make_runtime("pauses").await;
    connect(&mut runtime, &control).await;
    assert_eq!(runtime.monitor_view(T0 + 2_000).elapsed_ms, 1_000);
    assert!(!runtime.timer_paused());
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(runtime.timer_paused());
    assert_eq!(runtime.paused_total_ms(), 0);
    assert_eq!(runtime.adjusted_now_ms(T0 + 2_000), T0 + 2_000);
    assert_eq!(runtime.adjusted_now_ms(T0 + 7_000), T0 + 2_000);
    assert_eq!(runtime.monitor_view(T0 + 2_000).elapsed_ms, 1_000);
    assert_eq!(runtime.monitor_view(T0 + 7_000).elapsed_ms, 1_000);
    runtime.cancel_recovery();
}

#[tokio::test]
async fn interview_timer_excludes_recovery_duration() {
    let (_root, control, mut runtime) = make_runtime("excludes").await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_up();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(poll_until_resumed(&mut runtime, T0 + 7_000).await);
    assert!(!runtime.timer_paused());
    assert_eq!(runtime.paused_total_ms(), 5_000);
    assert_eq!(runtime.adjusted_now_ms(T0 + 7_000), T0 + 2_000);
    assert_eq!(runtime.adjusted_now_ms(T0 + 9_000), T0 + 4_000);
    assert_eq!(runtime.monitor_view(T0 + 7_000).elapsed_ms, 1_000);
    assert_eq!(runtime.monitor_view(T0 + 9_000).elapsed_ms, 3_000);
}

#[tokio::test]
async fn interview_timer_resumes_after_recovery() {
    let (_root, control, mut runtime) = make_runtime("resumes").await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_up();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert_eq!(runtime.monitor_view(T0 + 6_000).elapsed_ms, 1_000);
    assert!(poll_until_resumed(&mut runtime, T0 + 7_000).await);
    assert!(!runtime.timer_paused());
    assert_eq!(runtime.monitor_view(T0 + 7_000).elapsed_ms, 1_000);
    assert_eq!(runtime.monitor_view(T0 + 8_000).elapsed_ms, 2_000);
    assert_eq!(runtime.monitor_view(T0 + 10_000).elapsed_ms, 4_000);
}

#[tokio::test]
async fn interview_timer_does_not_pause_twice() {
    let (_root, control, mut runtime) = make_runtime("no-double-pause").await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_up();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 3_000,
        )
        .is_err());
    assert!(runtime.timer_paused());
    assert_eq!(runtime.adjusted_now_ms(T0 + 4_000), T0 + 2_000);
    assert!(poll_until_resumed(&mut runtime, T0 + 6_000).await);
    // Paused duration is measured from the first pause, not the rejected second.
    assert_eq!(runtime.paused_total_ms(), 4_000);
    assert_eq!(runtime.adjusted_now_ms(T0 + 6_000), T0 + 2_000);
}

#[tokio::test]
async fn interview_timer_does_not_resume_after_timeout() {
    let (_root, control, mut runtime) = make_runtime("no-resume-timeout").await;
    runtime.set_recovery_window_ms(30);
    let (leet, meta) = probes_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    let mut outcome = None;
    for _ in 0..100 {
        if let Some(o) = runtime.poll_recovery(T0 + 7_000).await {
            outcome = Some(o);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(outcome, Some(RecoveryOutcome::TechnicalFailure));
    assert!(runtime.timer_paused());
    assert_eq!(runtime.paused_total_ms(), 0);
    assert_eq!(runtime.adjusted_now_ms(T0 + 7_000), T0 + 2_000);
    assert_eq!(runtime.adjusted_now_ms(T0 + 12_000), T0 + 2_000);
    assert_eq!(runtime.monitor_view(T0 + 12_000).elapsed_ms, 1_000);
}

#[tokio::test]
async fn interview_timer_does_not_resume_after_termination() {
    let (_root, control, mut runtime) = make_runtime("no-resume-termination").await;
    runtime.set_recovery_window_ms(60_000);
    let (leet, meta) = probes_down();
    runtime.set_health_probes(leet, meta);
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    assert!(runtime.timer_paused());
    write_ack(&control);
    runtime
        .end_session_with_ack_deadline(T0 + 3_000, 500)
        .await
        .unwrap();
    // Termination cancels recovery without resuming the pause clock.
    assert!(runtime.timer_paused());
    assert_eq!(runtime.paused_total_ms(), 0);
    assert_eq!(runtime.adjusted_now_ms(T0 + 3_000), T0 + 2_000);
    assert_eq!(runtime.adjusted_now_ms(T0 + 8_000), T0 + 2_000);
    assert_eq!(runtime.poll_recovery(T0 + 9_000).await, None);
    assert!(runtime.timer_paused());
    assert_eq!(runtime.adjusted_now_ms(T0 + 9_000), T0 + 2_000);
}

#[tokio::test]
async fn stale_timer_transition_is_ignored() {
    let (_root, control, mut runtime) = make_runtime("stale").await;
    connect(&mut runtime, &control).await;
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 2_000,
        )
        .unwrap();
    let generation = runtime.recovery_state_for_snapshot().generation;
    assert!(runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::CodingStep { index: 0 },
            T0 + 5_000,
        )
        .is_err());
    assert_eq!(runtime.recovery_state_for_snapshot().generation, generation);
    assert!(runtime.timer_paused());
    assert_eq!(runtime.adjusted_now_ms(T0 + 6_000), T0 + 2_000);
    assert_eq!(runtime.monitor_view(T0 + 6_000).elapsed_ms, 1_000);
    // A stale poll after cancel must not resume the clock either.
    runtime.cancel_recovery();
    assert_eq!(runtime.poll_recovery(T0 + 7_000).await, None);
    assert!(runtime.timer_paused());
    assert_eq!(runtime.adjusted_now_ms(T0 + 7_000), T0 + 2_000);
}
