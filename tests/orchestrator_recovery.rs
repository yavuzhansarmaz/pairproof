use std::time::{Duration, Instant};

use pairproof::leetcode::LeetCodeError;
use pairproof::llm::{LlmError, MetaApiError};
use pairproof::orchestration::health::{InterruptedStep, RecoveryOutcome, ServiceKind};
use pairproof::orchestration::recovery::{
    classify_leetcode_recoverability, classify_llm_recoverability, classify_meta_recoverability,
    classify_pipeline_recoverability, classify_status, is_recoverable, PauseClock, Recoverability,
    RecoveryController, RecoveryError, RecoveryPhase,
};
use pairproof::orchestration::{OrchestratorError, PipelineError};

fn status_err(status: u16) -> MetaApiError {
    MetaApiError::Status {
        status,
        body: "canary-secret-body".to_string(),
    }
}

fn step() -> InterruptedStep {
    InterruptedStep::CodingStep { index: 2 }
}

#[test]
fn transient_network_failure_is_recoverable() {
    let fetch = PipelineError::Fetch(
        "meta api request failed: transport error: connection reset by peer".to_string(),
    );
    assert_eq!(
        classify_pipeline_recoverability(&fetch),
        Recoverability::Recoverable
    );
    let llm = PipelineError::Llm("request failed: operation timeout".to_string());
    assert_eq!(
        classify_pipeline_recoverability(&llm),
        Recoverability::Recoverable
    );
    assert!(is_recoverable(Recoverability::Recoverable));
}

#[test]
fn upstream_unavailable_is_recoverable() {
    for status in [500, 502, 503, 504, 408] {
        assert_eq!(classify_status(status), Recoverability::Recoverable);
        assert_eq!(
            classify_meta_recoverability(&status_err(status)),
            Recoverability::Recoverable
        );
    }
}

#[test]
fn authentication_failure_is_not_recoverable() {
    for status in [401, 403] {
        assert_eq!(
            classify_meta_recoverability(&status_err(status)),
            Recoverability::NonRecoverable
        );
    }
    assert_eq!(
        classify_meta_recoverability(&MetaApiError::MissingApiKey),
        Recoverability::NonRecoverable
    );
    assert!(!is_recoverable(Recoverability::NonRecoverable));
}

#[test]
fn invalid_request_is_not_recoverable() {
    for status in [400, 404, 422] {
        assert_eq!(classify_status(status), Recoverability::NonRecoverable);
        assert_eq!(
            classify_meta_recoverability(&status_err(status)),
            Recoverability::NonRecoverable
        );
    }
}

#[test]
fn candidate_code_failure_is_not_recoverable() {
    let sandbox = PipelineError::Sandbox("candidate code timeout after 5s".to_string());
    assert_eq!(
        classify_pipeline_recoverability(&sandbox),
        Recoverability::NonRecoverable
    );
    assert_eq!(
        classify_pipeline_recoverability(&PipelineError::NoApproaches),
        Recoverability::NonRecoverable
    );
}

#[test]
fn internal_error_is_not_recoverable() {
    let core = PipelineError::Core(OrchestratorError::AlreadyAttached);
    assert_eq!(
        classify_pipeline_recoverability(&core),
        Recoverability::NonRecoverable
    );
    assert_eq!(
        classify_llm_recoverability(&LlmError::Parse("bad json".to_string())),
        Recoverability::NonRecoverable
    );
    assert_eq!(classify_status(501), Recoverability::NonRecoverable);
    assert_eq!(
        classify_meta_recoverability(&MetaApiError::EmptyChoices),
        Recoverability::NonRecoverable
    );
    assert_eq!(
        classify_leetcode_recoverability(&LeetCodeError::BadResponse),
        Recoverability::NonRecoverable
    );
}

#[test]
fn recovery_uses_monotonic_deadline() {
    let t0 = Instant::now();
    let mut controller = RecoveryController::new(60_000);
    let generation = controller
        .begin(ServiceKind::Meta, step(), t0)
        .expect("begin");
    assert_eq!(generation, 1);
    assert_eq!(controller.generation(), 1);
    assert_eq!(controller.phase(), RecoveryPhase::Recovering);
    assert_eq!(controller.remaining_ms(t0), 60_000);
    assert_eq!(
        controller.remaining_ms(t0 + Duration::from_millis(15_000)),
        45_000
    );
    assert_eq!(
        controller.remaining_ms(t0 + Duration::from_millis(75_000)),
        0
    );
    assert_eq!(controller.state().deadline_ms, 60_000);
    assert_eq!(controller.state().service, Some(ServiceKind::Meta));
    assert!(controller.accepts(generation));
    assert!(!controller.accepts(generation + 1));
}

#[test]
fn recovery_expires_at_deadline() {
    let t0 = Instant::now();
    let mut controller = RecoveryController::new(10_000);
    let generation = controller
        .begin(ServiceKind::LeetCode, step(), t0)
        .expect("begin");
    assert!(!controller.is_expired(t0 + Duration::from_millis(9_999)));
    assert!(controller
        .expire(generation, t0 + Duration::from_millis(9_999))
        .is_err());
    assert_eq!(controller.phase(), RecoveryPhase::Recovering);
    assert!(controller.is_expired(t0 + Duration::from_millis(10_000)));
    assert_eq!(
        controller
            .expire(generation, t0 + Duration::from_millis(10_000))
            .expect("expire"),
        RecoveryOutcome::TechnicalFailure
    );
    assert_eq!(controller.phase(), RecoveryPhase::Failed);
    assert_eq!(
        controller.remaining_ms(t0 + Duration::from_millis(10_000)),
        0
    );
}

