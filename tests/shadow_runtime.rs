use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pairproof::core::{ApproachChoice, ArtifactLayout, Difficulty, Language, Session};
use pairproof::orchestration::lifecycle::{CancelToken, RequestSupervisor};
use pairproof::orchestration::pipeline::{
    ApproachGenerator, FetchedStatement, PipelineError, PipelineFuture, QuestionPipeline,
    StatementFetcher,
};
use pairproof::orchestration::shadow_runtime::{ShadowEvaluator, ShadowRuntime};
use pairproof::orchestration::Orchestrator;
use pairproof::scoring::shadow::{DecisionStanding, ShadowStatus};
use pairproof::scoring::{
    final_rating, live_rating, ApproachGrade, ApproachOption, ApproachQuality, ApproachSet,
    Confidence, DimensionScores, EvalLevel, OptionEvaluation, OptionVerdict, ShadowReport,
};

mod common_mod;
use common_mod::{qs, tmp};

fn opt(title: &str, summary: &str, quality: ApproachQuality) -> ApproachOption {
    ApproachOption {
        title: title.into(),
        summary: summary.into(),
        quality,
        expected_tokens: 100,
    }
}

fn distinct_set() -> ApproachSet {
    ApproachSet::new(vec![
        opt("Hash map", "single pass hash lookup", ApproachQuality::Best),
        opt(
            "Sort scan",
            "sort values then scan",
            ApproachQuality::Workable,
        ),
        opt("Brute force", "compare every pair", ApproachQuality::Weak),
        opt(
            "Heap",
            "push into heap pop smallest",
            ApproachQuality::Workable,
        ),
    ])
    .expect("set")
}

fn other_set() -> ApproachSet {
    ApproachSet::new(vec![
        opt("Alpha", "alpha unique approach", ApproachQuality::Best),
        opt("Beta", "beta unique approach", ApproachQuality::Workable),
        opt("Gamma", "gamma unique approach", ApproachQuality::Weak),
        opt("Delta", "delta unique approach", ApproachQuality::Workable),
    ])
    .expect("set")
}

fn evals() -> Vec<OptionEvaluation> {
    [
        OptionVerdict::Strong,
        OptionVerdict::Viable,
        OptionVerdict::Weak,
        OptionVerdict::Viable,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, verdict)| {
        OptionEvaluation::new(
            i,
            verdict,
            DimensionScores {
                correctness: EvalLevel::Adequate,
                constraint_fit: EvalLevel::Adequate,
                complexity: EvalLevel::Adequate,
                tradeoff: EvalLevel::Adequate,
                risk: EvalLevel::Adequate,
            },
            format!("rationale {i}"),
            Confidence::High,
        )
        .expect("eval")
    })
    .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Complete,
    Fail,
    Stale,
}

struct GatedEvaluator {
    released: Arc<AtomicBool>,
    calls: Arc<Mutex<usize>>,
    mode: Mode,
}

impl GatedEvaluator {
    fn new(mode: Mode) -> (Arc<Self>, Arc<AtomicBool>, Arc<Mutex<usize>>) {
        let released = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(0));
        let evaluator = Arc::new(Self {
            released: released.clone(),
            calls: calls.clone(),
            mode,
        });
        (evaluator, released, calls)
    }
    fn immediate(mode: Mode) -> Arc<Self> {
        let (evaluator, released, _) = Self::new(mode);
        released.store(true, Ordering::SeqCst);
        evaluator
    }
}

impl ShadowEvaluator for GatedEvaluator {
    fn evaluate<'a>(
        &'a self,
        _statement: &'a str,
        set: &'a ApproachSet,
        slot: usize,
    ) -> PipelineFuture<'a, ShadowReport> {
        Box::pin(async move {
            *self.calls.lock().expect("calls") += 1;
            while !self.released.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            match self.mode {
                Mode::Fail => Err(PipelineError::Llm("boom".into())),
                Mode::Stale => Ok(ShadowReport::complete(
                    slot,
                    "fake".into(),
                    &other_set(),
                    evals(),
                )),
                Mode::Complete => Ok(ShadowReport::complete(slot, "fake".into(), set, evals())),
            }
        })
    }
}

fn session_with(choice: Option<ApproachChoice>) -> Session {
    let mut session = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    if let Some(choice) = choice {
        session.select_approach(choice, 100, 5).expect("select");
    }
    session
}

fn layout(name: &str) -> (std::path::PathBuf, ArtifactLayout) {
    let dir = tmp("shadow-runtime", name);
    let layout = ArtifactLayout::create(&dir, "s1", Language::Python).expect("layout");
    (dir, layout)
}

