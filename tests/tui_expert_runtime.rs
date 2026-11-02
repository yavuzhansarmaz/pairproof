use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{EventKind, Language, QuestionRef, Session, SessionStatus};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::extraction::ExtractionResult;
use pairproof::llm::Message;
use pairproof::orchestration::control::{final_ack_bytes, FinalAckOutcome};
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::app::{AppEvent, AppScreen, ExpertApp};
use pairproof::tui::expert_runtime::{
    expert_context_from_extraction, first_free_port, format_session_id, SANDBOX_PORT_RANGE,
};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, PollOutcome,
};
use pairproof::tui::{apply_expert_setup_paste, ExpertSetupPastes};

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
    make_runtime_with_sync(name, hang, SessionSyncPhase::NotStarted).await
}
#[rustfmt::skip]
async fn make_runtime_with_sync(name: &str, hang: bool, sync_phase: SessionSyncPhase) -> (PathBuf, PathBuf, ExpertRuntime) {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: qs("e", 5, Difficulty::Easy), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher { hang }), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding::default()), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: Vec::new(), level: String::new(), sync_phase };
    let runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    (root, control, runtime)
}

#[rustfmt::skip]
fn write_hello(control: &std::path::Path, session_id: &str) {
    let body = serde_json::json!({ "v": 1, "session_id": session_id, "app_version": env!("CARGO_PKG_VERSION") }).to_string();
    std::fs::write(control.join("inbox").join("hello.json"), body).unwrap();
}
#[rustfmt::skip]
fn write_hello_version(control: &std::path::Path, session_id: &str, version: &str) {
    let body = serde_json::json!({ "v": 1, "session_id": session_id, "app_version": version }).to_string();
    std::fs::write(control.join("inbox").join("hello.json"), body).unwrap();
}
#[rustfmt::skip]
fn write_ack(control: &std::path::Path, session_id: &str) { std::fs::write(control.join("inbox").join("final-ack.json"), final_ack_bytes(session_id)).unwrap(); }
#[rustfmt::skip]
fn write_ack_raw(control: &std::path::Path, bytes: &[u8]) { std::fs::write(control.join("inbox").join("final-ack.json"), bytes).unwrap(); }
#[rustfmt::skip]
fn connected_events(runtime: &ExpertRuntime) -> usize {
    runtime.orchestrator().session().events().iter().filter(|e| e.kind == EventKind::CandidateConnected).count()
}

#[tokio::test] #[rustfmt::skip]
async fn expert_runtime_accepts_only_after_candidate_hello() {
    let (_root, control, mut runtime) = make_runtime("accept", false).await;
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Waiting);
    assert!(!runtime.orchestrator().is_connected()); assert_eq!(runtime.gate_phase(), GatePhase::Awaiting);
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 2_000).await, PollOutcome::Connected);
    assert!(runtime.orchestrator().is_connected()); assert_eq!(runtime.gate_phase(), GatePhase::Locked);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::InProgress);
    assert_eq!(runtime.orchestrator().current_index(), Some(0));
    assert_eq!(runtime.poll_candidate(T0 + 3_000).await, PollOutcome::AlreadyConnected);
    assert_eq!(connected_events(&runtime), 1);
}

#[tokio::test] #[rustfmt::skip]
async fn expert_runtime_rejects_hello_for_wrong_session() {
    let (_root, control, mut runtime) = make_runtime("wrong-session", false).await;
    write_hello(&control, "other-session");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::WrongSession);
    assert!(!runtime.orchestrator().is_connected()); assert_eq!(runtime.gate_phase(), GatePhase::Awaiting);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::WaitingForCandidate);
    assert_eq!(connected_events(&runtime), 0);
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 2_000).await, PollOutcome::Connected);
    assert!(runtime.orchestrator().is_connected());
}

