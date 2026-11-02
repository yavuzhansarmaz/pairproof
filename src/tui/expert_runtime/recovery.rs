//! Non-blocking recovery integration for `ExpertRuntime`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::SessionStatus;
use crate::orchestration::health::{
    InterruptedStep, RecoveryOutcome, ServiceKind, RECOVERY_STEP_MS,
};
use crate::orchestration::recovery::{
    classify_pipeline_recoverability, is_recoverable, RecoveryPhase,
};
use crate::orchestration::{
    build_snapshot_with_recovery, publish_snapshot, CancelToken, HealthGate, LeetCodeProbe,
    MetaProbe, PipelineError, Probes, RetryPolicy, ServiceHealth, SnapshotRecovery,
};

use super::{CannedFetcher, CannedGenerator, ExpertRuntime, ExpertRuntimeError};

/// Snapshot of recovery state for later UI/snapshot slices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySnapshot {
    pub generation: u64,
    pub phase: RecoveryPhase,
    pub service: Option<ServiceKind>,
    pub remaining_ms: u64,
    pub interrupted: Option<InterruptedStep>,
}

impl ExpertRuntime {
    pub fn set_health_probes(
        &mut self,
        leetcode: Arc<dyn LeetCodeProbe>,
        meta: Arc<dyn MetaProbe>,
    ) {
        self.leetcode_probe = Some(leetcode);
        self.meta_probe = Some(meta);
    }

    pub fn set_recovery_window_ms(&mut self, window_ms: u64) {
        if self.recovery.phase() == RecoveryPhase::Healthy {
            self.recovery = crate::orchestration::RecoveryController::new(window_ms);
            self.health = HealthGate::new(RetryPolicy::new(window_ms));
        }
    }

    #[rustfmt::skip]
    pub fn recovery_phase(&self) -> RecoveryPhase { self.recovery.phase() }
    #[rustfmt::skip]
    pub fn recovery_remaining_ms(&self) -> u64 { self.recovery.remaining_ms(Instant::now()) }

    pub fn recovery_state_for_snapshot(&self) -> RecoverySnapshot {
        let state = self.recovery.state();
        RecoverySnapshot {
            generation: state.generation,
            phase: state.phase,
            service: state.service,
            remaining_ms: self.recovery_remaining_ms(),
            interrupted: state.interrupted,
        }
    }

    pub fn snapshot_recovery(&self) -> Option<SnapshotRecovery> {
        let snap = self.recovery_state_for_snapshot();
        match snap.phase {
            RecoveryPhase::Recovering => Some(SnapshotRecovery::from_service(
                true,
                snap.service,
                snap.remaining_ms,
                snap.generation,
                false,
            )),
            RecoveryPhase::Failed => Some(SnapshotRecovery::from_service(
                false,
                snap.service,
                snap.remaining_ms,
                snap.generation,
                true,
            )),
            _ => None,
        }
    }

    pub fn adjusted_now_ms(&self, now_ms: u64) -> u64 {
        self.pause_clock.adjusted_now(now_ms)
    }

    /// Returns true while the interview timer is paused (recovery).
    pub fn timer_paused(&self) -> bool {
        self.pause_clock.is_paused()
    }

    /// Returns completed paused milliseconds, excluding any live pause.
    pub fn paused_total_ms(&self) -> u64 {
        self.pause_clock.paused_total_ms()
    }

    #[rustfmt::skip]
    pub fn is_recovery_active(&self) -> bool { self.recovery.phase() == RecoveryPhase::Recovering }
    #[rustfmt::skip]
    pub fn recovery_timed_out(&self) -> bool { self.recovery_timed_out }

    pub fn recovery_lines(&self) -> Vec<String> {
        match self.recovery.phase() {
            RecoveryPhase::Recovering => {
                let name = self
                    .recovery
                    .state()
                    .service
                    .map(SnapshotRecovery::service_name)
                    .unwrap_or("service");
                let secs = self.recovery_remaining_ms().saturating_add(999) / 1000;
                vec![
                    format!("service temporarily unavailable: {name}"),
                    format!("recovering... about {secs} seconds remaining."),
                ]
            }
            RecoveryPhase::Failed => vec![
                "service recovery failed.".to_string(),
                "session ended as a technical failure.".to_string(),
            ],
            RecoveryPhase::Recovered => vec!["service recovered.".to_string()],
            RecoveryPhase::Cancelled => Vec::new(),
            RecoveryPhase::Healthy => Vec::new(),
        }
    }

