//! LeetCode and Meta service health, bounded retry and recovery.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::leetcode::{Difficulty, LeetCodeClient, LeetCodeError};
use crate::llm::{LlmError, MetaApiError, MetaClient};
use crate::orchestration::Clock;

/// External service whose health is probed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    LeetCode,
    Meta,
}

/// Health of one service; `reason` never carries a key or body
/// beyond an HTTP status code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceHealth {
    Up,
    Down {
        service: ServiceKind,
        reason: String,
    },
}

impl ServiceHealth {
    /// Returns true only for `Up`.
    pub fn is_up(&self) -> bool {
        matches!(self, Self::Up)
    }
}

/// Coarse failure class used to decide bounded retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HealthFailure {
    /// The service is down; do not retry tightly.
    #[error("service down")]
    Down,
    /// The service rate-limited the request (HTTP 429).
    #[error("service rate limited")]
    RateLimited,
}

/// Typed health error carrying the affected service.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HealthError {
    /// The service is down for `reason` (redacted).
    #[error("service {service:?} down: {reason}")]
    Down {
        service: ServiceKind,
        reason: String,
    },
    /// The service rate-limited the request (HTTP 429).
    #[error("service {service:?} rate limited")]
    RateLimited { service: ServiceKind },
}

impl HealthError {
    /// Returns the coarse failure class for this error.
    pub fn failure(&self) -> HealthFailure {
        match self {
            Self::Down { .. } => HealthFailure::Down,
            Self::RateLimited { .. } => HealthFailure::RateLimited,
        }
    }

    /// Converts this error into a `Down` health value.
    pub fn into_health(self) -> ServiceHealth {
        match self {
            Self::Down { service, reason } => ServiceHealth::Down { service, reason },
            Self::RateLimited { service } => ServiceHealth::Down {
                service,
                reason: "rate limited (status 429)".to_string(),
            },
        }
    }
}

/// Classifies a Meta API error via recoverability for compatibility.
/// `RateLimited` stays distinct; all other failures map to `Down`.
pub fn classify_meta_error(err: &MetaApiError) -> HealthFailure {
    match crate::orchestration::recovery::classify_meta_recoverability(err) {
        crate::orchestration::recovery::Recoverability::RateLimited => HealthFailure::RateLimited,
        crate::orchestration::recovery::Recoverability::Recoverable
        | crate::orchestration::recovery::Recoverability::NonRecoverable => HealthFailure::Down,
    }
}

/// Classifies an LLM error through its wrapped Meta API error.
pub fn classify_llm_error(err: &LlmError) -> HealthFailure {
    match err {
        LlmError::Api(inner) => classify_meta_error(inner),
        _ => HealthFailure::Down,
    }
}

fn meta_reason(err: &MetaApiError) -> String {
    match err {
        MetaApiError::Status { status, .. } => {
            if *status == 429 {
                "rate limited (status 429)".to_string()
            } else {
                format!("status {status}")
            }
        }
        MetaApiError::MissingApiKey => "missing api key".to_string(),
        MetaApiError::Http(_) => "transport error".to_string(),
        MetaApiError::EmptyChoices => "empty response".to_string(),
        MetaApiError::EmptyStream => "empty stream".to_string(),
    }
}

fn leetcode_reason(err: &LeetCodeError) -> String {
    match err {
        LeetCodeError::Http(_) => "transport error".to_string(),
        LeetCodeError::GraphQl(_) => "graphql error".to_string(),
        LeetCodeError::NotFound(_) => "not found".to_string(),
        LeetCodeError::PremiumOnly(_) => "premium only".to_string(),
        LeetCodeError::BadResponse => "bad response".to_string(),
    }
}

/// Boxed future returned by every health probe.
pub type ProbeFuture<'a> = Pin<Box<dyn Future<Output = ServiceHealth> + Send + 'a>>;

/// Narrow boundary for probing LeetCode health.
pub trait LeetCodeProbe: Send + Sync {
    /// Probes LeetCode once and reports `Up` or `Down`.
    fn probe<'a>(&'a self) -> ProbeFuture<'a>;
}

/// Narrow boundary for probing Meta health.
pub trait MetaProbe: Send + Sync {
    /// Probes Meta once and reports `Up` or `Down`.
    fn probe<'a>(&'a self) -> ProbeFuture<'a>;
}

/// Production LeetCode probe backed by the real client.
#[derive(Debug)]
pub struct LeetCodeHealthProbe {
    client: LeetCodeClient,
}

impl LeetCodeHealthProbe {
    /// Creates a probe wrapping `client`.
    pub fn new(client: LeetCodeClient) -> Self {
        Self { client }
    }
}

impl LeetCodeProbe for LeetCodeHealthProbe {
    fn probe<'a>(&'a self) -> ProbeFuture<'a> {
        Box::pin(async move {
            match self
                .client
                .list_questions(Difficulty::Easy, &[], 1, 0)
                .await
            {
                Ok(_) => ServiceHealth::Up,
                Err(err) => ServiceHealth::Down {
                    service: ServiceKind::LeetCode,
                    reason: leetcode_reason(&err),
                },
            }
        })
    }
}

/// Production Meta probe backed by the real client.
#[derive(Debug)]
pub struct MetaHealthProbe {
    client: MetaClient,
}

impl MetaHealthProbe {
    /// Creates a probe wrapping `client`.
    pub fn new(client: MetaClient) -> Self {
        Self { client }
    }
}

impl MetaProbe for MetaHealthProbe {
    fn probe<'a>(&'a self) -> ProbeFuture<'a> {
        Box::pin(async move {
            match self.client.list_models().await {
                Ok(_) => ServiceHealth::Up,
                Err(err) => ServiceHealth::Down {
                    service: ServiceKind::Meta,
                    reason: meta_reason(&err),
                },
            }
        })
    }
}

