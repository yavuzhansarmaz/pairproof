//! Question preview stepping: left/right moves between offered
//! questions, clamped at both ends, via the runtime. Fake pool only.

use std::sync::{Arc, Mutex};

use pairproof::core::{Language, Session};
use pairproof::leetcode::selection::{PoolClient, PoolFuture};
use pairproof::leetcode::{CatalogEntry, CatalogPage, Difficulty};
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, SelectionScreen,
};

mod common_mod;
use common_mod::{easy_five, endpoint, fingerprint, username};

const T0: u64 = 1_730_000_000_000;

struct FakePool {
    slugs: Vec<String>,
    calls: Mutex<usize>,
}

impl PoolClient for FakePool {
    fn list_questions<'a>(
        &'a self,
        difficulty: Difficulty,
        _topics: &'a [&'a str],
        limit: u32,
        skip: u32,
    ) -> PoolFuture<'a> {
        *self.calls.lock().expect("calls lock") += 1;
        let label = match difficulty {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
        }
        .to_string();
        let slugs = self.slugs.clone();
        Box::pin(async move {
            let start = (skip as usize).min(slugs.len());
            let end = start.saturating_add(limit as usize).min(slugs.len());
            let questions: Vec<CatalogEntry> = slugs[start..end]
                .iter()
                .map(|slug| CatalogEntry {
                    title: format!("Title {slug}"),
                    title_slug: slug.clone(),
                    difficulty: label.clone(),
                    ac_rate: 50.0,
                    is_paid_only: false,
                    topic_tags: Vec::new(),
                })
                .collect();
            Ok(CatalogPage {
                total_reported: slugs.len() as u64,
                questions,
            })
        })
    }
}

type PF<'a, T> = PipelineFuture<'a, T>;

struct FakeFetcher;
#[rustfmt::skip]
impl StatementFetcher for FakeFetcher { fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> { let s = slug.to_string(); Box::pin(async move { Ok(FetchedStatement { statement: format!("Statement for {s}"), starter_code: "# starter".into() }) }) } }
#[derive(Default)]
struct FakeGenerator;
#[rustfmt::skip]
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> { Box::pin(async move { let opt = |t: &str, quality: ApproachQuality, tok: u64| pairproof::scoring::ApproachOption { title: t.into(), summary: format!("{t} summary"), quality, expected_tokens: tok }; ApproachSet::new(vec![opt("Brute force", ApproachQuality::Weak, 111), opt("Hash map", ApproachQuality::Best, 424242), opt("Sort first", ApproachQuality::Workable, 333), opt("Two pointers", ApproachQuality::Workable, 444)]).map_err(|e| PipelineError::Llm(e.to_string())) }) }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> { Box::pin(async move { ApproachGrade::new(ApproachQuality::Workable, 250, "sound".into()).map_err(|e| PipelineError::Llm(e.to_string())) }) }
}
#[derive(Default)]
struct FakeCoding;
#[rustfmt::skip]
impl CodingTransport for FakeCoding { fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CodingStepInput) -> PF<'a, CodingStepOutput> { Box::pin(async move { s.add_tokens(10).map_err(|e| PipelineError::Llm(e.to_string()))?; Ok(CodingStepOutput { code: "def solve():\n    return 42".into(), ready: false, summary: "draft".into() }) }) } }
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
impl ChatTransport for FakeChat { fn chat<'a>(&'a self, _s: &'a mut Session, _st: &'a pairproof::llm::chat::ChatState, _h: &'a [Message], _m: &'a str, _at: u64) -> PF<'a, String> { Box::pin(async move { Ok("Think about a map.".into()) }) } }
struct FakeGrader;
#[rustfmt::skip]
impl QuestionGrader for FakeGrader { fn grade_all<'a>(&'a self, _inputs: &'a [pairproof::llm::grading::QuestionScoringInput]) -> GradeFuture<'a> { Box::pin(async move { Vec::new() }) } }

#[rustfmt::skip]
async fn make_runtime(pool: Arc<FakePool>) -> ExpertRuntime {
    make_runtime_named(pool, "step").await
}

async fn make_runtime_named(pool: Arc<FakePool>, name: &str) -> ExpertRuntime {
    let root = common_mod::tmp("preview-step", name);
    let control = root.join("control");
    let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone());
    io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec {
        session_id: "s1".into(),
        language: Language::Python,
        easy: easy_five(),
        endpoint: endpoint(),
        username: username(),
        fingerprint: fingerprint(),
    };
    let deps = ExpertRuntimeDeps {
        control_io: Box::new(io),
        hello: Box::new(LocalHelloSource::new(control.clone())),
        fetcher: Arc::new(FakeFetcher),
        generator: Arc::new(FakeGenerator),
        coding: Arc::new(FakeCoding),
        sandbox_io: Arc::new(FakeSandbox),
        chat: Arc::new(FakeChat),
        grader: Arc::new(FakeGrader),
        leetcode: None,
        sandbox_handle: None,
        artifact_root: artifacts,
        topics: vec!["array".into()],
        level: String::new(),
        sync_phase: SessionSyncPhase::NotStarted,
        shadow_evaluator: None,
    };
    let mut runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap();
    runtime.set_pool(pool as Arc<dyn PoolClient>);
    runtime
}