#[tokio::test] #[rustfmt::skip]
async fn expert_runtime_drains_intents_and_publishes_snapshot() {
    let (_root, control, mut runtime) = make_runtime("drain", false).await;
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    let intent = Intent::new("s1", 1, IntentKind::Finish);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(control.join(INTENT_REL)).unwrap();
    file.write_all(format!("{}\n", intent.to_line().unwrap()).as_bytes()).unwrap();
    drop(file);
    let reports = runtime.drain_intents(T0 + 2_000).await.unwrap();
    assert!(reports.iter().any(|r| r.outcome == ApplyOutcome::Applied));
    assert_eq!(runtime.orchestrator().current_index(), Some(1));
    let bytes = std::fs::read(control.join(SNAPSHOT_REL)).unwrap();
    let snapshot = parse_snapshot(&bytes, "s1").unwrap();
    assert_eq!(snapshot.current.as_ref().map(|c| c.index), Some(1));
}

#[tokio::test] #[rustfmt::skip]
async fn expert_snapshot_carries_typed_sync_phase_labels() {
    let cases = [
        (SessionSyncPhase::NotStarted, "sync-not-started", "Sync: not started"),
        (SessionSyncPhase::Active, "sync-active", "Sync: active"),
        (SessionSyncPhase::Paused, "sync-paused", "Sync: paused"),
        (SessionSyncPhase::Conflict, "sync-conflict", "Sync: conflict needs attention \u{2014} ask the expert"),
    ];
    for (phase, name, label) in cases {
        let (_root, control, mut runtime) = make_runtime_with_sync(name, false, phase).await;
        assert_eq!(runtime.sync_phase(), phase);
        write_hello(&control, "s1");
        assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
        runtime.drain_intents(T0 + 2_000).await.unwrap();
        let bytes = std::fs::read(control.join(SNAPSHOT_REL)).unwrap();
        let snapshot = parse_snapshot(&bytes, "s1").unwrap();
        assert_eq!(snapshot.v, 1, "SNAPSHOT_VERSION must stay 1 for {phase:?}");
        assert_eq!(snapshot.sync_phase, label, "phase {phase:?}");
        let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(wire["sync_phase"], label, "wire label for {phase:?}");
        assert_eq!(wire["v"], 1);
    }
}

#[tokio::test] #[rustfmt::skip]
async fn expert_snapshot_uses_sync_phase_set_after_construction() {
    let (_root, control, mut runtime) = make_runtime("sync-setter", false).await;
    assert_eq!(runtime.sync_phase(), SessionSyncPhase::NotStarted);
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    runtime.set_sync_phase(SessionSyncPhase::Paused);
    runtime.drain_intents(T0 + 2_000).await.unwrap();
    let snapshot = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    assert_eq!(snapshot.sync_phase, "Sync: paused");
    runtime.set_sync_phase(SessionSyncPhase::Conflict);
    runtime.drain_intents(T0 + 3_000).await.unwrap();
    let snapshot = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    assert_eq!(snapshot.sync_phase, "Sync: conflict needs attention \u{2014} ask the expert");
}

#[tokio::test] #[rustfmt::skip]
async fn expert_runtime_end_cancels_inflight_and_cleans_up() {
    let (root, control, mut runtime) = make_runtime("end", true).await;
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    tokio::task::yield_now().await;
    assert_eq!(runtime.pending_count(), 1, "hanging statement load must be in flight");
    write_ack(&control, "s1");
    let report = runtime.end_session(T0 + 2_000).await.unwrap();
    let names: Vec<String> = ["cancel_inflight", "terminate_connection", "stop_sync", "destroy_sandbox", "delete_raw"].iter().map(ToString::to_string).collect();
    assert_eq!(report.attempted, names);
    assert!(runtime.is_killed()); assert_eq!(runtime.gate_phase(), GatePhase::Ended);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::EndedByExpert);
    assert!(!root.join("artifacts").join("s1").join("raw").exists());
    assert!(root.join("artifacts").join("s1").join("report.md").is_file());
    assert_eq!(runtime.cleanup_report().map(|r| r.attempted.len()), Some(5));
}