/// Injected async sleeper; every retry path goes through it.
pub trait Sleeper: Send + Sync {
    /// Sleeps for `ms` milliseconds without blocking a thread.
    fn sleep_ms<'a>(&'a self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Production sleeper backed by `tokio::time::sleep`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokioSleeper;

impl Sleeper for TokioSleeper {
    fn sleep_ms<'a>(&'a self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        })
    }
}

/// Bounded retry window for service recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub window_ms: u64,
}

impl RetryPolicy {
    /// Creates a policy with the given recovery window.
    pub fn new(window_ms: u64) -> Self {
        Self { window_ms }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { window_ms: 60_000 }
    }
}

/// Virtual step between re-probes inside the recovery window.
pub const RECOVERY_STEP_MS: u64 = 5_000;
/// Maximum attempts for one rate-limited operation.
pub const MAX_RATE_LIMIT_ATTEMPTS: u32 = 3;
/// Virtual delay between rate-limited attempts.
pub const RATE_LIMIT_DELAY_MS: u64 = 250;

/// The exact step interrupted by a service failure. This type
/// carries data only; resuming it repeats no side effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterruptedStep {
    FetchStatement { slug: String },
    GenerateApproaches { index: usize },
    CodingStep { index: usize },
    ChatTurn { index: usize },
    Grading { index: usize },
}

/// Outcome of waiting for service recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Resumed(InterruptedStep),
    TechnicalFailure,
}

/// Retries `operation` on `RateLimited` only, at most
/// [`MAX_RATE_LIMIT_ATTEMPTS`] attempts, sleeping through the
/// injected sleeper between attempts. Exhaustion yields `Down`.
pub async fn retry_bounded<T, S, F, Fut>(sleeper: &S, mut operation: F) -> Result<T, HealthFailure>
where
    S: Sleeper,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, MetaApiError>>,
{
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err) => match classify_meta_error(&err) {
                HealthFailure::RateLimited if attempt < MAX_RATE_LIMIT_ATTEMPTS => {
                    sleeper.sleep_ms(RATE_LIMIT_DELAY_MS).await;
                }
                HealthFailure::RateLimited => return Err(HealthFailure::Down),
                HealthFailure::Down => return Err(HealthFailure::Down),
            },
        }
    }
}

/// Both service probes, borrowed for one recovery wait.
pub struct Probes<'a> {
    pub leetcode: &'a dyn LeetCodeProbe,
    pub meta: &'a dyn MetaProbe,
}

impl<'a> Probes<'a> {
    /// Bundles the LeetCode and Meta probes.
    pub fn new(leetcode: &'a dyn LeetCodeProbe, meta: &'a dyn MetaProbe) -> Self {
        Self { leetcode, meta }
    }
}

/// Alias for [`Probes`].
pub type ServiceProbes<'a> = Probes<'a>;

/// Recovery gate driven by an injected clock and sleeper.
///
/// Implemented gate logic, NOT wired into TTY loop; wiring pending
/// product/design decision on interception point + countdown
/// propagation + non-blocking tick. See README Compatibility notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthGate {
    policy: RetryPolicy,
}

impl HealthGate {
    /// Creates a gate with the given retry policy.
    pub fn new(policy: RetryPolicy) -> Self {
        Self { policy }
    }

    /// Returns the gate retry policy.
    pub fn policy(&self) -> RetryPolicy {
        self.policy
    }

    /// Probes both services once, in LeetCode then Meta order.
    pub async fn probe_both(&self, probes: &Probes<'_>) -> Vec<ServiceHealth> {
        vec![probes.leetcode.probe().await, probes.meta.probe().await]
    }

    /// Waits for recovery: returns the exact `interrupted` step as
    /// soon as both services are up, re-probing in bounded sleeper
    /// steps until the policy window (by `clock`) elapses.
    ///
    /// Implemented gate logic, NOT wired into TTY loop; wiring
    /// pending product/design decision on interception point +
    /// countdown propagation + non-blocking tick.
    pub async fn await_recovery(
        &self,
        probes: Probes<'_>,
        interrupted: InterruptedStep,
        clock: &dyn Clock,
        sleeper: &dyn Sleeper,
    ) -> RecoveryOutcome {
        let first = self.probe_both(&probes).await;
        if first.iter().all(ServiceHealth::is_up) {
            return RecoveryOutcome::Resumed(interrupted);
        }
        let start_ms = clock.now_ms();
        loop {
            let elapsed = clock.now_ms().saturating_sub(start_ms);
            if elapsed >= self.policy.window_ms {
                return RecoveryOutcome::TechnicalFailure;
            }
            let step = RECOVERY_STEP_MS.min(self.policy.window_ms - elapsed);
            sleeper.sleep_ms(step).await;
            let health = self.probe_both(&probes).await;
            if health.iter().all(ServiceHealth::is_up) {
                return RecoveryOutcome::Resumed(interrupted);
            }
        }
    }
}

impl Default for HealthGate {
    fn default() -> Self {
        Self::new(RetryPolicy::default())
    }
}

/// Waits for recovery with the default 60-second policy.
///
/// Implemented gate logic, NOT wired into TTY loop; wiring pending
/// product/design decision on interception point + countdown
/// propagation + non-blocking tick.
pub async fn await_recovery(
    probes: Probes<'_>,
    interrupted: InterruptedStep,
    clock: &dyn Clock,
    sleeper: &dyn Sleeper,
) -> RecoveryOutcome {
    HealthGate::default()
        .await_recovery(probes, interrupted, clock, sleeper)
        .await
}
