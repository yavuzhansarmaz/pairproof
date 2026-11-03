//! Selection fallback and backoff checks on a deterministic fake pool.
//! Never touches the network: all pools are scripted in memory.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::selection::{
    auto_pick_easy, fallback_tag_sets, select_questions_with_fallback, PoolClient, PoolFuture,
    SelectionError,
};
use pairproof::leetcode::{CatalogEntry, CatalogPage, Difficulty};
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::llm::Message;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::{ApproachGrade, ApproachQuality, ApproachSet};
use pairproof::tui::expert_runtime::{
    ExpertRuntime, ExpertRuntimeDeps, ExpertSessionSpec, LocalHelloSource, SelectionScreen,
    SELECTION_RETRY_MS,
};
use rand::rngs::StdRng;
use rand::SeedableRng;

mod common_mod;
use common_mod::q;

const T0: u64 = 1_730_000_000_000;

/// Scripted pool source: tag set key -> slugs served for any difficulty.
struct FakePool {
    pools: HashMap<String, Vec<String>>,
    calls: Mutex<Vec<Vec<String>>>,
}

fn key(tags: &[&str]) -> String {
    tags.join("\u{1f}")
}

impl FakePool {
    fn new(pools: &[(&[&str], &[&str])]) -> Self {
        let mut map = HashMap::new();
        for (tags, slugs) in pools {
            map.insert(
                key(tags),
                slugs.iter().map(|slug| slug.to_string()).collect(),
            );
        }
        Self {
            pools: map,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<Vec<String>> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl PoolClient for FakePool {
    fn list_questions<'a>(
        &'a self,
        difficulty: Difficulty,
        topics: &'a [&'a str],
        limit: u32,
        skip: u32,
    ) -> PoolFuture<'a> {
        let tags: Vec<String> = topics.iter().map(|tag| tag.to_string()).collect();
        self.calls.lock().expect("calls lock").push(tags);
        let slugs = self.pools.get(&key(topics)).cloned().unwrap_or_default();
        let label = match difficulty {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
        }
        .to_string();
        Box::pin(async move {
            let start = (skip as usize).min(slugs.len());
            let end = start.saturating_add(limit as usize).min(slugs.len());
            let questions: Vec<CatalogEntry> = slugs[start..end]
                .iter()
                .map(|slug| CatalogEntry {
                    title: slug.clone(),
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

fn picked_slugs(picked: &[QuestionRef]) -> Vec<&str> {
    let mut slugs: Vec<&str> = picked.iter().map(|q| q.title_slug.as_str()).collect();
    slugs.sort_unstable();
    slugs
}

#[tokio::test]
async fn fallback_shrinks_tags_on_medium_shortage() {
    let fake = FakePool::new(&[
        (&["array", "hash-table", "graph"], &["m0"]),
        (&["array", "hash-table"], &["m0", "m1", "m2", "m3"]),
    ]);
    let mut rng = StdRng::seed_from_u64(11);
    let (picked, report) = select_questions_with_fallback(
        &fake,
        Difficulty::Medium,
        &["array", "hash-table", "graph"],
        3,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("shorter prefix satisfies");
    assert_eq!(picked.len(), 3);
    assert_eq!(
        report.tags_used,
        vec!["array".to_string(), "hash-table".to_string()]
    );
    assert!(report.widened);
    assert_eq!(
        fake.recorded(),
        vec![
            vec![
                "array".to_string(),
                "hash-table".to_string(),
                "graph".to_string()
            ],
            vec!["array".to_string(), "hash-table".to_string()],
        ]
    );
}

#[tokio::test]
async fn fallback_unfiltered_when_tags_exhausted() {
    let fake = FakePool::new(&[(&["graph"], &["m0"]), (&[], &["u0", "u1", "u2", "u3"])]);
    let mut rng = StdRng::seed_from_u64(21);
    let (picked, report) = select_questions_with_fallback(
        &fake,
        Difficulty::Medium,
        &["graph"],
        3,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("unfiltered pool satisfies");
    assert_eq!(picked.len(), 3);
    assert!(report.tags_used.is_empty());
    assert!(report.widened);
    assert_eq!(fake.recorded(), vec![vec!["graph".to_string()], Vec::new()]);
}

#[tokio::test]
async fn fallback_preserves_excludes() {
    let fake = FakePool::new(&[
        (&["array", "math"], &["e0", "e1", "m0"]),
        (&["array"], &["e0", "m1", "m2"]),
    ]);
    let exclude = BTreeSet::from(["e0".to_string(), "e1".to_string()]);
    let mut rng = StdRng::seed_from_u64(31);
    let (picked, report) = select_questions_with_fallback(
        &fake,
        Difficulty::Medium,
        &["array", "math"],
        2,
        &exclude,
        &mut rng,
    )
    .await
    .expect("excludes hold at every step");
    assert_eq!(picked_slugs(&picked), vec!["m1", "m2"]);
    assert_eq!(report.tags_used, vec!["array".to_string()]);
    assert!(report.widened);
}

#[tokio::test]
async fn fallback_reports_tags_used_and_widened_flag() {
    let mut rng = StdRng::seed_from_u64(41);
    let full = FakePool::new(&[(&["array", "math"], &["m0", "m1", "m2"])]);
    let (_, report) = select_questions_with_fallback(
        &full,
        Difficulty::Medium,
        &["array", "math"],
        3,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("full pool satisfies");
    assert_eq!(
        report.tags_used,
        vec!["array".to_string(), "math".to_string()]
    );
    assert!(!report.widened);
    let shrunk = FakePool::new(&[
        (&["array", "math"], &["m0"]),
        (&["array"], &["m0", "m1", "m2"]),
    ]);
    let (_, report) = select_questions_with_fallback(
        &shrunk,
        Difficulty::Medium,
        &["array", "math"],
        3,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("prefix satisfies");
    assert_eq!(report.tags_used, vec!["array".to_string()]);
    assert!(report.widened);
    let open = FakePool::new(&[
        (&["array", "math"], &[]),
        (&["array"], &[]),
        (&[], &["u0", "u1", "u2"]),
    ]);
    let (_, report) = select_questions_with_fallback(
        &open,
        Difficulty::Medium,
        &["array", "math"],
        3,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("unfiltered satisfies");
    assert!(report.tags_used.is_empty());
    assert!(report.widened);
}

#[tokio::test]
async fn fallback_first_success_wins_without_extra_calls() {
    let fake = FakePool::new(&[(&["array", "math"], &["m0", "m1", "m2", "m3"])]);
    let mut rng = StdRng::seed_from_u64(51);
    let (picked, report) = select_questions_with_fallback(
        &fake,
        Difficulty::Medium,
        &["array", "math"],
        3,
        &BTreeSet::new(),
        &mut rng,
    )
    .await
    .expect("full pool satisfies");
    assert_eq!(picked.len(), 3);
    assert!(!report.widened);
    assert_eq!(fake.recorded().len(), 1, "happy path makes one pool call");
    assert_eq!(fallback_tag_sets(&["array", "math"]).len(), 3);
}

#[tokio::test]
async fn unfiltered_shortage_still_errors() {
    let fake = FakePool::new(&[
        (&["array", "math"], &["m0"]),
        (&["array"], &["m0"]),
        (&[], &["u0"]),
    ]);
    let mut rng = StdRng::seed_from_u64(61);
    let result = select_questions_with_fallback(
        &fake,
        Difficulty::Hard,
        &["array", "math"],
        2,
        &BTreeSet::new(),
        &mut rng,
    )
    .await;
    assert!(matches!(
        result,
        Err(SelectionError::NotEnoughQuestions {
            needed: 2,
            available: 1
        })
    ));
    assert_eq!(fake.recorded().len(), 3, "every tag set is attempted once");
}

#[tokio::test]
async fn setup_auto_easy_falls_back_on_narrow_topics() {
    let topics = ["array", "string", "hash-table", "sorting", "greedy"];
    let fake = FakePool::new(&[(&[], &["e0", "e1", "e2", "e3", "e4", "e5"])]);
    let mut rng = StdRng::seed_from_u64(71);
    let picked = auto_pick_easy(&fake, &topics, &mut rng)
        .await
        .expect("narrow easy intersection falls back to unfiltered");
    assert_eq!(picked.len(), 5);
    assert!(picked.iter().all(|q| q.difficulty == Difficulty::Easy));
    assert_eq!(fake.recorded().len(), topics.len() + 1);
    assert!(fake.recorded().last().expect("last call").is_empty());
}

type PF<'a, T> = PipelineFuture<'a, T>;

#[rustfmt::skip]
fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("pairproof-sel-pool-{}-{name}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }

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
async fn make_runtime(name: &str, topics: Vec<String>, pool: Arc<FakePool>) -> ExpertRuntime {
    let root = tmp(name); let control = root.join("control"); let artifacts = root.join("artifacts");
    let io = LocalControlIo::new(control.clone()); io.setup_dirs("pp-testuser").await.unwrap();
    let spec = ExpertSessionSpec { session_id: "s1".into(), language: Language::Python, easy: (0..5).map(|i| q(&format!("e{i}"), Difficulty::Easy)).collect(), endpoint: Endpoint::new("203.0.113.10", 22022).unwrap(), username: EphemeralUsername::parse("pp-a1b2c3d4").unwrap(), fingerprint: Fingerprint::parse(&"a".repeat(64)).unwrap() };
    let deps = ExpertRuntimeDeps { control_io: Box::new(io), hello: Box::new(LocalHelloSource::new(control.clone())), fetcher: Arc::new(FakeFetcher), generator: Arc::new(FakeGenerator), coding: Arc::new(FakeCoding), sandbox_io: Arc::new(FakeSandbox), chat: Arc::new(FakeChat), grader: Arc::new(FakeGrader), leetcode: None, sandbox_handle: None, artifact_root: artifacts.clone(), topics, level: String::new(), sync_phase: SessionSyncPhase::NotStarted, shadow_evaluator: None };
    let mut runtime = ExpertRuntime::new(spec, deps, T0).await.unwrap(); runtime.set_pool(pool as Arc<dyn PoolClient>); runtime
}

#[tokio::test]
async fn selection_backoff_allows_immediate_first_attempt() {
    let pool = Arc::new(FakePool::new(&[]));
    let runtime = make_runtime("backoff-first", vec!["array".into()], pool).await;
    assert!(runtime.selection_allowed(SelectionScreen::Medium, T0));
    assert!(runtime.selection_allowed(SelectionScreen::Hard, T0));
    assert!(runtime.selection_note().is_none());
}

#[tokio::test]
async fn selection_backoff_suppresses_hot_loop() {
    let pool = Arc::new(FakePool::new(&[]));
    let topics = vec!["array".into(), "math".into()];
    let mut runtime = make_runtime("backoff-hot", topics, pool.clone()).await;
    let mut attempts = 0u32;
    for tick in 0..(SELECTION_RETRY_MS * 2 / 250) {
        let now = T0 + tick * 250;
        if runtime.selection_allowed(SelectionScreen::Medium, now) {
            attempts += 1;
            assert!(runtime.fetch_medium(now).await.is_err());
        }
    }
    assert_eq!(attempts, 2, "one attempt per 30s window");
    assert_eq!(pool.recorded().len(), 6, "each attempt walks 3 tag sets");
    assert!(
        runtime.selection_allowed(SelectionScreen::Hard, T0 + SELECTION_RETRY_MS * 2 - 1),
        "screens back off independently"
    );
}

#[tokio::test]
async fn fetch_medium_caches_pending_without_refetch() {
    let pool = Arc::new(FakePool::new(&[(&["array"], &["m0", "m1", "m2", "m3"])]));
    let mut runtime = make_runtime("medium-cache", vec!["array".into()], pool.clone()).await;
    // The fetch now offers the whole pool (4) for the expert to pick
    // 3 from, instead of auto-picking 3.
    assert_eq!(runtime.fetch_medium(T0).await.expect("medium fetch"), 4);
    assert_eq!(pool.recorded().len(), 1);
    assert!(
        runtime.selection_note().is_none(),
        "happy path sets no note"
    );
    assert_eq!(runtime.fetch_medium(T0 + 1_000).await.expect("cached"), 4);
    assert_eq!(pool.recorded().len(), 1, "cached fetch makes no pool call");
}