#[tokio::test] #[rustfmt::skip]
async fn expert_runtime_never_fakes_connection_lock() {
    let (_root, _control, mut runtime) = make_runtime("never-fake", false).await;
    for step in 0..3u64 {
        assert_eq!(runtime.poll_candidate(T0 + step * 1_000).await, PollOutcome::Waiting);
        let current = runtime.current_string(T0 + step * 1_000).unwrap();
        assert!(!current.encode().is_empty()); assert_eq!(runtime.gate_phase(), GatePhase::Awaiting);
        assert!(!runtime.orchestrator().is_connected());
        assert_eq!(runtime.orchestrator().status(), SessionStatus::WaitingForCandidate);
    }
    assert_eq!(connected_events(&runtime), 0);
}

#[tokio::test]
async fn expert_runtime_publishes_session_id_for_candidate() {
    let (_root, control, _runtime) = make_runtime("session-publish", false).await;
    let path = control.join("outbox").join("session.json");
    let bytes = std::fs::read(&path).expect("session binding published");
    assert_eq!(
        pairproof::orchestration::control::parse_session_binding(&bytes).unwrap(),
        "s1"
    );
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["session_id"], "s1");
}

#[tokio::test] #[rustfmt::skip]
async fn expert_runtime_publishes_final_snapshot_with_breakdown() {
    let (_root, control, mut runtime) = make_runtime("final-snapshot", false).await;
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    let intent = Intent::new("s1", 1, IntentKind::Finish);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(control.join(INTENT_REL)).unwrap();
    file.write_all(format!("{}\n", intent.to_line().unwrap()).as_bytes()).unwrap();
    drop(file);
    runtime.drain_intents(T0 + 2_000).await.unwrap();
    let before = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    assert_eq!(before.final_breakdown, None);
    write_ack(&control, "s1");
    runtime.end_session(T0 + 3_000).await.unwrap();
    let after = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    let breakdown = after.final_breakdown.expect("final breakdown published");
    assert!(breakdown.rating.is_finite(), "rating: {}", breakdown.rating);
    assert_eq!(breakdown.per_question.len(), 5); assert_eq!(after.phase, "EndedByExpert");
}

#[tokio::test]
async fn expert_runtime_selection_delegates_to_core() {
    let (_root, _control, mut runtime) = make_runtime("select", false).await;
    runtime
        .expert_select(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            T0 + 1_000,
        )
        .unwrap();
    assert_eq!(runtime.orchestrator().session().slots().len(), 10);
    assert!(runtime
        .orchestrator()
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::ExpertQuestionsSet));
    assert!(runtime
        .expert_select(
            qs("m2-", 3, Difficulty::Medium),
            qs("h2-", 2, Difficulty::Hard),
            T0 + 2_000
        )
        .is_err());
    assert_eq!(runtime.orchestrator().session().slots().len(), 10);
    let (_root2, _control2, mut fresh) = make_runtime("select-bad", false).await;
    assert!(fresh
        .expert_select(
            qs("m", 2, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            T0 + 1_000
        )
        .is_err());
    assert_eq!(fresh.orchestrator().session().slots().len(), 5);
}

#[tokio::test]
#[rustfmt::skip]
async fn expert_waits_for_ack_before_destroy() {
    let (root, control, mut runtime) = make_runtime("ack-wait", false).await;
    write_hello(&control, "s1"); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    let ack_control = control.clone();
    let writer = std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(300)); write_ack(&ack_control, "s1"); });
    let started = std::time::Instant::now();
    let report = runtime.end_session_with_ack_deadline(T0 + 2_000, 5_000).await.unwrap();
    writer.join().unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_millis(250), "end must wait for ack, elapsed {:?}", started.elapsed());
    assert_eq!(runtime.final_ack_outcome(), Some(FinalAckOutcome::Received));
    assert_eq!(report.attempted.len(), 5); assert!(report.failed.is_empty(), "{report:?}");
    assert!(root.join("artifacts").join("s1").join("report.md").is_file()); assert!(!root.join("artifacts").join("s1").join("raw").exists());
    let again = std::time::Instant::now(); let second = runtime.end_session_with_ack_deadline(T0 + 3_000, 5_000).await.unwrap();
    assert!(again.elapsed() < std::time::Duration::from_secs(2), "re-entry must not wait again"); assert_eq!(second.attempted.len(), 5);
}

