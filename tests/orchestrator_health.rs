use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pairproof::connection::{Endpoint, EphemeralUsername, Fingerprint};
use pairproof::core::{EventKind, Language, QuestionStatus, SessionStatus};
use pairproof::llm::MetaApiError;
use pairproof::orchestration::connection::{ConnectionGate, HostKeyVerifier};
use pairproof::orchestration::health::{
    classify_meta_error, retry_bounded, HealthFailure, HealthGate, InterruptedStep, LeetCodeProbe,
    MetaProbe, ProbeFuture, Probes, RecoveryOutcome, RetryPolicy, ServiceHealth, ServiceKind,
    Sleeper,
};
use pairproof::orchestration::{Clock, ManualClock, Orchestrator, OrchestratorError};
use pairproof::scoring::{live_rating, ApproachQuality};

mod common_mod;
use common_mod::easy_five as easy;

fn next_fail(failures: &AtomicUsize) -> bool {
    let value = failures.load(Ordering::SeqCst);
    value > 0 && {
        failures.store(value - 1, Ordering::SeqCst);
        true
    }
}

#[derive(Clone)]
struct FakeProbe {
    service: ServiceKind,
    failures: Arc<AtomicUsize>,
}
impl FakeProbe {
    fn new(service: ServiceKind, n: usize) -> Self {
        Self {
            service,
            failures: Arc::new(AtomicUsize::new(n)),
        }
    }
    fn health(&self) -> ServiceHealth {
        if next_fail(&self.failures) {
            let reason = match self.service {
                ServiceKind::LeetCode => "transport error",
                ServiceKind::Meta => "status 500",
            };
            ServiceHealth::Down {
                service: self.service,
                reason: reason.to_string(),
            }
        } else {
            ServiceHealth::Up
        }
    }
}
fn lp(n: usize) -> FakeProbe {
    FakeProbe::new(ServiceKind::LeetCode, n)
}
fn mp(n: usize) -> FakeProbe {
    FakeProbe::new(ServiceKind::Meta, n)
}
impl LeetCodeProbe for FakeProbe {
    fn probe<'a>(&'a self) -> ProbeFuture<'a> {
        Box::pin(async move { self.health() })
    }
}
impl MetaProbe for FakeProbe {
    fn probe<'a>(&'a self) -> ProbeFuture<'a> {
        Box::pin(async move { self.health() })
    }
}

#[derive(Clone)]
struct RecSleeper {
    calls: Arc<Mutex<Vec<u64>>>,
    clock: Arc<ManualClock>,
}
impl RecSleeper {
    fn new(clock: Arc<ManualClock>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            clock,
        }
    }
    fn total(&self) -> u64 {
        self.calls.lock().expect("lock").iter().sum()
    }
    fn count(&self) -> usize {
        self.calls.lock().expect("lock").len()
    }
}
impl Sleeper for RecSleeper {
    fn sleep_ms<'a>(&'a self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().expect("lock").push(ms);
            self.clock.advance(ms);
        })
    }
}

fn started() -> Orchestrator {
    let mut orch = Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create");
    orch.mark_waiting_for_candidate(1).expect("waiting");
    orch.attach_candidate(2).expect("attach");
    orch.start(3).expect("start");
    orch
}

fn gate(window_ms: u64) -> HealthGate {
    HealthGate::new(RetryPolicy::new(window_ms))
}

fn waiting() -> Orchestrator {
    let mut orch = Orchestrator::create("s1".into(), Language::Python, easy(), 0).expect("create");
    orch.mark_waiting_for_candidate(1).expect("waiting");
    orch
}

async fn recover(
    lc: &FakeProbe,
    meta: &FakeProbe,
    step: InterruptedStep,
    clock: &ManualClock,
    sleeper: &RecSleeper,
) -> RecoveryOutcome {
    gate(60_000)
        .await_recovery(Probes::new(lc, meta), step, clock, sleeper)
        .await
}

#[tokio::test]
async fn service_health_identifies_leetcode_failure() {
    let (lc, meta) = (lp(1_000_000), mp(0));
    let health = HealthGate::default()
        .probe_both(&Probes::new(&lc, &meta))
        .await;
    assert_eq!(
        health[0],
        ServiceHealth::Down {
            service: ServiceKind::LeetCode,
            reason: "transport error".to_string()
        }
    );
    assert_eq!(health[1], ServiceHealth::Up);
}

