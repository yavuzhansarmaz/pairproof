use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::Language;
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource,
};

use crate::common_mod;
use common_mod::qs;
type PF<'a, T> = PipelineFuture<'a, T>;
const T0: u64 = 1_730_000_000_000;

#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-rec-ui-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

#[derive(Default)]
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
            ApproachSet::new(vec![opt("Brute force", ApproachQuality::Weak, 111), opt("Hash map", ApproachQuality::Best, 222), opt("Sort first", ApproachQuality::Workable, 333), opt("Two pointers", ApproachQuality::Workable, 444)]).map_err(|e| PipelineError::Llm(e.to_string()))
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
struct FixedLeet(bool);
impl LeetCodeProbe for FixedLeet {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let up = self.0;
        Box::pin(async move {
            if up {
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
struct FixedMeta(bool);
impl MetaProbe for FixedMeta {
    fn probe<'a>(&'a self) -> pairproof::orchestration::health::ProbeFuture<'a> {
        let up = self.0;
        Box::pin(async move {
            if up {
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
fn probes(up: bool) -> (Arc<dyn LeetCodeProbe>, Arc<dyn MetaProbe>) {
    (
        Arc::new(FixedLeet(up)) as Arc<dyn LeetCodeProbe>,
        Arc::new(FixedMeta(up)) as Arc<dyn MetaProbe>,
    )
}

#[rustfmt::skip]
async fn make_runtime(name: &str) -> (PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control)), fetcher: Arc::new(FakeFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts, topics: Vec::new(), level: String::new(), sync_phase: SessionSyncPhase::NotStarted, shadow_evaluator: None };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, runtime)
}

fn recovery(active: bool, remaining_ms: u64, technical_failure: bool) -> SnapshotRecovery {
    SnapshotRecovery {
        active,
        service: Some("Meta".to_string()),
        remaining_ms,
        generation: 7,
        technical_failure,
    }
}
fn candidate_view(rec: Option<SnapshotRecovery>) -> CandidateView {
    CandidateView {
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        question_title: Some("Two Sum".to_string()),
        question_index: Some(0),
        statement: Some("Find two numbers that sum to target.".to_string()),
        options: vec![SnapshotOption {
            title: "Hash map".to_string(),
            summary: "One pass lookup".to_string(),
        }],
        nav: Vec::new(),
        selected: Some(SnapshotSelection::Preset(0)),
        latest_test: Some(SnapshotTest {
            passed: 3,
            failed: 1,
            at_ms: 10,
        }),
        chat_replies: vec!["Try a hash map.".to_string()],
        chat_partial: None,
        final_breakdown: None,
        recovery: rec,
    }
}

#[tokio::test]
async fn expert_renders_active_recovery_state() {
    let (_root, mut runtime) = make_runtime("expert-active").await;
    runtime.set_recovery_window_ms(60_000);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::ChatTurn { index: 0 },
            T0,
        )
        .unwrap();
    let lines = runtime.recovery_lines();
    let joined = lines.join("\n");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("Meta"), "{joined}");
    assert_eq!(lines[0], "service temporarily unavailable: Meta");
    assert!(lines[1].contains("seconds remaining."), "{joined}");
    assert!(!joined.contains("ms remaining"), "{joined}");
    assert!(!joined.contains("attempt"), "{joined}");
    assert!(!joined.contains("Some("), "{joined}");
    assert!(!joined.contains("Recovering Meta"), "{joined}");
    runtime.cancel_recovery();
    // Cancelled renders nothing (session ending).
    assert!(
        runtime.recovery_lines().is_empty(),
        "{:?}",
        runtime.recovery_lines()
    );
}

#[test]
fn candidate_renders_active_recovery_state() {
    let view = candidate_view(Some(recovery(true, 45_000, false)));
    let lines = view.lines();
    let joined = lines.join("\n");
    assert!(
        joined.contains("technical problem - service temporarily unavailable."),
        "{joined}"
    );
    assert!(
        joined.contains("retrying... about 45 seconds remaining."),
        "{joined}"
    );
    assert!(joined.contains("your progress is preserved."), "{joined}");
    assert!(!joined.contains("ms remaining"), "{joined}");
}

#[tokio::test]
async fn both_roles_render_authoritative_countdown() {
    // Candidate: authoritative remaining_ms 61_500 -> ceil 62s.
    let view = candidate_view(Some(recovery(true, 61_500, false)));
    let candidate_joined = view.lines().join("\n");
    assert!(
        candidate_joined.contains("about 62 seconds remaining."),
        "{candidate_joined}"
    );
    // Expert: authoritative controller window 61_500 -> same ceil.
    let (_root, mut runtime) = make_runtime("both-countdown").await;
    runtime.set_recovery_window_ms(61_500);
    runtime
        .begin_recovery(
            ServiceKind::LeetCode,
            InterruptedStep::ChatTurn { index: 0 },
            T0,
        )
        .unwrap();
    let lines = runtime.recovery_lines();
    let joined = lines.join("\n");
    assert_eq!(
        lines[0], "service temporarily unavailable: LeetCode",
        "{joined}"
    );
    assert!(joined.contains("about 62 seconds remaining."), "{joined}");
    assert!(!joined.contains("ms remaining"), "{joined}");
    runtime.cancel_recovery();
}

#[test]
fn candidate_recovery_ui_hides_internal_diagnostics() {
    let mut rec = recovery(true, 5_000, false);
    rec.generation = 99;
    let view = candidate_view(Some(rec));
    let joined = view.lines().join("\n");
    for banned in [
        "Meta",
        "LeetCode",
        "generation",
        "99",
        "503",
        "status 5",
        "Some(",
        "ServiceKind",
        "remaining_ms",
    ] {
        assert!(!joined.contains(banned), "{banned} leaked: {joined}");
    }
    assert!(
        joined.contains("technical problem - service temporarily unavailable."),
        "{joined}"
    );
    assert!(joined.contains("about 5 seconds remaining."), "{joined}");
}

#[tokio::test]
async fn recovery_success_restores_normal_screen() {
    let (_root, mut runtime) = make_runtime("success-restores").await;
    runtime.set_recovery_window_ms(5_000);
    let (leet, meta) = probes(true);
    runtime.set_health_probes(leet, meta);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::ChatTurn { index: 0 },
            T0,
        )
        .unwrap();
    assert!(!runtime.recovery_lines().is_empty());
    let mut resumed = false;
    for _ in 0..100 {
        if let Some(RecoveryOutcome::Resumed(_)) = runtime.poll_recovery(T0 + 1_000).await {
            resumed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(resumed, "recovery should resume");
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Healthy);
    assert!(
        runtime.recovery_lines().is_empty(),
        "{:?}",
        runtime.recovery_lines()
    );
    // Candidate normal screen: no recovery lines.
    let view = candidate_view(None);
    let joined = view.lines().join("\n");
    assert!(!joined.contains("technical problem"), "{joined}");
    assert!(!joined.contains("Retrying"), "{joined}");
    assert!(joined.contains("Two Sum"), "{joined}");
}

#[tokio::test]
async fn recovery_timeout_renders_technical_failure() {
    let (_root, mut runtime) = make_runtime("timeout-failure").await;
    runtime.set_recovery_window_ms(80);
    let (leet, meta) = probes(false);
    runtime.set_health_probes(leet, meta);
    runtime
        .begin_recovery(
            ServiceKind::Meta,
            InterruptedStep::ChatTurn { index: 0 },
            T0,
        )
        .unwrap();
    let mut outcome = None;
    for _ in 0..100 {
        if let Some(o) = runtime.poll_recovery(T0 + 5_000).await {
            outcome = Some(o);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(outcome, Some(RecoveryOutcome::TechnicalFailure));
    assert_eq!(runtime.recovery_phase(), RecoveryPhase::Failed);
    assert_eq!(
        runtime.recovery_lines(),
        vec![
            "service recovery failed.".to_string(),
            "session ended as a technical failure.".to_string()
        ]
    );
    // Candidate technical-failure screen.
    let mut view = candidate_view(Some(recovery(false, 0, true)));
    view.phase = "TechnicalFailure".to_string();
    let joined = view.lines().join("\n");
    assert!(
        joined.contains("the session ended as a technical failure."),
        "{joined}"
    );
    assert!(
        joined.contains("no evaluation was produced; the interview can be rescheduled."),
        "{joined}"
    );
    assert!(!joined.contains("Meta"), "{joined}");
    assert!(!joined.contains("503"), "{joined}");
}

#[test]
fn recovery_ui_preserves_existing_candidate_progress() {
    let snapshot = CandidateSnapshot {
        v: 1,
        session_id: "s1".to_string(),
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        current: Some(SnapshotQuestion {
            index: 0,
            title: "Two Sum".to_string(),
            slug: "two-sum".to_string(),
            statement: Some("Find two numbers that sum to target.".to_string()),
        }),
        options: vec![SnapshotOption {
            title: "Hash map".to_string(),
            summary: "One pass lookup".to_string(),
        }],
        nav: Vec::new(),
        selected: Some(SnapshotSelection::Preset(0)),
        latest_test: Some(SnapshotTest {
            passed: 3,
            failed: 1,
            at_ms: 10,
        }),
        chat_replies: vec!["Try a hash map.".to_string()],
        final_breakdown: None,
        chat_partial: None,
        recovery: Some(recovery(true, 30_000, false)),
    };
    let view = CandidateView::from_snapshot(Some(&snapshot));
    let lines = view.lines();
    let joined = lines.join("\n");
    assert!(joined.contains("Q1: Two Sum"), "{joined}");
    assert!(
        joined.contains("Find two numbers that sum to target."),
        "{joined}"
    );
    assert!(joined.contains("Hash map"), "{joined}");
    assert!(joined.contains("selected approach: preset 1"), "{joined}");
    assert!(
        joined.contains("technical problem - service temporarily unavailable."),
        "{joined}"
    );
    assert!(
        joined.contains("retrying... about 30 seconds remaining."),
        "{joined}"
    );
    assert!(joined.contains("your progress is preserved."), "{joined}");
}
