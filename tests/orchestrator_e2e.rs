use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::ArtifactLayout as A;
use pairproof::core::*;
use pairproof::leetcode::Difficulty;
use pairproof::llm::chat::{build_live_state_block, ChatState as CS};
use pairproof::llm::coding::{parse_coding_step, CodingStepInput as CSI, CodingStepOutput as CSO};
use pairproof::llm::grading::QuestionScoringInput as QSI;
use pairproof::llm::Message as Msg;
use pairproof::orchestration::Orchestrator as O;
use pairproof::orchestration::*;
use pairproof::sandbox::{RunnabilityReport, TestOutcome};
use pairproof::scoring::ApproachQuality::{Best, Weak, Workable};
use pairproof::scoring::GradeOutcome as G;
use pairproof::scoring::*;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
type PF<'a, T> = PipelineFuture<'a, T>;
type QMap = BTreeMap<usize, Vec<ApproachQuality>>;
type Scripted = (
    A,
    O,
    String,
    Vec<G>,
    QMap,
    bool,
    Vec<String>,
    bool,
    String,
    Vec<Msg>,
);
const API: &str = "[REDACTED]";
const SECRET: &str = "conn-secret-CANARY-1a2b3c";
const JD: &str = "JD-RAW-CANARY-4d5e6f";
const T0: u64 = 1_730_000_000_000;
const CODE: &str = r#"{"code":"def solve():\n    return 42","ready":true,"summary":"complete"}"#;
const HEADS: &str = "# PairProof Session Report|## Final breakdown|## Per-question grades|## Follow-up suggestions|## Notes";
const WANTS: &str =
    "SessionCreated|CandidateConnected|ApproachSelected|TestsRequested|QuestionFinished";