#[tokio::test]
async fn service_health_identifies_meta_failure() {
    let (lc, meta) = (lp(0), mp(1_000_000));
    let health = HealthGate::default()
        .probe_both(&Probes::new(&lc, &meta))
        .await;
    assert_eq!(health[0], ServiceHealth::Up);
    assert_eq!(
        health[1],
        ServiceHealth::Down {
            service: ServiceKind::Meta,
            reason: "status 500".to_string()
        }
    );
    let err = MetaApiError::Status {
        status: 500,
        body: "secret-body".to_string(),
    };
    assert_eq!(classify_meta_error(&err), HealthFailure::Down);
}

#[tokio::test]
async fn service_retry_resumes_exact_interrupted_step() {
    let clock = Arc::new(ManualClock::new(0));
    let sleeper = RecSleeper::new(clock.clone());
    let (lc, meta) = (lp(1), mp(0));
    let step = InterruptedStep::CodingStep { index: 2 };
    let outcome = recover(&lc, &meta, step.clone(), clock.as_ref(), &sleeper).await;
    assert_eq!(outcome, RecoveryOutcome::Resumed(step));
    assert_eq!(sleeper.total(), 5_000);
}

#[tokio::test]
async fn service_retry_does_not_duplicate_question_or_action() {
    let orch = started();
    let (events, index, slots) = (
        orch.session().events().len(),
        orch.current_index(),
        orch.session().slots().len(),
    );
    let clock = Arc::new(ManualClock::new(0));
    let sleeper = RecSleeper::new(clock.clone());
    let (lc, meta) = (lp(1), mp(0));
    let step = InterruptedStep::FetchStatement {
        slug: "e0".to_string(),
    };
    let outcome = recover(&lc, &meta, step.clone(), clock.as_ref(), &sleeper).await;
    assert_eq!(outcome, RecoveryOutcome::Resumed(step));
    assert_eq!(orch.session().events().len(), events);
    assert_eq!(orch.current_index(), index);
    assert_eq!(orch.session().slots().len(), slots);
    assert_eq!(
        orch.current_question().map(|item| item.title_slug.as_str()),
        Some("e0")
    );
}

#[tokio::test]
async fn service_failure_after_retry_window_ends_as_technical_failure() {
    let clock = Arc::new(ManualClock::new(0));
    let sleeper = RecSleeper::new(clock.clone());
    let (lc, meta) = (lp(1_000_000), mp(0));
    let outcome = recover(
        &lc,
        &meta,
        InterruptedStep::Grading { index: 0 },
        clock.as_ref(),
        &sleeper,
    )
    .await;
    assert_eq!(outcome, RecoveryOutcome::TechnicalFailure);
    let mut orch = started();
    orch.mark_technical_failure(clock.now_ms())
        .expect("failure");
    assert_eq!(orch.status(), SessionStatus::TechnicalFailure);
    assert!(orch.is_terminal());
}

#[tokio::test]
async fn technical_failure_produces_no_candidate_evaluation() {
    let mut orch = started();
    orch.mark_technical_failure(9).expect("failure");
    assert_eq!(orch.status(), SessionStatus::TechnicalFailure);
    assert!(orch.is_terminal());
    assert!(orch
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::TechnicalFailure));
    assert!(!orch
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::Completed));
    assert!(orch.mark_completed(10).is_err());
    assert!(orch.finish_question(10).is_err());
    assert!(orch.add_tokens(1).is_err());
}

#[tokio::test]
async fn rate_limit_is_not_treated_as_success() {
    let err = MetaApiError::Status {
        status: 429,
        body: "canary-secret-body".to_string(),
    };
    assert_eq!(classify_meta_error(&err), HealthFailure::RateLimited);
    let clock = Arc::new(ManualClock::new(0));
    let sleeper = RecSleeper::new(clock);
    let result: Result<(), HealthFailure> = retry_bounded(&sleeper, || async {
        Err::<(), _>(MetaApiError::Status {
            status: 429,
            body: "canary-secret-body".to_string(),
        })
    })
    .await;
    assert_eq!(result, Err(HealthFailure::Down));
    let meta = mp(1_000_000);
    assert!(!MetaProbe::probe(&meta).await.is_up());
}