#[tokio::test]
#[rustfmt::skip]
async fn expert_proceeds_after_ack_timeout() {
    let (root, control, mut runtime) = make_runtime("ack-timeout", false).await;
    write_hello(&control, "s1"); assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    let started = std::time::Instant::now();
    let report = runtime.end_session_with_ack_deadline(T0 + 2_000, 60).await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "timeout must be bounded, elapsed {:?}", started.elapsed());
    assert_eq!(runtime.final_ack_outcome(), Some(FinalAckOutcome::Timeout));
    assert_eq!(report.attempted.len(), 5); assert!(report.failed.contains(&("final_snapshot_ack".to_string(), "timeout".to_string())), "{report:?}");
    assert_eq!(runtime.gate_phase(), GatePhase::Ended); assert!(root.join("artifacts").join("s1").join("report.md").is_file());
    let after = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap(); assert!(after.final_breakdown.is_some());
    let again = std::time::Instant::now(); runtime.end_session_with_ack_deadline(T0 + 3_000, 60).await.unwrap(); assert!(again.elapsed() < std::time::Duration::from_secs(2));
}

#[tokio::test]
#[rustfmt::skip]
async fn wrong_session_ack_is_ignored() {
    let (_r1, control1, mut rt1) = make_runtime("ack-wrong1", false).await;
    write_hello(&control1, "s1"); assert_eq!(rt1.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); write_ack(&control1, "other-session");
    let rep1 = rt1.end_session_with_ack_deadline(T0 + 2_000, 60).await.unwrap();
    assert_eq!(rt1.final_ack_outcome(), Some(FinalAckOutcome::Timeout)); assert!(rep1.failed.contains(&("final_snapshot_ack".to_string(), "timeout".to_string())));
    let (_r2, control2, mut rt2) = make_runtime("ack-wrong2", false).await;
    write_hello(&control2, "s1"); assert_eq!(rt2.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    write_ack_raw(&control2, br#"{"v":99,"session_id":"s1","rating_seen":true}"#);
    let rep2 = rt2.end_session_with_ack_deadline(T0 + 2_000, 60).await.unwrap();
    assert_eq!(rt2.final_ack_outcome(), Some(FinalAckOutcome::Timeout)); assert!(rep2.failed.contains(&("final_snapshot_ack".to_string(), "timeout".to_string())));
    let (_r3, control3, mut rt3) = make_runtime("ack-wrong3", false).await;
    write_hello(&control3, "s1"); assert_eq!(rt3.poll_candidate(T0 + 1_000).await, PollOutcome::Connected); write_ack(&control3, "other-session");
    let c3 = control3.clone(); let writer = std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(300)); write_ack(&c3, "s1"); });
    let rep3 = rt3.end_session_with_ack_deadline(T0 + 2_000, 5_000).await.unwrap(); writer.join().unwrap();
    assert_eq!(rt3.final_ack_outcome(), Some(FinalAckOutcome::Received)); assert!(rep3.failed.is_empty(), "{rep3:?}");
}