fn q(s: &str, d: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: s.into(),
        title: s.into(),
        difficulty: d,
        acceptance_pct: None,
    }
}
fn qs(p: &str, n: usize, d: Difficulty) -> Vec<QuestionRef> {
    (0..n).map(|i| q(&format!("{p}{i}"), d)).collect()
}
fn opt(t: &str, s: &str, quality: ApproachQuality, tok: u64) -> ApproachOption {
    ApproachOption {
        title: t.into(),
        summary: s.into(),
        quality,
        expected_tokens: tok,
    }
}
fn raw_outcome() -> TestOutcome {
    TestOutcome {
        success: false,
        exit_code: Some(1),
        stdout: "raw out".into(),
        stderr: "raw err".into(),
        stdout_truncated: true,
        stderr_truncated: false,
    }
}
fn tmp_root(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pairproof-e2e-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp");
    d
}
const HIDE: [&str; 7] = ["1111", "2222", "3333", "4444", "Best", "Workable", "Weak"];
fn hide(s: &str) -> bool {
    HIDE.iter().any(|h| s.contains(h))
}
fn go(i: &QSI) -> GradeOutcome {
    if i.slot_index == 1 {
        return GradeOutcome::NotScored { slot_index: 1 };
    }
    let g = QuestionGrade::new(i.slot_index, 8.0, format!("r{}", i.slot_index), 0.05);
    GradeOutcome::Graded(g.expect("grade"))
}
#[derive(Debug)]
struct OkVerifier;
impl HostKeyVerifier for OkVerifier {
    fn verify_host_key(&self, _e: &Endpoint, _f: &Fingerprint) -> Result<(), ConnectionGateError> {
        Ok(())
    }
}
struct FakeFetcher;
impl StatementFetcher for FakeFetcher {
    fn fetch_statement<'a>(&'a self, slug: &'a str, _l: Language) -> PF<'a, FetchedStatement> {
        let s = slug.to_string();
        Box::pin(async move {
            Ok(FetchedStatement {
                statement: format!("Statement for {s}: solve it."),
                starter_code: format!("# starter {s}"),
            })
        })
    }
}
struct FakeGenerator;
impl ApproachGenerator for FakeGenerator {
    fn generate<'a>(&'a self, _s: &'a str, _l: Language) -> PF<'a, ApproachSet> {
        Box::pin(async move {
            ApproachSet::new(vec![
                opt("Hash map", "One pass", Best, 1111),
                opt("Brute force", "Try pairs", Workable, 2222),
                opt("Sort first", "Sort scan", Weak, 3333),
                opt("Two pointers", "Converge", Workable, 4444),
            ])
            .map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
    fn grade_other<'a>(&'a self, _s: &'a str, _t: &'a str) -> PF<'a, ApproachGrade> {
        Box::pin(async move {
            ApproachGrade::new(Workable, 2222, "sound".into())
                .map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
}
struct FakeCoding;
impl CodingTransport for FakeCoding {
    fn coding_step<'a>(&'a self, s: &'a mut Session, _i: &'a CSI) -> PF<'a, CSO> {
        Box::pin(async move {
            s.add_tokens(50)
                .map_err(|e| PipelineError::Llm(e.to_string()))?;
            parse_coding_step(CODE).map_err(|e| PipelineError::Llm(e.to_string()))
        })
    }
}
#[derive(Default)]
struct FakeSandbox {
    writes: Mutex<Vec<(usize, String)>>,
    runnable: AtomicU32,
    tests: AtomicU32,
}
impl SandboxIo for FakeSandbox {
    fn write_solution<'a>(&'a self, i: usize, _l: Language, c: &'a str) -> PF<'a, ()> {
        let c = c.to_string();
        Box::pin(async move {
            self.writes.lock().unwrap().push((i, c));
            Ok(())
        })
    }
    fn read_solution<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, String> {
        Box::pin(async move { Ok(String::new()) })
    }
    fn check_runnable<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, RunnabilityReport> {
        Box::pin(async move {
            self.runnable.fetch_add(1, Ordering::SeqCst);
            Ok(RunnabilityReport {
                runnable: true,
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            })
        })
    }
    fn run_requested_tests<'a>(&'a self, _i: usize, _l: Language) -> PF<'a, TestOutcome> {
        Box::pin(async move {
            self.tests.fetch_add(1, Ordering::SeqCst);
            Ok(raw_outcome())
        })
    }
}
struct FakeChat;
impl ChatTransport for FakeChat {
    fn chat<'a>(
        &'a self,
        _s: &'a mut Session,
        _st: &'a CS,
        _h: &'a [Msg],
        _u: &'a str,
        _at: u64,
    ) -> PF<'a, String> {
        Box::pin(async move { Ok("Consider edge cases and invariants.".into()) })
    }
}
struct FakeGrader;
impl QuestionGrader for FakeGrader {
    fn grade_all<'a>(&'a self, inputs: &'a [QSI]) -> GradeFuture<'a> {
        Box::pin(async move { inputs.iter().map(go).collect() })
    }
}
fn rec(name: &'static str, log: Arc<Mutex<Vec<String>>>) -> CleanupStep {
    Box::new(move || {
        let log = log.clone();
        Box::pin(async move {
            log.lock().unwrap().push(name.to_string());
            Ok(())
        })
    })
}
async fn run_scripted(name: &str) -> Scripted {
    let root = tmp_root(name);
    let easy = qs("e", 5, Difficulty::Easy);
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy, T0).expect("create");
    o.mark_waiting_for_candidate(T0 + 1).expect("waiting");
    let ep = Endpoint::new("203.0.113.10", 22022).expect("ep");
    let un = EphemeralUsername::parse("pp-a1b2c3d4").expect("user");
    let fp = Fingerprint::parse(&"a".repeat(64)).expect("fp");
    let mut gate = ConnectionGate::with_verifier(ep, un, fp, OkVerifier, T0);
    let presented = gate.current_string(T0).expect("string");
    gate.accept(&mut o, &presented, T0 + 2).expect("accept");
    let ev = o.session().events();
    let cc = EventKind::CandidateConnected;
    assert_eq!(ev.iter().filter(|e| e.kind == cc).count(), 1);
    o.start(T0 + 3).expect("start");
    let (med, hard) = (qs("m", 3, Difficulty::Medium), qs("h", 2, Difficulty::Hard));
    o.set_expert_questions(med, hard, T0 + 4).expect("ex");
    let fetcher = FakeFetcher;
    let generator = FakeGenerator;
    let coding = FakeCoding;
    let sb = FakeSandbox::default();
    let chat = FakeChat;
    let mut pipe = QuestionPipeline::new();
    let mut now = T0 + 100;
    let mut statements: HashMap<String, String> = HashMap::new();
    let mut qm: QMap = BTreeMap::new();
    let mut hist: Vec<Msg> = Vec::new();
    for idx in 0..10usize {
        let slug = o.current_question().expect("q").title_slug.clone();
        let fut = pipe.load_current_statement(&o, &fetcher);
        let f = fut.await.expect("fetch");
        assert_eq!(f.statement, format!("Statement for {slug}: solve it."));
        statements.insert(slug, f.statement.clone());
        let fut = pipe.generate_current_approaches(&o, &generator);
        let views = fut.await.expect("gen");
        assert!(!hide(&format!("{views:?}")));
        assert_eq!(views, pipe.candidate_options());
        now += 10;
        let rb = sb.runnable.load(Ordering::SeqCst);
        let tb = sb.tests.load(Ordering::SeqCst);
        let fut = pipe.select_preset(&mut o, 0, &coding, &sb, now);
        let out = fut.await.expect("select");
        assert!(out.runnable && out.ready);
        let r2 = sb.runnable.load(Ordering::SeqCst);
        let t2 = sb.tests.load(Ordering::SeqCst);
        assert_eq!((r2, t2), (rb + 1, tb));
        let contains = sb.writes.lock().unwrap().contains(&(idx, out.code.clone()));
        assert!(contains);
        qm.insert(idx, vec![Best]);
        now += 10;
        if idx == 0 {
            let fut = pipe.redirect_coding(&mut o, "use a heap", &coding, &sb, now);
            fut.await.expect("redirect");
        }
        if idx == 1 {
            let fut = pipe.continue_coding(&mut o, &coding, &sb, now);
            fut.await.expect("continue");
        }
        now += 10;
        let fut = pipe.request_tests(&mut o, &sb, now);
        let rec = fut.await.expect("tests");
        assert_eq!(rec.outcome, raw_outcome());
        assert_eq!(pipe.test_records().last(), Some(&rec));
        assert!(o
            .session()
            .current_slot()
            .expect("slot")
            .latest_test
            .is_none());
        let ev = o.session().events();
        let mut iter = ev.iter();
        let any_tc = iter.any(|e| matches!(e.kind, EventKind::TestRunCompleted { .. }));
        assert!(!any_tc);
        if idx < 2 {
            let code = pipe.current_code(idx).unwrap_or("");
            let st = CS::from_session(o.session(), &f.statement, code);
            assert!(!hide(&build_live_state_block(&st)));
            now += 10;
            let fut = pipe.submit_chat(&mut o, &chat, &mut hist, "hint please", now);
            let reply = fut.await.expect("chat");
            assert_eq!(reply, "Consider edge cases and invariants.");
        }
        now += 10;
        pipe.finish_question(&mut o, now).expect("finish");
    }
    assert_eq!(o.status(), SessionStatus::Scoring);
    let codes: HashMap<usize, String> = (0..10)
        .map(|i| (i, pipe.current_code(i).expect("code").to_string()))
        .collect();
    let fut = grade_session(&o, &FakeGrader, &statements, &codes);
    let gr = fut.await.expect("grade");
    assert_eq!(gr[1], GradeOutcome::NotScored { slot_index: 1 });
    let ex = compute_final(&o, &qm, &gr).expect("final");
    let (_, bf) = both_roles_final(&o, &qm, &gr).expect("both");
    now += 10;
    o.mark_completed(now).expect("completed");
    let rep = generate_report(o.session(), &ex, &gr, &[]);
    let lay = ArtifactLayout::create(root.join("root"), "s1", Language::Python).expect("lay");
    std::fs::write(lay.raw_dir().join("jd.txt"), JD).expect("raw");
    let local = root.join("candidate-local/questions/01/solution.py");
    std::fs::create_dir_all(local.parent().unwrap()).expect("local dir");
    std::fs::write(&local, "keep me\n").expect("local file");
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let flag = Arc::new(AtomicBool::new(false));
    let (lay2, flag2, log2) = (lay.clone(), flag.clone(), log.clone());
    let destroy: CleanupStep = Box::new(move || {
        let (lay, flag, log) = (lay2.clone(), flag2.clone(), log2.clone());
        Box::pin(async move {
            let a = lay.report_path().exists();
            let b = lay.events_path().exists();
            let c = lay.solution_path(0).exists();
            flag.store(a && b && c, Ordering::SeqCst);
            log.lock().unwrap().push("destroy_sandbox".to_string());
            Ok(())
        })
    });
    let a = rec("cancel_inflight", log.clone());
    let b = rec("terminate_connection", log.clone());
    let c = rec("stop_sync", log.clone());
    let e = rec("delete_raw", log.clone());
    let mut coord = CleanupCoordinator::with_production_steps(a, b, c, destroy, e);
    let ctx = ContextSummary::from_session(o.session(), vec!["arrays".into()], "mid".into());
    let fut = coord.finalize_and_cleanup(&lay, &o, &rep, &ctx, &codes);
    let creport = fut.await.expect("finalize");
    assert!(creport.all_succeeded());
    let lt = std::fs::read_to_string(&local).expect("local");
    let att = creport.attempted;
    let ad = flag.load(Ordering::SeqCst);
    (lay, o, rep, gr, qm, bf.is_none(), att, ad, lt, hist)
}
#[tokio::test]
async fn scripted_session_runs_from_expert_setup_to_final_artifacts() {
    let s = run_scripted("runs").await;
    assert_eq!(s.1.status(), SessionStatus::Completed);
    let via = compute_final(&s.1, &s.4, &s.3).unwrap();
    let direct = final_rating(s.1.session(), &s.4, &s.3).unwrap();
    assert_eq!(via, direct);
    assert!(s.0.report_path().exists() && s.0.solution_path(0).exists());
    HEADS.split('|').for_each(|h| assert!(s.2.contains(h)));
    let code = std::fs::read_to_string(s.0.solution_path(0)).unwrap();
    assert_eq!(code, "def solve():\n    return 42");
}
#[tokio::test]
async fn scripted_session_records_all_expected_core_events() {
    let s = run_scripted("events").await;
    let ev = read_events(s.0.events_path()).expect("parse");
    assert_eq!(ev.as_slice(), s.1.session().events());
    WANTS
        .split('|')
        .for_each(|w| assert!(ev.iter().any(|e| format!("{:?}", e.kind).contains(w))));
    let cc = EventKind::CandidateConnected;
    assert_eq!(ev.iter().filter(|e| e.kind == cc).count(), 1);
}
#[tokio::test]
async fn scripted_session_produces_parseable_section15_artifacts() {
    let s = run_scripted("artifacts").await;
    assert!(s.0.tokens_path().exists() && s.0.context_path().exists());
    let toks = std::fs::read_to_string(s.0.tokens_path()).unwrap();
    assert!(toks.contains("1111"));
    let txt = std::fs::read_to_string(s.0.context_path()).unwrap();
    let ctx: serde_json::Value = serde_json::from_str(&txt).expect("ctx");
    assert_eq!(ctx["session_id"], "s1");
    (0..10).for_each(|i| assert!(s.0.solution_path(i).exists()));
}
#[tokio::test]
async fn scripted_session_keeps_candidate_score_hidden_until_final() {
    let s = run_scripted("hidden").await;
    assert!(s.5);
    let (_, cand) = both_roles_final(&s.1, &s.4, &s.3).unwrap();
    let via = compute_final(&s.1, &s.4, &s.3).unwrap();
    assert_eq!(cand, Some(via));
    assert_eq!(s.9.len(), 4);
    assert_eq!(s.9[1].content, "Consider edge cases and invariants.");
}
#[tokio::test]
async fn scripted_session_contains_no_raw_pastes_or_secrets() {
    let s = run_scripted("secrets").await;
    assert!(!s.0.raw_dir().exists());
    assert_artifacts_clean(s.0.session_dir(), &[API, SECRET, JD]).expect("clean");
    assert!(s.2.contains("Not scored: Q2"));
    [API, SECRET, JD]
        .iter()
        .for_each(|c| assert!(!s.2.contains(c)));
}
#[tokio::test]
async fn scripted_session_cleans_up_only_after_artifact_copy() {
    let s = run_scripted("cleanup").await;
    let order = s.6.join(",");
    assert_eq!(
        order,
        "cancel_inflight,terminate_connection,stop_sync,destroy_sandbox,delete_raw"
    );
    assert!(s.7);
    assert_eq!(s.8, "keep me\n");
}