#[test]
fn recovery_cancellation_stops_attempts() {
    let t0 = Instant::now();
    let mut controller = RecoveryController::new(60_000);
    let generation = controller
        .begin(ServiceKind::Meta, step(), t0)
        .expect("begin");
    assert_eq!(controller.record_attempt(generation).expect("attempt"), 1);
    controller.cancel(generation).expect("cancel");
    assert_eq!(controller.phase(), RecoveryPhase::Cancelled);
    assert!(!controller.accepts(generation));
    assert!(controller.record_attempt(generation).is_err());
    assert!(controller.succeed(generation, t0).is_err());
    assert_eq!(controller.remaining_ms(t0), 0);
}

#[test]
fn recovery_rejects_stale_success() {
    let t0 = Instant::now();
    let mut controller = RecoveryController::new(60_000);
    let first = controller
        .begin(ServiceKind::Meta, step(), t0)
        .expect("first");
    controller.cancel(first).expect("cancel");
    let second = controller
        .begin(ServiceKind::Meta, step(), t0)
        .expect("second");
    assert_eq!(second, first + 1);
    assert_eq!(
        controller.succeed(first, t0),
        Err(RecoveryError::StaleGeneration {
            expected: second,
            got: first,
        })
    );
    assert_eq!(
        controller.succeed(second, t0).expect("succeed"),
        RecoveryOutcome::Resumed(step())
    );
    assert_eq!(controller.phase(), RecoveryPhase::Recovered);
    controller.reset().expect("reset");
    assert_eq!(controller.phase(), RecoveryPhase::Healthy);
}

#[test]
fn recovery_rejects_invalid_state_transition() {
    let t0 = Instant::now();
    let mut controller = RecoveryController::new(60_000);
    assert_eq!(
        controller.succeed(0, t0),
        Err(RecoveryError::InvalidTransition)
    );
    assert!(controller.reset().is_err());
    assert!(controller.cancel(0).is_err());
    let generation = controller
        .begin(ServiceKind::Meta, step(), t0)
        .expect("begin");
    assert_eq!(
        controller.begin(ServiceKind::LeetCode, step(), t0),
        Err(RecoveryError::AlreadyRecovering)
    );
    assert!(controller.reset().is_err());
    assert_eq!(controller.generation(), generation);
}

#[test]
fn recovery_attempts_are_bounded() {
    let t0 = Instant::now();
    let mut controller = RecoveryController::new(10_000);
    assert_eq!(controller.max_attempts(), 2);
    let generation = controller
        .begin(ServiceKind::Meta, step(), t0)
        .expect("begin");
    assert_eq!(controller.record_attempt(generation).expect("a1"), 1);
    assert_eq!(controller.record_attempt(generation).expect("a2"), 2);
    assert!(controller.record_attempt(generation).is_err());
    assert_eq!(controller.phase(), RecoveryPhase::Failed);
    let mut standard = RecoveryController::new(60_000);
    assert_eq!(standard.max_attempts(), 12);
    let gen = standard
        .begin(ServiceKind::Meta, step(), t0)
        .expect("begin");
    for expected in 1..=12 {
        assert_eq!(
            standard.record_attempt(gen).expect("bounded attempt"),
            expected
        );
    }
    assert!(standard.record_attempt(gen).is_err());
}

#[test]
fn rate_limited_is_distinguished_from_outage() {
    assert_eq!(classify_status(429), Recoverability::RateLimited);
    assert_eq!(
        classify_meta_recoverability(&status_err(429)),
        Recoverability::RateLimited
    );
    assert!(is_recoverable(Recoverability::RateLimited));
}

#[test]
fn pipeline_embedded_status_is_parsed_without_body() {
    let err = PipelineError::Llm("meta api returned status 503: secret-body".to_string());
    assert_eq!(
        classify_pipeline_recoverability(&err),
        Recoverability::Recoverable
    );
    let auth = PipelineError::Fetch("meta api returned status 401: secret-body".to_string());
    assert_eq!(
        classify_pipeline_recoverability(&auth),
        Recoverability::NonRecoverable
    );
    let unknown = PipelineError::Llm("model produced invalid output".to_string());
    assert_eq!(
        classify_pipeline_recoverability(&unknown),
        Recoverability::NonRecoverable
    );
}

#[test]
fn leetcode_non_transport_failures_are_not_recoverable() {
    for err in [
        LeetCodeError::GraphQl("bad query".to_string()),
        LeetCodeError::NotFound("two-sum".to_string()),
        LeetCodeError::PremiumOnly("premium".to_string()),
    ] {
        assert_eq!(
            classify_leetcode_recoverability(&err),
            Recoverability::NonRecoverable
        );
    }
    assert_eq!(
        classify_llm_recoverability(&LlmError::Api(status_err(503))),
        Recoverability::Recoverable
    );
}

#[test]
fn pause_clock_does_not_pause_twice() {
    let mut clock = PauseClock::new();
    assert!(clock.pause(1_000));
    assert!(!clock.pause(1_100));
    assert!(clock.is_paused());
    assert_eq!(clock.adjusted_now(3_000), 1_000);
    assert!(clock.resume(3_000));
    assert!(!clock.resume(3_100));
    assert!(!clock.is_paused());
    assert_eq!(clock.paused_total_ms(), 2_000);
    assert_eq!(clock.adjusted_now(5_000), 3_000);
}