#[test] #[rustfmt::skip]
fn expert_setup_paste_builds_proposal() {
    let mut pastes = ExpertSetupPastes::new();
    assert!(!pastes.is_complete());
    assert_eq!(apply_expert_setup_paste(&mut pastes, "Senior Rust role building trading systems."), Ok(false));
    assert!(!pastes.is_complete());
    assert_eq!(apply_expert_setup_paste(&mut pastes, "Focus on async runtime behavior and testing."), Ok(true));
    assert!(pastes.is_complete());
    assert_eq!(pastes.jd(), "Senior Rust role building trading systems.");
    assert_eq!(pastes.notes(), "Focus on async runtime behavior and testing.");
    let mut app = ExpertApp::new();
    assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    app.set_preflight_passed(true);
    assert_eq!(app.handle(AppEvent::PreflightCompleted), Ok(AppScreen::ApiKey));
    assert_eq!(app.handle(AppEvent::ApiKeySaved), Ok(AppScreen::Setup));
    assert_eq!(app.handle(AppEvent::DisclosureAcknowledged), Ok(AppScreen::Setup));
    assert!(app.handle(AppEvent::ProposalReady).is_err());
    app.set_has_proposal(pastes.is_complete());
    assert_eq!(app.handle(AppEvent::ProposalReady), Ok(AppScreen::Proposal));
}

#[test] #[rustfmt::skip]
fn expert_setup_requires_both_pastes() {
    let mut pastes = ExpertSetupPastes::new();
    assert_eq!(apply_expert_setup_paste(&mut pastes, "Only a JD, no notes yet."), Ok(false));
    assert!(!pastes.is_complete());
    let mut app = ExpertApp::new();
    assert_eq!(app.handle(AppEvent::SelectExpert), Ok(AppScreen::Preflight));
    app.set_preflight_passed(true);
    assert_eq!(app.handle(AppEvent::PreflightCompleted), Ok(AppScreen::ApiKey));
    assert_eq!(app.handle(AppEvent::ApiKeySaved), Ok(AppScreen::Setup));
    assert_eq!(app.handle(AppEvent::DisclosureAcknowledged), Ok(AppScreen::Setup));
    app.set_has_proposal(pastes.is_complete());
    assert!(app.handle(AppEvent::ProposalReady).is_err());
}

#[test] #[rustfmt::skip]
fn expert_setup_rejects_oversized_paste() {
    let mut pastes = ExpertSetupPastes::new();
    let oversized = "x".repeat(pairproof::ingest::MAX_TEXT_CHARS + 1);
    assert!(apply_expert_setup_paste(&mut pastes, &oversized).is_err());
    assert!(apply_expert_setup_paste(&mut pastes, "   ").is_err());
    assert!(!pastes.is_complete());
    let boundary = "y".repeat(pairproof::ingest::MAX_TEXT_CHARS);
    assert_eq!(apply_expert_setup_paste(&mut pastes, &boundary), Ok(false));
    assert_eq!(apply_expert_setup_paste(&mut pastes, "notes"), Ok(true));
    assert!(pastes.is_complete());
}