    pub fn begin_recovery(
        &mut self,
        service: ServiceKind,
        step: InterruptedStep,
        now_ms: u64,
    ) -> Result<u64, ExpertRuntimeError> {
        if self.ended || self.supervisor.is_killed() || self.supervisor.token().is_cancelled() {
            return Err(ExpertRuntimeError::Message("session ended".to_string()));
        }
        let generation = self
            .recovery
            .begin(service, step.clone(), Instant::now())
            .map_err(|e| ExpertRuntimeError::Message(e.to_string()))?;
        self.pause_clock.pause(now_ms);
        if let (Some(leetcode), Some(meta)) = (self.leetcode_probe.clone(), self.meta_probe.clone())
        {
            let (tx, rx) = tokio::sync::oneshot::channel();
            self.recovery_rx = Some((generation, rx));
            let cancel = CancelToken::new();
            self.recovery_cancel = Some(cancel.clone());
            let gate = self.health;
            let window = Duration::from_millis(gate.policy().window_ms);
            self.supervisor.spawn("recovery", async move {
                let start = Instant::now();
                loop {
                    let up = tokio::select! {
                        _ = cancel.cancelled() => return,
                        up = async {
                            gate.probe_both(&Probes::new(&*leetcode, &*meta))
                                .await
                                .iter()
                                .all(ServiceHealth::is_up)
                        } => up,
                    };
                    if up {
                        let _ = tx.send(RecoveryOutcome::Resumed(step.clone()));
                        return;
                    }
                    let elapsed = start.elapsed();
                    if elapsed >= window {
                        let _ = tx.send(RecoveryOutcome::TechnicalFailure);
                        return;
                    }
                    let sleep_for = Duration::from_millis(RECOVERY_STEP_MS).min(window - elapsed);
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = tokio::time::sleep(sleep_for) => {}
                    }
                }
            });
        } else {
            self.recovery_rx = None;
            self.recovery_cancel = None;
        }
        Ok(generation)
    }

    pub fn cancel_recovery(&mut self) {
        if let Some(token) = self.recovery_cancel.take() {
            token.cancel();
        }
        if self.recovery.phase() == RecoveryPhase::Recovering {
            let generation = self.recovery.generation();
            let _ = self.recovery.cancel(generation);
        }
        self.recovery_rx = None;
    }

    pub async fn enter_technical_failure(&mut self, now_ms: u64) {
        self.recovery_timed_out = true;
        if !self.orch.is_terminal() {
            let _ = self.orch.mark_technical_failure(now_ms);
        }
        let snapshot = build_snapshot_with_recovery(
            &self.orch,
            &self.pipeline,
            self.sync_phase,
            self.executor.chat_replies(),
            None,
            self.snapshot_recovery(),
        );
        let _ = publish_snapshot(&*self.control_io, &snapshot).await;
    }

    async fn expire_recovery(&mut self, now_ms: u64) -> Option<RecoveryOutcome> {
        let generation = self.recovery.generation();
        let Ok(outcome) = self.recovery.expire(generation, Instant::now()) else {
            return None;
        };
        self.recovery_rx = None;
        self.recovery_cancel = None;
        self.enter_technical_failure(now_ms).await;
        Some(outcome)
    }

    pub async fn poll_recovery(&mut self, now_ms: u64) -> Option<RecoveryOutcome> {
        if self.recovery.phase() != RecoveryPhase::Recovering && self.recovery_rx.is_none() {
            return None;
        }
        if self.ended || self.supervisor.is_killed() || self.supervisor.token().is_cancelled() {
            self.recovery_rx = None;
            return None;
        }
        // (a) Non-blocking consumption of the spawned gate result.
        enum Recv {
            Empty,
            Closed,
            Outcome(RecoveryOutcome),
        }
        let mut generation = None;
        let recv = match &mut self.recovery_rx {
            Some((gen, rx)) => {
                generation = Some(*gen);
                match rx.try_recv() {
                    Ok(outcome) => Recv::Outcome(outcome),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Recv::Empty,
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Recv::Closed,
                }
            }
            None => Recv::Empty,
        };
        match recv {
            Recv::Outcome(outcome) => {
                let gen = generation.unwrap_or_default();
                if !self.recovery.accepts(gen) {
                    self.recovery_rx = None;
                    return None;
                }
                match outcome {
                    RecoveryOutcome::Resumed(step) => {
                        match self.recovery.succeed(gen, Instant::now()) {
                            Ok(resumed) => {
                                self.pause_clock.resume(now_ms);
                                let _ = self.recovery.reset();
                                self.recovery_rx = None;
                                self.recovery_cancel = None;
                                let should_retry = matches!(
                                    step,
                                    InterruptedStep::FetchStatement { .. }
                                        | InterruptedStep::GenerateApproaches { .. }
                                );
                                if should_retry
                                    && !self.ended
                                    && !self.supervisor.is_killed()
                                    && !self.supervisor.token().is_cancelled()
                                {
                                    self.maybe_trigger_load();
                                }
                                return Some(resumed);
                            }
                            Err(_) => {
                                self.recovery_rx = None;
                                return None;
                            }
                        }
                    }
                    RecoveryOutcome::TechnicalFailure => {
                        if !self.recovery.is_expired(Instant::now()) {
                            self.recovery_rx = None;
                            self.recovery_cancel = None;
                            return None;
                        }
                        return self.expire_recovery(now_ms).await;
                    }
                }
            }
            Recv::Closed => {
                self.recovery_rx = None;
                self.recovery_cancel = None;
            }
            Recv::Empty => {}
        }
        // (b) Monotonic deadline expiry.
        if self.recovery.is_expired(Instant::now()) {
            return self.expire_recovery(now_ms).await;
        }
        None
    }

    pub(crate) fn handle_load_failure(&mut self, err: &PipelineError, now_ms: u64) {
        if !is_recoverable(classify_pipeline_recoverability(err)) {
            return;
        }
        let (service, step) = match err {
            PipelineError::Fetch(_) => {
                let slug = self
                    .orch
                    .current_question()
                    .map(|q| q.title_slug.clone())
                    .unwrap_or_default();
                if slug.is_empty() {
                    (
                        ServiceKind::LeetCode,
                        InterruptedStep::CodingStep {
                            index: self.orch.current_index().unwrap_or(0),
                        },
                    )
                } else {
                    (
                        ServiceKind::LeetCode,
                        InterruptedStep::FetchStatement { slug },
                    )
                }
            }
            PipelineError::Llm(_) => (
                ServiceKind::Meta,
                InterruptedStep::GenerateApproaches {
                    index: self.orch.current_index().unwrap_or(0),
                },
            ),
            _ => (
                ServiceKind::Meta,
                InterruptedStep::CodingStep {
                    index: self.orch.current_index().unwrap_or(0),
                },
            ),
        };
        let _ = self.begin_recovery(service, step, now_ms);
    }

    pub(crate) async fn apply_pending_load(&mut self, now_ms: u64) {
        let Some((index, mut rx)) = self.pending_load.take() else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(outcome) => outcome,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.pending_load = Some((index, rx));
                return;
            }
            Err(_) => return,
        };
        if self.supervisor.token().is_cancelled() {
            return;
        }
        if outcome.session_id != self.orch.session_id()
            || self.orch.current_index() != Some(outcome.index)
        {
            return;
        }
        match outcome.result {
            Ok(Ok((fetched, set))) => {
                let fetcher = CannedFetcher(fetched);
                if self
                    .pipeline
                    .load_current_statement(&self.orch, &fetcher)
                    .await
                    .is_err()
                {
                    return;
                }
                let generator = CannedGenerator(std::sync::Mutex::new(Some(set)));
                if self
                    .pipeline
                    .generate_current_approaches(&self.orch, &generator)
                    .await
                    .is_ok()
                {
                    self.loaded_index = Some(outcome.index);
                }
            }
            Ok(Err(err)) => {
                self.handle_load_failure(&err, now_ms);
            }
            Err(_) => {}
        }
    }

    pub(crate) fn is_technical_failure(&self) -> bool {
        self.orch.status() == SessionStatus::TechnicalFailure
    }
}
