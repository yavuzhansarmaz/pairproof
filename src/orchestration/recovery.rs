//! Recoverability classification and non-blocking recovery state.

use std::time::{Duration, Instant};

use crate::leetcode::LeetCodeError;
use crate::llm::{LlmError, MetaApiError};
use crate::orchestration::health::{
    InterruptedStep, RecoveryOutcome, ServiceKind, RECOVERY_STEP_MS,
};
use crate::orchestration::pipeline::PipelineError;

/// Whether a failure may succeed after waiting and retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recoverability {
    Recoverable,
    RateLimited,
    NonRecoverable,
}

/// Returns true for failures worth waiting on and retrying.
pub fn is_recoverable(classification: Recoverability) -> bool {
    matches!(
        classification,
        Recoverability::Recoverable | Recoverability::RateLimited
    )
}

/// Classifies an HTTP status; 501 and unlisted 5xx are permanent.
pub fn classify_status(status: u16) -> Recoverability {
    match status {
        429 => Recoverability::RateLimited,
        408 | 500 | 502 | 503 | 504 => Recoverability::Recoverable,
        _ => Recoverability::NonRecoverable,
    }
}

fn classify_reqwest_error(err: &reqwest::Error) -> Recoverability {
    if err.is_timeout() || err.is_connect() {
        return Recoverability::Recoverable;
    }
    if let Some(status) = err.status() {
        return classify_status(status.as_u16());
    }
    Recoverability::NonRecoverable
}

/// Classifies a Meta API error by status or transport kind only.
pub fn classify_meta_recoverability(err: &MetaApiError) -> Recoverability {
    match err {
        MetaApiError::MissingApiKey => Recoverability::NonRecoverable,
        MetaApiError::Http(inner) => classify_reqwest_error(inner),
        MetaApiError::Status { status, .. } => classify_status(*status),
        MetaApiError::EmptyChoices | MetaApiError::EmptyStream => Recoverability::NonRecoverable,
    }
}

/// Classifies an LLM error through its wrapped Meta error, if any.
pub fn classify_llm_recoverability(err: &LlmError) -> Recoverability {
    match err {
        LlmError::Api(inner) => classify_meta_recoverability(inner),
        LlmError::Parse(_) | LlmError::Scoring(_) | LlmError::Core(_) => {
            Recoverability::NonRecoverable
        }
    }
}

/// Classifies a LeetCode error by transport kind only.
pub fn classify_leetcode_recoverability(err: &LeetCodeError) -> Recoverability {
    match err {
        LeetCodeError::Http(inner) => classify_reqwest_error(inner),
        LeetCodeError::GraphQl(_)
        | LeetCodeError::NotFound(_)
        | LeetCodeError::PremiumOnly(_)
        | LeetCodeError::BadResponse => Recoverability::NonRecoverable,
    }
}

fn parse_embedded_status(text: &str) -> Option<u16> {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("status ").find_map(|(idx, _)| {
        let digits: String = lower[idx + 7..].chars().take(3).collect();
        digits
            .parse::<u16>()
            .ok()
            .filter(|c| (100..=599).contains(c))
    })
}

/// Classifies redacted pipeline text from an LLM or fetch failure.
fn classify_pipeline_text(text: &str) -> Recoverability {
    if let Some(status) = parse_embedded_status(text) {
        return classify_status(status);
    }
    let lower = text.to_ascii_lowercase();
    let needles = [
        "timeout",
        "timed out",
        "connection",
        "dns",
        "transport error",
        "request failed",
    ];
    if needles.iter().any(|needle| lower.contains(needle)) {
        Recoverability::Recoverable
    } else {
        Recoverability::NonRecoverable
    }
}

/// Classifies a pipeline error; sandbox and core errors never recover.
pub fn classify_pipeline_recoverability(err: &PipelineError) -> Recoverability {
    match err {
        PipelineError::Llm(text) | PipelineError::Fetch(text) => classify_pipeline_text(text),
        PipelineError::Core(_)
        | PipelineError::Sandbox(_)
        | PipelineError::NoApproaches
        | PipelineError::InvalidPreset
        | PipelineError::ChatBusy
        | PipelineError::ChatUnavailable => Recoverability::NonRecoverable,
    }
}