#[tokio::test]
async fn preview_step_moves_between_questions_and_clamps() {
    let pool = Arc::new(FakePool {
        slugs: (0..5).map(|i| format!("q{i}")).collect(),
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime(pool).await;
    runtime.fetch_medium(T0).await.expect("medium fetch");
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 0);
    // Left at the first question stays; right advances one by one.
    assert!(!runtime.step_preview_question(SelectionScreen::Medium, -1));
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 0);
    assert!(runtime.step_preview_question(SelectionScreen::Medium, 1));
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 1);
    assert!(runtime.step_preview_question(SelectionScreen::Medium, 1));
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 2);
    assert!(runtime.step_preview_question(SelectionScreen::Medium, -1));
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 1);
    // The preview follows the cursor.
    let preview = runtime
        .question_preview(SelectionScreen::Medium)
        .expect("preview");
    assert_eq!(preview.position, 2);
    assert_eq!(preview.total, 5);
    // Right clamps at the last question instead of wrapping.
    for _ in 0..10 {
        runtime.step_preview_question(SelectionScreen::Medium, 1);
    }
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 4);
    assert!(!runtime.step_preview_question(SelectionScreen::Medium, 1));
}

#[tokio::test]
async fn preview_pick_toggle_and_statement_cache_survive_stepping() {
    let pool = Arc::new(FakePool {
        slugs: (0..5).map(|i| format!("q{i}")).collect(),
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime_named(pool, "pick-cache").await;
    runtime.fetch_medium(T0).await.expect("medium fetch");
    // Space in the preview is the same toggle as space in the list.
    let before = runtime
        .question_preview(SelectionScreen::Medium)
        .expect("preview");
    assert!(!before.picked);
    assert!(runtime.toggle_selection_pick(SelectionScreen::Medium));
    let after = runtime
        .question_preview(SelectionScreen::Medium)
        .expect("preview");
    assert!(after.picked);
    assert_eq!(runtime.pick_count(SelectionScreen::Medium), 1);
    // Statements are cached per question: stepping away and back
    // shows the first statement again without a new fetch.
    runtime.store_preview(
        SelectionScreen::Medium,
        "q0".to_string(),
        "Statement zero.".to_string(),
    );
    assert!(runtime.step_preview_question(SelectionScreen::Medium, 1));
    runtime.store_preview(
        SelectionScreen::Medium,
        "q1".to_string(),
        "Statement one.".to_string(),
    );
    assert!(runtime.step_preview_question(SelectionScreen::Medium, -1));
    let back = runtime
        .question_preview(SelectionScreen::Medium)
        .expect("preview");
    assert_eq!(back.body.as_deref(), Some("Statement zero."));
    assert!(runtime.preview_request(SelectionScreen::Medium).is_none());
}

#[tokio::test]
async fn preview_load_failure_is_not_cached_and_stepping_retries() {
    let pool = Arc::new(FakePool {
        slugs: (0..5).map(|i| format!("q{i}")).collect(),
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime_named(pool, "load-error").await;
    runtime.fetch_medium(T0).await.expect("medium fetch");
    // A failed load records the reason and blocks the per-tick
    // auto-retry, but is never cached as the statement itself.
    runtime.store_preview_error(
        SelectionScreen::Medium,
        "q0".to_string(),
        "premium-only question".to_string(),
    );
    let failed = runtime
        .question_preview(SelectionScreen::Medium)
        .expect("preview");
    assert_eq!(failed.load_error.as_deref(), Some("premium-only question"));
    assert!(failed.body.is_none());
    assert!(runtime.preview_request(SelectionScreen::Medium).is_none());
    // Stepping away and back clears the failure and retries.
    assert!(runtime.step_preview_question(SelectionScreen::Medium, 1));
    assert!(runtime.step_preview_question(SelectionScreen::Medium, -1));
    assert!(runtime.preview_request(SelectionScreen::Medium).is_some());
    // A later success replaces the failure state.
    runtime.store_preview_error(
        SelectionScreen::Medium,
        "q0".to_string(),
        "timeout".to_string(),
    );
    runtime.store_preview(
        SelectionScreen::Medium,
        "q0".to_string(),
        "Statement zero.".to_string(),
    );
    let loaded = runtime
        .question_preview(SelectionScreen::Medium)
        .expect("preview");
    assert!(loaded.load_error.is_none());
    assert_eq!(loaded.body.as_deref(), Some("Statement zero."));
}
