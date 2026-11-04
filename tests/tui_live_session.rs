//! Live two-runtime session: real Docker sandbox, FIFO SSH auth,
//! control channel and isolated Mutagen sync. Meta/LeetCode transports
//! are faked at the expert boundary (statement, approaches, coding,
//! chat, grading); every transport layer underneath is real.
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Secret};
use pairproof::core::{Language, Session};
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::lifecycle::{create_and_provision, destroy, destroy_by_session_id};
use pairproof::sandbox::preflight::port_is_free;
use pairproof::scoring::{
    ApproachGrade, ApproachQuality, ApproachSet, GradeOutcome, QuestionGrade,
};
use pairproof::tui::candidate_runtime::{
    CandidateRuntime, CandidateRuntimePhase, SshCandidateTransport,
};
use pairproof::tui::expert_runtime::{
    DockerHelloSource, ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, PollOutcome,
};

use crate::common_mod;
use common_mod::q_easy as q;
type PF<'a, T> = PipelineFuture<'a, T>;
const CODE: &str = "def solve():\n    return 42  # pairproof-live-10j\n";
#[rustfmt::skip]
fn now_ms() -> u64 { u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)).unwrap_or(u64::MAX) }

#[rustfmt::skip]
fn chk(cond: bool, msg: &str) -> Result<(), String> { if cond { Ok(()) } else { Err(msg.to_string()) } }