async fn wait_calls(calls: &Arc<Mutex<usize>>, want: usize) {
    for _ in 0..200 {
        if *calls.lock().expect("calls") >= want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    panic!("evaluator was not called {want} times");
}

async fn poll_until(
    runtime: &mut ShadowRuntime,
    session: &Session,
    layout: &ArtifactLayout,
) -> usize {
    let mut total = 0;
    for _ in 0..200 {
        total += runtime.poll("s1", session, layout);
        if total > 0 {
            return total;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    total
}

fn trigger(
    runtime: &mut ShadowRuntime,
    supervisor: &mut RequestSupervisor,
    evaluator: Arc<GatedEvaluator>,
    slot: usize,
    set: ApproachSet,
) -> bool {
    runtime.trigger(
        supervisor,
        evaluator,
        supervisor.token(),
        "s1".to_string(),
        slot,
        "Find a pair.".to_string(),
        set,
    )
}

#[tokio::test]
async fn dedup_by_slot_and_fingerprint() {
    let (evaluator, released, calls) = GatedEvaluator::new(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator.clone(),
        0,
        distinct_set()
    ));
    assert!(!trigger(
        &mut runtime,
        &mut supervisor,
        evaluator.clone(),
        0,
        distinct_set()
    ));
    wait_calls(&calls, 1).await;
    assert_eq!(*calls.lock().expect("calls"), 1);
    released.store(true, Ordering::SeqCst);
    let (_dir, layout) = layout("dedup");
    let session = session_with(None);
    assert_eq!(poll_until(&mut runtime, &session, &layout).await, 1);
    // Same set again after completion is still deduplicated.
    assert!(!trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
}

#[tokio::test]
async fn poll_is_non_blocking_while_gated() {
    let (evaluator, released, calls) = GatedEvaluator::new(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    wait_calls(&calls, 1).await;
    let (_dir, layout) = layout("non-blocking");
    let session = session_with(None);
    assert_eq!(runtime.poll("s1", &session, &layout), 0);
    assert_eq!(runtime.pending_count(), 1);
    assert!(!layout.shadow_path().exists());
    released.store(true, Ordering::SeqCst);
    assert_eq!(poll_until(&mut runtime, &session, &layout).await, 1);
}

#[tokio::test]
async fn snapshot_survives_pipeline_clear() {
    // Build a real pipeline set, snapshot it, then finish the question
    // (which clears the live set); the shadow result must still land.
    struct FakeFetcher;
    impl StatementFetcher for FakeFetcher {
        fn fetch_statement<'a>(
            &'a self,
            _slug: &'a str,
            _lang: Language,
        ) -> PipelineFuture<'a, FetchedStatement> {
            Box::pin(async move {
                Ok(FetchedStatement {
                    statement: "Find a pair.".into(),
                    starter_code: String::new(),
                })
            })
        }
    }
    struct FakeGenerator;
    impl ApproachGenerator for FakeGenerator {
        fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PipelineFuture<'a, ApproachSet> {
            Box::pin(async move { Ok(distinct_set()) })
        }
        fn grade_other<'a>(
            &'a self,
            _s: &'a str,
            _t: &'a str,
        ) -> PipelineFuture<'a, ApproachGrade> {
            Box::pin(async move { Err(PipelineError::Llm("unused".into())) })
        }
    }
    let mut orch = Orchestrator::create(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("orch");
    orch.mark_waiting_for_candidate(1).expect("waiting");
    orch.attach_candidate(2).expect("attach");
    orch.start(3).expect("start");
    let mut pipeline = QuestionPipeline::new();
    pipeline
        .load_current_statement(&orch, &FakeFetcher)
        .await
        .expect("load");
    pipeline
        .generate_current_approaches(&orch, &FakeGenerator)
        .await
        .expect("generate");
    let (slot, snapshot) = pipeline.approaches_snapshot().expect("snapshot");
    let evaluator = GatedEvaluator::immediate(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        slot,
        snapshot
    ));
    pipeline.finish_question(&mut orch, 10).expect("finish");
    assert!(pipeline.approaches_snapshot().is_none());
    let (_dir, layout) = layout("snapshot-clear");
    assert_eq!(poll_until(&mut runtime, orch.session(), &layout).await, 1);
    assert!(runtime.report(0).is_some());
}