#[tokio::test] #[rustfmt::skip]
async fn expert_scored_session_shows_approach_lines_and_rating() {
    fn push_intent(control: &std::path::Path, seq: u64, kind: IntentKind) {
        let intent = Intent::new("s1", seq, kind);
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(control.join(INTENT_REL)).unwrap();
        file.write_all(format!("{}\n", intent.to_line().unwrap()).as_bytes()).unwrap();
    }
    let (_root, control, mut runtime) = make_runtime("scored", false).await;
    // No selection yet: empty-session behaviour is unchanged.
    assert!(runtime.monitor_view(T0 + 500).rating_lines.is_empty());
    assert!((runtime.monitor_view(T0 + 500).rating - 6.0).abs() < 1e-9);
    write_hello(&control, "s1");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    for _ in 0..50 {
        runtime.drain_intents(T0 + 2_000).await.unwrap();
        let snap = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
        if snap.options.len() == 4 { break; }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // Preset 1 is the fake set's Best option ("Hash map").
    push_intent(&control, 1, IntentKind::SelectPreset { preset: 1 });
    runtime.drain_intents(T0 + 3_000).await.unwrap();
    let live = runtime.monitor_view(T0 + 3_000);
    assert!(live.rating_lines.iter().any(|l| l.contains("Approach") && l.contains("Best")), "live lines: {:?}", live.rating_lines);
    assert!((live.rating - 6.15).abs() < 1e-9, "live rating: {}", live.rating);
    push_intent(&control, 2, IntentKind::Finish);
    runtime.drain_intents(T0 + 4_000).await.unwrap();
    assert_eq!(runtime.orchestrator().current_index(), Some(1));
    let done = runtime.monitor_view(T0 + 4_000);
    assert!(done.rating_lines.iter().any(|l| l.contains("Approach") && l.contains("Best")), "finished lines: {:?}", done.rating_lines);
    assert!((done.rating - 6.25).abs() < 1e-9, "finished rating: {}", done.rating);
    write_ack(&control, "s1");
    runtime.end_session(T0 + 5_000).await.unwrap();
    let after = parse_snapshot(&std::fs::read(control.join(SNAPSHOT_REL)).unwrap(), "s1").unwrap();
    let breakdown = after.final_breakdown.expect("final breakdown published");
    assert!((breakdown.rating - 6.25).abs() < 1e-9, "final rating: {}", breakdown.rating);
}

#[test] #[rustfmt::skip]
fn expert_format_session_id_and_port_scan() {
    assert_eq!(format_session_id(123, 456), "e123x456");
    assert_eq!(SANDBOX_PORT_RANGE, 2222..2322);
    assert_eq!(first_free_port(SANDBOX_PORT_RANGE.clone(), |_| false), None);
    assert_eq!(first_free_port(SANDBOX_PORT_RANGE.clone(), |p| p != 2222), Some(2223));
    assert_eq!(first_free_port(SANDBOX_PORT_RANGE, |p| p == 2322), None);
}

#[test] #[rustfmt::skip]
fn expert_build_uses_extracted_topics() {
    let extracted = ExtractionResult { topics: vec!["hash-table".into(), "graph".into()], level: "senior".into(), proposed_language: Some(Language::Python), unsupported_language: None, language_reason: "JD asks for Python".into() };
    assert_eq!(expert_context_from_extraction(&extracted), (vec!["hash-table".to_string(), "graph".to_string()], "senior".to_string(), Language::Python));
    let unsupported = ExtractionResult { topics: vec!["math".into()], level: "mid".into(), proposed_language: None, unsupported_language: Some("Brainfuck".into()), language_reason: String::new() };
    assert_eq!(expert_context_from_extraction(&unsupported), (vec!["math".to_string()], "mid".to_string(), Language::Python));
}

#[tokio::test] #[rustfmt::skip]
async fn expert_candidate_version_match_is_accepted() {
    let (_root, control, mut runtime) = make_runtime("ver-match", false).await;
    write_hello_version(&control, "s1", env!("CARGO_PKG_VERSION"));
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::Connected);
    assert!(runtime.orchestrator().is_connected()); assert_eq!(runtime.version_mismatch(), None);
}

#[tokio::test] #[rustfmt::skip]
async fn expert_candidate_version_mismatch_is_rejected() {
    let (_root, control, mut runtime) = make_runtime("ver-mismatch", false).await;
    write_hello_version(&control, "s1", "9.9.9");
    assert_eq!(runtime.poll_candidate(T0 + 1_000).await, PollOutcome::VersionMismatch);
    assert!(!runtime.orchestrator().is_connected()); assert_eq!(runtime.gate_phase(), GatePhase::Awaiting);
    assert_eq!(runtime.orchestrator().status(), SessionStatus::WaitingForCandidate);
    assert_eq!(connected_events(&runtime), 0);
    let (cand, exp) = runtime.version_mismatch().expect("mismatch stored");
    assert_eq!(cand, "9.9.9"); assert_eq!(exp, env!("CARGO_PKG_VERSION"));
    assert!(control.join("inbox").join("hello.json").exists(), "hello must not be consumed on mismatch");
}