struct LiveFetcher;
#[rustfmt::skip]
impl StatementFetcher for LiveFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let s = slug.to_string();
        Box::pin(async move { Ok(FetchedStatement { statement: format!("Live statement for {s}"), starter_code: "# starter\n".into() }) })
    }
}
struct LiveGenerator;
#[rustfmt::skip]
impl ApproachGenerator for LiveGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> {
        Box::pin(async move {
            let opt = |t: &str, quality: ApproachQuality, tok: u64| pairproof::scoring::ApproachOption { title: t.into(), summary: format!("{t} summary"), quality, expected_tokens: tok };
            ApproachSet::new(vec![opt("Brute force", ApproachQuality::Weak, 111), opt("Hash map", ApproachQuality::Best, 4242), opt("Sort first", ApproachQuality::Workable, 333), opt("Two pointers", ApproachQuality::Workable, 444)]).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> {
        Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) })
    }
}
#[derive(Default)]
struct LiveCoding {
    calls: AtomicU32,
}
#[rustfmt::skip]
impl CodingTransport for LiveCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> {
        Box::pin(async move { self.calls.fetch_add(1, Ordering::SeqCst); s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: CODE.into(), ready: false, summary: "live draft".into() }) })
    }
}
struct LiveChat;
#[rustfmt::skip]
impl ChatTransport for LiveChat {
    fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> {
        Box::pin(async move { Ok("Think about a map.".into()) })
    }
}
struct LiveGrader;
#[rustfmt::skip]
impl QuestionGrader for LiveGrader {
    fn grade_all<'a>(&'a self, inputs: &'a [pairproof::llm::grading::QuestionScoringInput]) -> GradeFuture<'a> {
        let out: Vec<GradeOutcome> = inputs.iter().map(|i| GradeOutcome::Graded(QuestionGrade::new(i.slot_index, 8.0, "live fake grader".into(), 0.1).expect("grade"))).collect();
        Box::pin(async move { out })
    }
}
async fn flow(
    session_id: &str,
    endpoint: &Endpoint,
    username: &EphemeralUsername,
    secret: &Secret,
    handle: &pairproof::sandbox::SandboxHandle,
    fingerprint: pairproof::connection::Fingerprint,
) -> Result<(), String> {
    let base = std::env::temp_dir().join(format!(
        "pairproof-live10j-{}-{}",
        std::process::id(),
        endpoint.port()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let alpha = base.join("alpha");
    let artifacts = base.join("artifacts");
    std::fs::create_dir_all(&alpha).map_err(|e| e.to_string())?;
    let control_io = DockerControlIo::new(handle.clone());
    control_io
        .setup_dirs(username.as_str())
        .await
        .map_err(|e| e.to_string())?;
    let spec = ExpertSessionSpec {
        session_id: session_id.into(),
        language: Language::Python,
        easy: (0..5).map(|i| q(&format!("live-e{i}"))).collect(),
        endpoint: endpoint.clone(),
        username: username.clone(),
        fingerprint: fingerprint.clone(),
    };
    let deps = ExpertRuntimeDeps {
        control_io: Box::new(control_io),
        hello: Box::new(DockerHelloSource::new(handle.clone())),
        fetcher: Arc::new(LiveFetcher),
        generator: Arc::new(LiveGenerator),
        coding: Arc::new(LiveCoding::default()),
        sandbox_io: Arc::new(DockerSandboxIo::new(handle.clone(), username.clone())),
        chat: Arc::new(LiveChat),
        grader: Arc::new(LiveGrader),
        leetcode: None,
        sandbox_handle: Some(handle.clone()),
        artifact_root: artifacts.clone(),
        topics: Vec::new(),
        level: "live".into(),
        sync_phase: SessionSyncPhase::NotStarted,
        shadow_evaluator: None,
    };
    let mut expert = ExpertRuntime::new(spec, deps, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    println!("live10j expert built session={session_id} session_json=published");
    let connection = ConnectionString::new(
        endpoint.clone(),
        username.clone(),
        secret.clone(),
        4_102_444_800_000,
        fingerprint,
    );
    let mut candidate =
        CandidateRuntime::new_discovering(alpha.clone(), SshCandidateTransport::new());
    candidate.connect(&connection).map_err(|e| e.to_string())?;
    chk(
        candidate.phase() == CandidateRuntimePhase::Connected,
        "candidate not Connected",
    )?;
    chk(
        candidate.session_id() == session_id,
        "candidate did not discover expert session id",
    )?;
    println!(
        "live10j candidate connected discovered={} phase=Connected",
        candidate.session_id()
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if expert.poll_candidate(now_ms()).await == PollOutcome::Connected {
            break;
        }
        chk(Instant::now() < deadline, "expert never accepted hello")?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!("live10j expert poll_candidate=Connected");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| e.to_string())?;
        let _ = candidate.poll_snapshot();
        if candidate.view().options.len() == 4 {
            break;
        }
        chk(
            Instant::now() < deadline,
            "approaches never reached candidate",
        )?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    candidate
        .send_intent(IntentKind::SelectPreset { preset: 1 })
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        expert
            .drain_intents(now_ms())
            .await
            .map_err(|e| e.to_string())?;
        let _ = candidate.poll_snapshot();
        if candidate.view().question_index == Some(0) && candidate.view().selected.is_some() {
            break;
        }
        chk(Instant::now() < deadline, "SelectPreset never applied")?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!(
        "live10j SelectPreset applied selected={:?}",
        candidate.view().selected
    );
    let _ = candidate.flush();
    let solution = alpha.join("q1").join("solution.py");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if std::fs::read_to_string(&solution)
            .map(|c| c.contains("pairproof-live-10j"))
            .unwrap_or(false)
        {
            break;
        }
        chk(
            Instant::now() < deadline,
            "solution never synced to candidate alpha",
        )?;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!("live10j solution synced to alpha: {}", solution.display());
    if expert.orchestrator().current_index() != Some(1) {
        candidate
            .send_intent(IntentKind::RequestTests)
            .map_err(|e| e.to_string())?;
        candidate
            .send_intent(IntentKind::Finish)
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            expert
                .drain_intents(now_ms())
                .await
                .map_err(|e| e.to_string())?;
            if expert.orchestrator().current_index() == Some(1) {
                break;
            }
            chk(Instant::now() < deadline, "Finish never applied")?;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
    println!(
        "live10j question advanced current_index={}",
        expert.orchestrator().current_index().unwrap_or(0)
    );
    let poller = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let _ = candidate.poll_snapshot();
            if candidate.view().final_breakdown.is_some() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        let rating = candidate.view().final_breakdown.as_ref().map(|f| f.rating);
        let lines = candidate.view().lines();
        let acked = candidate.final_ack_sent();
        (candidate, rating, lines, acked)
    });
    let report = expert
        .end_session(now_ms())
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "live10j end_session attempted={:?} failed={:?}",
        report.attempted, report.failed
    );
    chk(
        expert.final_ack_outcome()
            == Some(pairproof::orchestration::control::FinalAckOutcome::Received),
        "expert never received candidate final ack before destroy",
    )?;
    println!("live10j expert final_snapshot_ack=received before destroy");
    let (mut candidate, rating, lines, acked) = poller
        .join()
        .map_err(|_| "final poller panicked".to_string())?;
    chk(acked, "candidate never acked final snapshot")?;
    let rating = rating.ok_or("candidate never polled final snapshot with rating")?;
    chk(rating.is_finite(), "final rating not finite")?;
    chk(
        lines.iter().any(|l| l.contains("final rating")),
        "candidate view lacks final rating line",
    )?;
    println!("live10j candidate final rating={rating:.2} lines={lines:?}");
    candidate.end().map_err(|e| e.to_string())?;
    chk(
        candidate.phase() == CandidateRuntimePhase::Ended,
        "candidate not Ended",
    )?;
    let kept = std::fs::read_to_string(&solution).map_err(|e| e.to_string())?;
    chk(
        kept.contains("pairproof-live-10j"),
        "terminate_sync lost synced solution",
    )?;
    println!("live10j terminate_sync preserved alpha solution");
    let bundle = artifacts.join(session_id);
    chk(
        bundle.join("report.md").is_file(),
        "section-15 report.md missing",
    )?;
    chk(
        bundle.join("events.jsonl").is_file(),
        "section-15 events.jsonl missing",
    )?;
    chk(!bundle.join("raw").exists(), "raw artifacts remain")?;
    println!("live10j bundle ok: {}", bundle.display());
    let _ = std::fs::remove_dir_all(&base);
    Ok(())
}

#[tokio::test]
#[ignore = "live docker+ssh+mutagen: real sandbox/SSH/Mutagen/control, fake Meta/LeetCode"]
async fn live_two_runtime_session_over_ssh_and_mutagen() {
    let port = (22500..22700u16)
        .find(|p| port_is_free(*p))
        .expect("free port");
    let session_id = format!("live{}x{}", std::process::id(), port);
    let endpoint = Endpoint::new("127.0.0.1", port).expect("endpoint");
    let username = EphemeralUsername::generate();
    let secret = Secret::generate();
    let (handle, fingerprint) =
        match create_and_provision(&session_id, &endpoint, &username, &secret).await {
            Ok(pair) => pair,
            Err(err) => {
                let _ = destroy_by_session_id(&session_id).await;
                panic!("create_and_provision failed: {err}");
            }
        };
    println!(
        "live10j provisioned container={} port={port}",
        handle.container_name()
    );
    let outcome = flow(
        &session_id,
        &endpoint,
        &username,
        &secret,
        &handle,
        fingerprint,
    )
    .await;
    let destroyed = destroy(&handle).await;
    let _ = destroy_by_session_id(&session_id).await;
    assert!(destroyed.is_ok(), "destroy: {destroyed:?}");
    outcome.expect("live two-runtime session (transport-live, model-faked)");
}
