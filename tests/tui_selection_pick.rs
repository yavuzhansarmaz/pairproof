//! Expert list selection: paged pool, cursor/page movement, picking
//! exactly 3 medium then 2 hard, and the atomic commit. Fake pool only.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
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

use crate::common_mod;
use common_mod::q;
const T0: u64 = 1_730_000_000_000;

struct FakePool {
    slugs: Vec<String>,
    hard_slugs: Option<Vec<String>>,
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
        let slugs = match (&self.hard_slugs, difficulty) {
            (Some(hard), Difficulty::Hard) => hard.clone(),
            _ => self.slugs.clone(),
        };
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

#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-sel-pick-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

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
async fn make_runtime(name: &str, pool: Arc<FakePool>) -> ExpertRuntime {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: (0..5).map(|i| q(&format!("e{i}"), Difficulty::Easy)).collect(), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics: vec!["array".into()], level: String::new(), sync_phase: SessionSyncPhase::NotStarted, shadow_evaluator: None };
    let mut runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap(); runtime.set_pool(pool as Arc<dyn PoolClient>); runtime
}

fn slugs(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("q{i}")).collect()
}

#[tokio::test]
async fn randomize_picks_draws_the_effective_count() {
    let pool = Arc::new(FakePool {
        slugs: slugs(12),
        hard_slugs: None,
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime("random", pool).await;
    runtime.fetch_medium(T0).await.expect("medium fetch");
    // Drawing replaces any manual picks and lands exactly 3 unique.
    pick_after_move(&mut runtime, SelectionScreen::Medium, 0);
    assert_eq!(runtime.randomize_picks(SelectionScreen::Medium), 3);
    assert_eq!(runtime.pick_count(SelectionScreen::Medium), 3);
    runtime
        .confirm_medium()
        .expect("random medium picks confirm");
    runtime.fetch_hard(T0).await.expect("hard fetch");
    assert_eq!(runtime.randomize_picks(SelectionScreen::Hard), 2);
    runtime.complete_selection(T0).await.expect("commit");
    assert_eq!(runtime.orchestrator().session().slots().len(), 10);
}

#[tokio::test]
async fn randomize_draws_from_whole_pool_not_only_current_page() {
    let pool = Arc::new(FakePool {
        slugs: slugs(15),
        hard_slugs: None,
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime("random-whole-pool", pool).await;
    assert_eq!(runtime.fetch_medium(T0).await.expect("medium fetch"), 15);
    runtime.move_selection_page(SelectionScreen::Medium, 1);
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 10);

    // From page 2 (q10..q14), a page-local draw could never mark a
    // question on page 1. A whole-pool draw does so almost at once;
    // 40 draws make a false failure effectively impossible.
    let mut drew_outside_current_page = false;
    for _ in 0..40 {
        assert_eq!(runtime.randomize_picks(SelectionScreen::Medium), 3);
        runtime.move_selection_page(SelectionScreen::Medium, -1);
        let first_page = runtime.selection_lines(SelectionScreen::Medium);
        drew_outside_current_page = first_page.iter().any(|line| line.contains("[x]"));
        runtime.move_selection_page(SelectionScreen::Medium, 1);
        if drew_outside_current_page {
            break;
        }
    }
    assert!(
        drew_outside_current_page,
        "random picks stayed confined to the visible page"
    );
}

#[tokio::test]
async fn small_pools_auto_pick_everything_and_commit() {
    // Only 2 medium and 1 hard exist for these topics: both pools
    // arrive fully picked, and the session commits over 8 questions.
    let pool = Arc::new(FakePool {
        slugs: vec!["m0".into(), "m1".into()],
        hard_slugs: Some(vec!["h0".into()]),
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime("small-pools", pool).await;
    assert_eq!(runtime.fetch_medium(T0).await.expect("medium fetch"), 2);
    assert_eq!(runtime.pick_count(SelectionScreen::Medium), 2);
    let lines = runtime.selection_lines(SelectionScreen::Medium);
    assert!(lines[0].contains("selected: 2/2"), "{lines:?}");
    assert!(runtime.auto_selected(SelectionScreen::Medium));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("all available questions are selected automatically.")),
        "{lines:?}"
    );
    // Auto-selected picks cannot be dropped with space: the pool
    // holds no replacement questions.
    assert!(!runtime.toggle_selection_pick(SelectionScreen::Medium));
    assert_eq!(runtime.pick_count(SelectionScreen::Medium), 2);
    runtime.confirm_medium().expect("auto-picked medium");
    assert_eq!(runtime.fetch_hard(T0).await.expect("hard fetch"), 1);
    assert_eq!(runtime.pick_count(SelectionScreen::Hard), 1);
    runtime.complete_selection(T0).await.expect("commit");
    assert_eq!(runtime.orchestrator().session().slots().len(), 8);
}

/// Picks the question under the cursor after moving `delta` from it.
fn pick_after_move(runtime: &mut ExpertRuntime, screen: SelectionScreen, delta: i32) {
    runtime.move_selection_cursor(screen, delta);
    assert!(runtime.toggle_selection_pick(screen));
}

#[tokio::test]
async fn medium_list_pages_by_ten_and_moves_cursor() {
    let pool = Arc::new(FakePool {
        slugs: slugs(23),
        hard_slugs: None,
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime("pages", pool).await;
    assert_eq!(runtime.fetch_medium(T0).await.expect("fetch"), 23);
    let lines = runtime.selection_lines(SelectionScreen::Medium);
    assert_eq!(lines[0], "page 1/3, selected: 0/3");
    assert_eq!(lines.len(), 11, "page header plus 10 questions: {lines:?}");
    assert!(lines[1].starts_with("> 1. [ ] Title q0"), "{lines:?}");
    runtime.move_selection_page(SelectionScreen::Medium, 1);
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 10);
    let lines = runtime.selection_lines(SelectionScreen::Medium);
    assert_eq!(lines[0], "page 2/3, selected: 0/3");
    assert!(lines[1].starts_with("> 11."), "{lines:?}");
    runtime.move_selection_page(SelectionScreen::Medium, 5);
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 20);
    runtime.move_selection_cursor(SelectionScreen::Medium, 99);
    assert_eq!(runtime.selection_cursor(SelectionScreen::Medium), 22);
    let lines = runtime.selection_lines(SelectionScreen::Medium);
    assert_eq!(lines.len(), 4, "last page holds the remaining 3: {lines:?}");
}

#[tokio::test]
async fn expert_picks_three_medium_then_two_hard_and_commits() {
    let pool = Arc::new(FakePool {
        slugs: slugs(12),
        hard_slugs: None,
        calls: Mutex::new(0),
    });
    let mut runtime = make_runtime("picks", pool).await;
    runtime.fetch_medium(T0).await.expect("medium fetch");
    // Confirming early warns how many more are needed, and picking
    // a 4th is refused.
    let err = runtime.confirm_medium().expect_err("no picks yet");
    assert!(
        err.to_string()
            .contains("select 3 more medium questions from the list (0 of 3 selected)"),
        "{err}"
    );
    // The question under the cursor previews with its metadata,
    // then its fetched statement once stored.
    let preview = runtime.selection_preview_lines(SelectionScreen::Medium);
    assert!(preview[0].contains("Title q0"), "{preview:?}");
    assert!(
        preview
            .iter()
            .any(|line| line.contains("loading the question...")),
        "{preview:?}"
    );
    runtime.store_preview(
        SelectionScreen::Medium,
        "q0".to_string(),
        "Given an array, find two numbers.".to_string(),
    );
    let preview = runtime.selection_preview_lines(SelectionScreen::Medium);
    assert!(
        preview
            .iter()
            .any(|line| line.contains("Given an array, find two numbers.")),
        "{preview:?}"
    );
    assert!(
        preview
            .iter()
            .any(|line| line.contains("b back to the list")),
        "{preview:?}"
    );
    pick_after_move(&mut runtime, SelectionScreen::Medium, 0);
    pick_after_move(&mut runtime, SelectionScreen::Medium, 1);
    let err = runtime.confirm_medium().expect_err("only two picked");
    assert!(
        err.to_string()
            .contains("select 1 more medium question from the list (2 of 3 selected)"),
        "{err}"
    );
    pick_after_move(&mut runtime, SelectionScreen::Medium, 1);
    assert_eq!(runtime.pick_count(SelectionScreen::Medium), 3);
    runtime.move_selection_cursor(SelectionScreen::Medium, 1);
    assert!(!runtime.toggle_selection_pick(SelectionScreen::Medium));
    // Toggling a picked question frees its slot again.
    runtime.move_selection_cursor(SelectionScreen::Medium, -1);
    assert!(runtime.toggle_selection_pick(SelectionScreen::Medium));
    assert_eq!(runtime.pick_count(SelectionScreen::Medium), 2);
    assert!(runtime.toggle_selection_pick(SelectionScreen::Medium));
    runtime.confirm_medium().expect("three medium picked");

    // Hard pool excludes the picked medium slugs (q0, q1, q2 here).
    assert_eq!(runtime.fetch_hard(T0).await.expect("hard fetch"), 9);
    runtime
        .complete_selection(T0)
        .await
        .expect_err("no hard picks yet");
    pick_after_move(&mut runtime, SelectionScreen::Hard, 0);
    pick_after_move(&mut runtime, SelectionScreen::Hard, 1);
    runtime.complete_selection(T0).await.expect("commit");
    let slots = runtime.orchestrator().session().slots();
    assert_eq!(slots.len(), 10);
    let expert: Vec<&str> = slots[5..]
        .iter()
        .map(|slot| slot.question.title_slug.as_str())
        .collect();
    assert_eq!(expert, vec!["q0", "q1", "q2", "q3", "q4"]);
    assert!(slots[5..8]
        .iter()
        .all(|slot| slot.question.difficulty == Difficulty::Medium));
    assert!(slots[8..]
        .iter()
        .all(|slot| slot.question.difficulty == Difficulty::Hard));
}