/// Phase of one recovery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPhase {
    Healthy,
    Recovering,
    Recovered,
    Failed,
    Cancelled,
}

/// Snapshot of recovery state; no error bodies are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryState {
    pub phase: RecoveryPhase,
    pub generation: u64,
    pub service: Option<ServiceKind>,
    pub interrupted: Option<InterruptedStep>,
    pub started: Option<Instant>,
    pub deadline_ms: u64,
}

/// Typed recovery state-machine error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryError {
    #[error("recovery already in progress")]
    AlreadyRecovering,
    #[error("stale recovery generation: expected {expected}, got {got}")]
    StaleGeneration { expected: u64, got: u64 },
    #[error("invalid recovery state transition")]
    InvalidTransition,
}

/// Non-blocking recovery controller driven by caller-supplied instants.
#[derive(Debug)]
pub struct RecoveryController {
    state: RecoveryState,
    attempts: u32,
}

impl RecoveryController {
    /// Creates a healthy controller with a `window_ms` recovery window.
    pub fn new(window_ms: u64) -> Self {
        Self {
            state: RecoveryState {
                phase: RecoveryPhase::Healthy,
                generation: 0,
                service: None,
                interrupted: None,
                started: None,
                deadline_ms: window_ms,
            },
            attempts: 0,
        }
    }

    /// Returns the current phase.
    pub fn phase(&self) -> RecoveryPhase {
        self.state.phase
    }
    /// Returns the current generation.
    pub fn generation(&self) -> u64 {
        self.state.generation
    }
    /// Returns a snapshot of the current state.
    pub fn state(&self) -> RecoveryState {
        self.state.clone()
    }
    /// Returns attempts recorded for the current generation.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    /// Returns the attempt bound `window_ms / RECOVERY_STEP_MS`.
    pub fn max_attempts(&self) -> u32 {
        u32::try_from(self.state.deadline_ms / RECOVERY_STEP_MS).unwrap_or(u32::MAX)
    }

    /// Returns true only for the live recovering generation.
    pub fn accepts(&self, generation: u64) -> bool {
        self.state.phase == RecoveryPhase::Recovering && generation == self.state.generation
    }

    /// Starts recovery; a recovering attempt is never superseded.
    pub fn begin(
        &mut self,
        service: ServiceKind,
        step: InterruptedStep,
        now: Instant,
    ) -> Result<u64, RecoveryError> {
        if self.state.phase == RecoveryPhase::Recovering {
            return Err(RecoveryError::AlreadyRecovering);
        }
        self.state.generation = self.state.generation.saturating_add(1);
        self.state.phase = RecoveryPhase::Recovering;
        self.state.service = Some(service);
        self.state.interrupted = Some(step);
        self.state.started = Some(now);
        self.attempts = 0;
        Ok(self.state.generation)
    }

    /// Returns milliseconds left, saturating to zero past the deadline.
    pub fn remaining_ms(&self, now: Instant) -> u64 {
        if self.state.phase != RecoveryPhase::Recovering {
            return 0;
        }
        let Some(started) = self.state.started else {
            return 0;
        };
        let elapsed = now.saturating_duration_since(started).as_millis();
        let elapsed_ms = u64::try_from(elapsed).unwrap_or(u64::MAX);
        self.state.deadline_ms.saturating_sub(elapsed_ms)
    }

    /// Returns true when the monotonic deadline has been reached.
    pub fn is_expired(&self, now: Instant) -> bool {
        self.state.phase == RecoveryPhase::Recovering
            && self.state.started.is_some_and(|started| {
                now.saturating_duration_since(started)
                    >= Duration::from_millis(self.state.deadline_ms)
            })
    }

    /// Validates that `generation` is the current generation.
    fn check_generation(&self, generation: u64) -> Result<(), RecoveryError> {
        if generation == self.state.generation {
            Ok(())
        } else {
            Err(RecoveryError::StaleGeneration {
                expected: self.state.generation,
                got: generation,
            })
        }
    }