#[tokio::test]
async fn selection_is_assessed_from_last_event() {
    let evaluator = GatedEvaluator::immediate(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("selection");
    let session = session_with(Some(ApproachChoice::Preset(0)));
    assert_eq!(poll_until(&mut runtime, &session, &layout).await, 1);
    let decision = runtime
        .report(0)
        .expect("report")
        .decision
        .clone()
        .expect("decision");
    assert_eq!(decision.standing, DecisionStanding::AmongBest);
    assert!(decision.evidence_complete);
}

#[tokio::test]
async fn selection_after_completion_is_assessed_on_next_poll() {
    let evaluator = GatedEvaluator::immediate(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("late-selection");
    let before = session_with(None);
    assert_eq!(poll_until(&mut runtime, &before, &layout).await, 1);
    assert!(runtime.report(0).expect("report").decision.is_none());
    // Candidate selects only after the background evaluation finished.
    let after = session_with(Some(ApproachChoice::Preset(0)));
    assert_eq!(runtime.poll("s1", &after, &layout), 0, "no new record");
    let decision = runtime
        .report(0)
        .expect("report")
        .decision
        .clone()
        .expect("decision assessed after selection");
    assert_eq!(decision.standing, DecisionStanding::AmongBest);
    let text = std::fs::read_to_string(layout.shadow_path()).expect("read");
    let records: Vec<serde_json::Value> = serde_json::from_str(&text).expect("records");
    assert_eq!(
        records.len(),
        1,
        "decision update must not duplicate record"
    );
}

#[tokio::test]
async fn other_selection_appends_recorded_grade() {
    let evaluator = GatedEvaluator::immediate(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    runtime.record_other(
        0,
        ApproachGrade::new(ApproachQuality::Workable, 90, "sound custom".into()).expect("grade"),
    );
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("other");
    let session = session_with(Some(ApproachChoice::Other("custom".into())));
    assert_eq!(poll_until(&mut runtime, &session, &layout).await, 1);
    let report = runtime.report(0).expect("report");
    assert_eq!(report.options.len(), 5);
    assert_eq!(report.options[4].option_index, 4);
    assert_eq!(
        report.decision.clone().expect("decision").standing,
        DecisionStanding::Defensible
    );
}

#[tokio::test]
async fn persists_only_once_on_new_key() {
    let evaluator = GatedEvaluator::immediate(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("persist-once");
    let session = session_with(None);
    assert_eq!(poll_until(&mut runtime, &session, &layout).await, 1);
    let first = std::fs::read_to_string(layout.shadow_path()).expect("read");
    assert!(first.contains("shadow-v1"));
    assert_eq!(runtime.poll("s1", &session, &layout), 0);
    let second = std::fs::read_to_string(layout.shadow_path()).expect("read");
    assert_eq!(first, second);
}

#[tokio::test]
async fn failure_is_explicit_and_scoring_unchanged() {
    let evaluator = GatedEvaluator::immediate(Mode::Fail);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    let session = session_with(Some(ApproachChoice::Preset(0)));
    let qualities = BTreeMap::from([(0usize, vec![ApproachQuality::Best])]);
    let live_before = live_rating(&session, &qualities);
    let final_before = final_rating(&session, &qualities, &[]).expect("final");
    let events_before = session.events().len();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("failure");
    assert_eq!(poll_until(&mut runtime, &session, &layout).await, 1);
    let report = runtime.report(0).expect("report");
    assert!(matches!(report.status, ShadowStatus::Failed { .. }));
    assert!(report.options.is_empty());
    assert!(report.decision.is_none());
    assert_eq!(session.events().len(), events_before);
    assert_eq!(live_rating(&session, &qualities), live_before);
    assert_eq!(
        final_rating(&session, &qualities, &[]).expect("final"),
        final_before
    );
}

#[tokio::test]
async fn cancelled_evaluation_produces_no_report() {
    let (evaluator, _released, calls) = GatedEvaluator::new(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let token: CancelToken = supervisor.token();
    let mut runtime = ShadowRuntime::new();
    assert!(runtime.trigger(
        &mut supervisor,
        evaluator,
        token.clone(),
        "s1".to_string(),
        0,
        "Find a pair.".to_string(),
        distinct_set(),
    ));
    wait_calls(&calls, 1).await;
    token.cancel();
    let (_dir, layout) = layout("cancelled");
    let session = session_with(None);
    for _ in 0..50 {
        runtime.poll("s1", &session, &layout);
        if runtime.pending_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(runtime.pending_count(), 0);
    assert!(runtime.reports().is_empty());
    assert!(!layout.shadow_path().exists());
}

#[tokio::test]
async fn stale_fingerprint_is_dropped() {
    let evaluator = GatedEvaluator::immediate(Mode::Stale);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("stale");
    let session = session_with(None);
    for _ in 0..50 {
        runtime.poll("s1", &session, &layout);
        if runtime.pending_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(runtime.reports().is_empty());
    assert!(!layout.shadow_path().exists());
}

#[tokio::test]
async fn wrong_session_outcome_is_dropped() {
    let evaluator = GatedEvaluator::immediate(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    let (_dir, layout) = layout("wrong-session");
    let session = session_with(None);
    for _ in 0..50 {
        runtime.poll("other-session", &session, &layout);
        if runtime.pending_count() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(runtime.reports().is_empty());
    assert!(!layout.shadow_path().exists());
}

#[tokio::test]
async fn shutdown_clears_pending() {
    let (evaluator, _released, calls) = GatedEvaluator::new(Mode::Complete);
    let mut supervisor = RequestSupervisor::new();
    let mut runtime = ShadowRuntime::new();
    assert!(trigger(
        &mut runtime,
        &mut supervisor,
        evaluator,
        0,
        distinct_set()
    ));
    wait_calls(&calls, 1).await;
    runtime.shutdown();
    assert_eq!(runtime.pending_count(), 0);
    let (_dir, layout) = layout("shutdown");
    let session = session_with(None);
    assert_eq!(runtime.poll("s1", &session, &layout), 0);
    assert!(runtime.reports().is_empty());
}