#[tokio::test]
async fn rate_limit_retry_is_bounded() {
    let clock = Arc::new(ManualClock::new(0));
    let sleeper = RecSleeper::new(clock);
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let result: Result<(), HealthFailure> = retry_bounded(&sleeper, move || {
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            Err::<(), _>(MetaApiError::Status {
                status: 429,
                body: "limited".to_string(),
            })
        }
    })
    .await;
    assert_eq!(result, Err(HealthFailure::Down));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(sleeper.count(), 2);
}

#[tokio::test]
async fn disconnect_is_logged_and_score_neutral() {
    let mut orch = started();
    let qualities: BTreeMap<usize, Vec<ApproachQuality>> = BTreeMap::new();
    let before = live_rating(orch.session(), &qualities).rating;
    orch.disconnect_candidate(10).expect("disconnect");
    assert!(orch
        .session()
        .events()
        .iter()
        .any(|e| e.kind == EventKind::CandidateDisconnected));
    assert_eq!(live_rating(orch.session(), &qualities).rating, before);
    orch.reconnect_candidate(60).expect("reconnect");
    assert!(orch
        .session()
        .events()
        .iter()
        .any(|e| matches!(e.kind, EventKind::CandidateReconnected { downtime_ms: 50 })));
    assert_eq!(live_rating(orch.session(), &qualities).rating, before);
}

#[tokio::test]
async fn reconnect_resumes_same_question_and_stage() {
    let mut orch = started();
    let started_at = orch.session().slots()[0].started_at_ms;
    orch.disconnect_candidate(10).expect("disconnect");
    orch.reconnect_candidate(25).expect("reconnect");
    assert!(orch.is_connected());
    assert_eq!(orch.status(), SessionStatus::InProgress);
    assert_eq!(orch.current_index(), Some(0));
    assert_eq!(orch.session().slots()[0].status, QuestionStatus::Active);
    assert_eq!(orch.session().slots()[0].started_at_ms, started_at);
}

#[derive(Debug, Clone, Copy)]
struct OkVerifier;
impl HostKeyVerifier for OkVerifier {
    fn verify_host_key(
        &self,
        _endpoint: &Endpoint,
        _fingerprint: &Fingerprint,
    ) -> Result<(), pairproof::orchestration::connection::ConnectionGateError> {
        Ok(())
    }
}

const T0: u64 = 1_730_000_000_000;

#[tokio::test]
async fn reconnect_rejects_stale_or_second_candidate() {
    let endpoint = Endpoint::new("203.0.113.10", 22022).expect("endpoint");
    let username = EphemeralUsername::parse("pp-a1b2c3d4").expect("username");
    let fingerprint = Fingerprint::parse(&"a".repeat(64)).expect("fingerprint");
    let mut gate = ConnectionGate::with_verifier(endpoint, username, fingerprint, OkVerifier, T0);
    let mut orch = waiting();
    let old = gate.current_string(T0).expect("string");
    gate.accept(&mut orch, &old, T0 + 1_000).expect("accept");
    orch.start(T0 + 2_000).expect("start");
    gate.handle_disconnect(&mut orch, T0 + 3_000)
        .expect("disconnect");
    let fresh = gate.reissue_after_disconnect(T0 + 4_000).expect("reissue");
    assert!(gate.accept(&mut orch, &old, T0 + 5_000).is_err());
    assert!(!orch.is_connected());
    gate.accept(&mut orch, &fresh, T0 + 6_000)
        .expect("reconnect");
    assert!(orch.is_connected());
    let events = orch.session().events().len();
    assert_eq!(
        orch.attach_candidate(T0 + 7_000).expect_err("second"),
        OrchestratorError::AlreadyAttached
    );
    assert_eq!(orch.session().events().len(), events);
}

#[tokio::test]
async fn retry_countdown_uses_injected_clock() {
    let clock = Arc::new(ManualClock::new(1_000));
    let sleeper = RecSleeper::new(clock.clone());
    let (lc, meta) = (lp(1_000_000), mp(1_000_000));
    let outcome = recover(
        &lc,
        &meta,
        InterruptedStep::ChatTurn { index: 1 },
        clock.as_ref(),
        &sleeper,
    )
    .await;
    assert_eq!(outcome, RecoveryOutcome::TechnicalFailure);
    assert_eq!(clock.now_ms(), 61_000);
    assert_eq!(sleeper.total(), 60_000);
    assert_eq!(sleeper.count(), 12);
}