    /// Records one probe attempt; exceeding the bound fails recovery.
    pub fn record_attempt(&mut self, generation: u64) -> Result<u32, RecoveryError> {
        self.check_generation(generation)?;
        if self.state.phase != RecoveryPhase::Recovering {
            return Err(RecoveryError::InvalidTransition);
        }
        if self.attempts >= self.max_attempts() {
            self.state.phase = RecoveryPhase::Failed;
            return Err(RecoveryError::InvalidTransition);
        }
        self.attempts = self.attempts.saturating_add(1);
        Ok(self.attempts)
    }

    /// Completes recovery, returning the exact interrupted step.
    pub fn succeed(
        &mut self,
        generation: u64,
        _now: Instant,
    ) -> Result<RecoveryOutcome, RecoveryError> {
        self.check_generation(generation)?;
        if self.state.phase != RecoveryPhase::Recovering {
            return Err(RecoveryError::InvalidTransition);
        }
        let Some(step) = self.state.interrupted.clone() else {
            return Err(RecoveryError::InvalidTransition);
        };
        self.state.phase = RecoveryPhase::Recovered;
        Ok(RecoveryOutcome::Resumed(step))
    }

    /// Fails recovery only when its deadline has expired.
    pub fn expire(
        &mut self,
        generation: u64,
        now: Instant,
    ) -> Result<RecoveryOutcome, RecoveryError> {
        self.check_generation(generation)?;
        if self.state.phase != RecoveryPhase::Recovering || !self.is_expired(now) {
            return Err(RecoveryError::InvalidTransition);
        }
        self.state.phase = RecoveryPhase::Failed;
        Ok(RecoveryOutcome::TechnicalFailure)
    }

    /// Cancels the live recovery attempt.
    pub fn cancel(&mut self, generation: u64) -> Result<(), RecoveryError> {
        self.check_generation(generation)?;
        if self.state.phase != RecoveryPhase::Recovering {
            return Err(RecoveryError::InvalidTransition);
        }
        self.state.phase = RecoveryPhase::Cancelled;
        Ok(())
    }

    /// Moves `Recovered` back to `Healthy`; other phases are rejected.
    pub fn reset(&mut self) -> Result<(), RecoveryError> {
        if self.state.phase != RecoveryPhase::Recovered {
            return Err(RecoveryError::InvalidTransition);
        }
        self.state.phase = RecoveryPhase::Healthy;
        self.state.service = None;
        self.state.interrupted = None;
        self.state.started = None;
        self.attempts = 0;
        Ok(())
    }
    /// Acknowledges a recovered attempt, returning to `Healthy`.
    pub fn acknowledge(&mut self) -> Result<(), RecoveryError> {
        self.reset()
    }
}

/// Caller-clock pause accounting; duplicate pause/resume are no-ops.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PauseClock {
    paused_since: Option<u64>,
    paused_total_ms: u64,
}

/// Alias kept for later session-timer reuse.
pub type RecoveryTimer = PauseClock;

impl PauseClock {
    /// Creates a running clock with no paused time.
    pub fn new() -> Self {
        Self::default()
    }
    /// Starts a pause; returns false when already paused.
    pub fn pause(&mut self, at_ms: u64) -> bool {
        if self.paused_since.is_some() {
            return false;
        }
        self.paused_since = Some(at_ms);
        true
    }

    /// Ends a pause; returns false when not paused.
    pub fn resume(&mut self, at_ms: u64) -> bool {
        let Some(since) = self.paused_since.take() else {
            return false;
        };
        let delta = at_ms.saturating_sub(since);
        self.paused_total_ms = self.paused_total_ms.saturating_add(delta);
        true
    }

    /// Returns true while paused.
    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }
    /// Returns completed paused milliseconds, excluding any live pause.
    pub fn paused_total_ms(&self) -> u64 {
        self.paused_total_ms
    }

    /// Returns `now_ms` shifted back by all paused time.
    pub fn adjusted_now(&self, now_ms: u64) -> u64 {
        let current = self.paused_since.map_or(0, |s| now_ms.saturating_sub(s));
        now_ms.saturating_sub(self.paused_total_ms.saturating_add(current))
    }
}
