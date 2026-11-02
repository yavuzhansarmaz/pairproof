use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::*;
use pairproof::leetcode::Difficulty;
use pairproof::llm::grading::QuestionScoringInput as QSI;
use pairproof::orchestration::health::ProbeFuture;
use pairproof::orchestration::*;
use pairproof::scoring::ApproachQuality::Best;
use pairproof::scoring::*;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
const T0: u64 = 1_730_000_000_000;
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
fn started() -> Orchestrator {
    let easy = qs("e", 5, Difficulty::Easy);
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy, 0).expect("create");
    o.mark_waiting_for_candidate(1).expect("waiting");
    o.attach_candidate(2).expect("attach");
    o.start(3).expect("start");
    o
}
fn waiting() -> Orchestrator {
    let easy = qs("e", 5, Difficulty::Easy);
    let mut o = Orchestrator::create("s1".into(), Language::Python, easy, 0).expect("create");
    o.mark_waiting_for_candidate(1).expect("waiting");
    o
}
#[derive(Debug)]
struct OkVerifier;
impl HostKeyVerifier for OkVerifier {
    fn verify_host_key(&self, _e: &Endpoint, _f: &Fingerprint) -> Result<(), ConnectionGateError> {
        Ok(())
    }
}
struct DownProbe;
impl LeetCodeProbe for DownProbe {
    fn probe<'a>(&'a self) -> ProbeFuture<'a> {
        Box::pin(async move {
            ServiceHealth::Down {
                service: ServiceKind::LeetCode,
                reason: "transport error".into(),
            }
        })
    }
}
impl MetaProbe for DownProbe {
    fn probe<'a>(&'a self) -> ProbeFuture<'a> {
        Box::pin(async move {
            ServiceHealth::Down {
                service: ServiceKind::Meta,
                reason: "status 500".into(),
            }
        })
    }
}
struct AdvSleeper {
    clock: Arc<ManualClock>,
}
impl Sleeper for AdvSleeper {
    fn sleep_ms<'a>(&'a self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        let clock = self.clock.clone();
        Box::pin(async move {
            clock.advance(ms);
        })
    }
}
fn go(i: &QSI) -> GradeOutcome {
    if i.slot_index == 1 {
        return GradeOutcome::NotScored { slot_index: 1 };
    }
    let g = QuestionGrade::new(i.slot_index, 8.0, format!("r{}", i.slot_index), 0.05);
    GradeOutcome::Graded(g.expect("grade"))
}
struct FakeGrader;
impl QuestionGrader for FakeGrader {
    fn grade_all<'a>(&'a self, inputs: &'a [QSI]) -> GradeFuture<'a> {
        Box::pin(async move { inputs.iter().map(go).collect() })
    }
}
struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
fn pending(dropped: Arc<AtomicBool>, delivered: Arc<AtomicBool>) -> impl Future<Output = ()> {
    let guard = DropFlag(dropped);
    async move {
        let _g = guard;
        std::future::pending::<()>().await;
        delivered.store(true, Ordering::SeqCst);
    }
}
type Log = Arc<Mutex<Vec<String>>>;
fn rec(name: &'static str, log: Log) -> CleanupStep {
    Box::new(move || {
        let log = log.clone();
        Box::pin(async move {
            log.lock().unwrap().push(name.to_string());
            Ok(())
        })
    })
}
#[tokio::test]
async fn scripted_service_failure_path_ends_without_evaluation() {
    let clock = Arc::new(ManualClock::new(0));
    let sleeper = AdvSleeper {
        clock: clock.clone(),
    };
    let (lc, meta) = (DownProbe, DownProbe);
    let gate = HealthGate::new(RetryPolicy::new(10_000));
    let fut = gate.await_recovery(
        Probes::new(&lc, &meta),
        InterruptedStep::CodingStep { index: 0 },
        clock.as_ref(),
        &sleeper,
    );
    let out = fut.await;
    assert_eq!(out, RecoveryOutcome::TechnicalFailure);
    assert_eq!(clock.now_ms(), 10_000);
    let mut o = started();
    o.mark_technical_failure(clock.now_ms()).expect("tf");
    assert_eq!(o.status(), SessionStatus::TechnicalFailure);
    assert!(o.is_terminal());
    let (_, cand) = both_roles_final(&o, &BTreeMap::new(), &[]).expect("both");
    assert_eq!(cand, None);
    assert!(!final_visible_to_candidate(o.status()));
}
#[tokio::test]
async fn scripted_disconnect_resume_path_preserves_session_state() {
    let ep = Endpoint::new("203.0.113.10", 22022).expect("ep");
    let un = EphemeralUsername::parse("pp-a1b2c3d4").expect("user");
    let fp = Fingerprint::parse(&"a".repeat(64)).expect("fp");
    let mut gate = ConnectionGate::with_verifier(ep, un, fp, OkVerifier, T0);
    let mut o = waiting();
    let old = gate.current_string(T0).expect("string");
    gate.accept(&mut o, &old, T0 + 1_000).expect("accept");
    o.start(T0 + 2_000).expect("start");
    o.select_approach(ApproachChoice::Preset(0), 100, T0 + 2_500)
        .expect("select");
    o.add_tokens(50).expect("tokens");
    let qm: BTreeMap<usize, Vec<ApproachQuality>> = BTreeMap::from([(0, vec![Best])]);
    let idx_before = o.current_index();
    let rating_before = live_rating(o.session(), &qm);
    gate.handle_disconnect(&mut o, T0 + 3_000)
        .expect("disconnect");
    assert!(!o.is_connected());
    let fresh = gate.reissue_after_disconnect(T0 + 4_000).expect("reissue");
    assert!(gate.accept(&mut o, &old, T0 + 5_000).is_err());
    assert!(!o.is_connected());
    gate.accept(&mut o, &fresh, T0 + 6_000).expect("reconnect");
    assert!(o.is_connected());
    assert_eq!(o.current_index(), idx_before);
    assert_eq!(live_rating(o.session(), &qm), rating_before);
    let ev = o.session().events();
    let cc = EventKind::CandidateConnected;
    assert_eq!(ev.iter().filter(|e| e.kind == cc).count(), 1);
    assert!(ev
        .iter()
        .any(|e| matches!(e.kind, EventKind::CandidateReconnected { .. })));
}
#[tokio::test]
async fn scripted_kill_switch_path_cancels_and_cleans_up() {
    let dropped = Arc::new(AtomicBool::new(false));
    let delivered = Arc::new(AtomicBool::new(false));
    let mut sup = RequestSupervisor::new();
    sup.spawn("coding", pending(dropped.clone(), delivered.clone()));
    assert_eq!(sup.pending_count(), 1);
    let report = sup.kill_switch().await;
    assert_eq!(report.cancelled, vec!["coding".to_string()]);
    assert!(!report.already_done);
    assert!(sup.token().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst));
    assert!(!delivered.load(Ordering::SeqCst));
    assert_eq!(sup.pending_count(), 0);
    let second = sup.kill_switch().await;
    assert!(second.already_done && second.cancelled.is_empty());
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let a = rec("cancel_inflight", log.clone());
    let b = rec("terminate_connection", log.clone());
    let c = rec("stop_sync", log.clone());
    let d = rec("destroy_sandbox", log.clone());
    let e = rec("delete_raw", log.clone());
    let mut seq = CleanupSequence::new(a, b, c, d, e);
    let r = seq.run().await;
    let want = vec![
        "cancel_inflight",
        "terminate_connection",
        "stop_sync",
        "destroy_sandbox",
        "delete_raw",
    ];
    assert_eq!(r.attempted, want);
    assert_eq!(log.lock().unwrap().join(","), want.join(","));
    assert!(r.all_succeeded() && !r.already_done);
    let r2 = seq.run().await;
    assert!(r2.already_done && r2.attempted.is_empty());
    assert_eq!(log.lock().unwrap().len(), 5);
}
#[tokio::test]
async fn scripted_grading_failure_path_marks_question_not_scored() {
    let mut o = started();
    o.finish_question(10).expect("fin0");
    o.finish_question(11).expect("fin1");
    let (stm, codes): (HashMap<String, String>, HashMap<usize, String>) =
        (HashMap::new(), HashMap::new());
    let fut = grade_session(&o, &FakeGrader, &stm, &codes);
    let grades = fut.await.expect("grade");
    assert_eq!(grades.len(), 2);
    assert_eq!(grades[1], GradeOutcome::NotScored { slot_index: 1 });
    let b = compute_final(&o, &BTreeMap::new(), &grades).expect("final");
    let direct = final_rating(o.session(), &BTreeMap::new(), &grades).expect("direct");
    assert_eq!(b, direct);
    assert_eq!(b.per_question[1].grade, None);
    assert_eq!(b.per_question[1].correction, 0.0);
    assert_eq!(b.not_scored, vec![1]);
    let rep = generate_report(o.session(), &b, &grades, &[]);
    assert!(rep.contains("not scored"));
    assert!(rep.contains("Not scored: Q2"));
}
