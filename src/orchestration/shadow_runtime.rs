//! Non-blocking shadow evaluation runtime.
//! Shadow reports are expert-side evidence only; they never feed
//! production scoring, candidate views, or session state.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::core::{ApproachChoice, ArtifactLayout, EventKind, Session};
use crate::llm::shadow_eval::{evaluate_shadow_set, report_or_failed};
use crate::llm::MetaClient;
use crate::orchestration::lifecycle::{run_cancellable, CancelToken, RequestSupervisor};
use crate::orchestration::pipeline::PipelineFuture;
use crate::scoring::shadow::{assess_decision, fingerprint_for, other_evaluation};
use crate::scoring::{ApproachGrade, ApproachSet, ShadowReport, ShadowStatus};

/// Bounded wait for one shadow evaluation.
pub const SHADOW_TIMEOUT: Duration = Duration::from_secs(60);

/// Narrow boundary for one independent shadow evaluation.
pub trait ShadowEvaluator: Send + Sync {
    /// Evaluates `set` for `slot` against `statement`.
    fn evaluate<'a>(
        &'a self,
        statement: &'a str,
        set: &'a ApproachSet,
        slot: usize,
    ) -> PipelineFuture<'a, ShadowReport>;
}

/// Production evaluator backed by the strong Meta model. Usage is
/// dropped: shadow tokens are never booked on the session.
#[derive(Debug)]
pub struct MetaShadowEvaluator {
    client: MetaClient,
}

impl MetaShadowEvaluator {
    /// Creates an evaluator wrapping `client`.
    pub fn new(client: MetaClient) -> Self {
        Self { client }
    }
}

impl ShadowEvaluator for MetaShadowEvaluator {
    fn evaluate<'a>(
        &'a self,
        statement: &'a str,
        set: &'a ApproachSet,
        slot: usize,
    ) -> PipelineFuture<'a, ShadowReport> {
        Box::pin(async move {
            let model = self.client.strong_model.clone();
            let result = evaluate_shadow_set(&self.client, statement, set, slot)
                .await
                .map(|outcome| outcome.value);
            Ok(report_or_failed(result, slot, &model))
        })
    }
}

struct ShadowOutcome {
    session_id: String,
    slot: usize,
    fingerprint: String,
    report: Option<ShadowReport>,
}

struct PendingShadow {
    key: (usize, String),
    rx: oneshot::Receiver<ShadowOutcome>,
}

/// Owns shadow snapshots, in-flight evaluations, and persisted reports.
#[derive(Default)]
pub struct ShadowRuntime {
    snapshots: HashMap<(usize, String), ApproachSet>,
    pending: Vec<PendingShadow>,
    reports: BTreeMap<usize, ShadowReport>,
    persisted: HashSet<(usize, String)>,
    report_keys: HashMap<usize, (usize, String)>,
    other_grades: HashMap<usize, ApproachGrade>,
    latest_key: HashMap<usize, (usize, String)>,
    persist_dirty: bool,
}

impl ShadowRuntime {
    /// Creates an empty runtime with nothing in flight.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seeds persisted state from an existing artifact file so a
    /// rebuilt runtime never re-requests an evaluated set.
    pub fn seed_persisted(&mut self, layout: &ArtifactLayout) {
        let Ok(text) = std::fs::read_to_string(layout.shadow_path()) else {
            return;
        };
        let Ok(reports) = serde_json::from_str::<Vec<ShadowReport>>(&text) else {
            return;
        };
        for report in reports {
            if report.fingerprint.is_empty() {
                continue;
            }
            let key = (report.slot_index, report.fingerprint.clone());
            self.persisted.insert(key.clone());
            self.report_keys.insert(report.slot_index, key.clone());
            self.latest_key.insert(report.slot_index, key);
            self.reports.insert(report.slot_index, report);
        }
    }

    /// Spawns a shadow evaluation for `(slot, fingerprint)` unless one
    /// is already pending, reported, or persisted. Returns `true` only
    /// when a new task was spawned on `supervisor`.
    #[allow(clippy::too_many_arguments)]
    pub fn trigger(
        &mut self,
        supervisor: &mut RequestSupervisor,
        evaluator: Arc<dyn ShadowEvaluator>,
        token: CancelToken,
        session_id: String,
        slot: usize,
        statement: String,
        set: ApproachSet,
    ) -> bool {
        let fingerprint = fingerprint_for(&set);
        let key = (slot, fingerprint.clone());
        if self.snapshots.contains_key(&key)
            || self.pending.iter().any(|p| p.key == key)
            || self.persisted.contains(&key)
        {
            return false;
        }
        self.latest_key.insert(slot, key.clone());
        self.snapshots.insert(key.clone(), set.clone());
        let (tx, rx) = oneshot::channel();
        self.pending.push(PendingShadow {
            key: key.clone(),
            rx,
        });
        supervisor.spawn(format!("shadow-q{slot}"), async move {
            let timed =
                tokio::time::timeout(SHADOW_TIMEOUT, evaluator.evaluate(&statement, &set, slot));
            let report = match run_cancellable(&token, timed).await {
                Ok(Ok(Ok(report))) => Some(report),
                Ok(Ok(Err(err))) => Some(ShadowReport::failed(
                    slot,
                    "shadow-evaluator".to_string(),
                    err.to_string(),
                )),
                Ok(Err(_)) => Some(ShadowReport::failed(
                    slot,
                    "shadow-evaluator".to_string(),
                    "shadow evaluation timed out".to_string(),
                )),
                Err(_) => None,
            };
            let _ = tx.send(ShadowOutcome {
                session_id,
                slot,
                fingerprint,
                report,
            });
        });
        true
    }

    /// Records the production grade for an `Other` selection on `slot`.
    pub fn record_other(&mut self, slot: usize, grade: ApproachGrade) {
        self.other_grades.insert(slot, grade);
    }

    /// Non-blocking poll: consumes finished evaluations with
    /// `try_recv`, drops wrong-session, stale, cancelled, and
    /// already-persisted outcomes, and persists every report through
    /// `layout` only when a new `(slot, fingerprint)` key arrives.
    /// Returns how many reports were newly persisted in this call.
    pub fn poll(&mut self, session_id: &str, session: &Session, layout: &ArtifactLayout) -> usize {
        let pending = std::mem::take(&mut self.pending);
        let mut newly = 0usize;
        for mut item in pending {
            let outcome = match item.rx.try_recv() {
                Ok(outcome) => outcome,
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending.push(item);
                    continue;
                }
                Err(oneshot::error::TryRecvError::Closed) => continue,
            };
            if outcome.session_id != session_id {
                continue;
            }
            let Some(mut report) = outcome.report else {
                continue;
            };
            let key = (outcome.slot, outcome.fingerprint.clone());
            if self.latest_key.get(&outcome.slot) != Some(&key) {
                continue;
            }
            if self.persisted.contains(&key) || self.report_keys.get(&outcome.slot) == Some(&key) {
                continue;
            }
            let Some(set) = self.snapshots.get(&key) else {
                continue;
            };
            if report.slot_index != outcome.slot
                || (!report.fingerprint.is_empty() && report.fingerprint != fingerprint_for(set))
            {
                continue;
            }
            let last_choice = session
                .events()
                .iter()
                .rev()
                .find_map(|event| match &event.kind {
                    EventKind::ApproachSelected { index, choice } if *index == outcome.slot => {
                        Some(choice.clone())
                    }
                    _ => None,
                });
            if let Some(choice) = last_choice.filter(|_| {
                matches!(
                    report.status,
                    ShadowStatus::Complete | ShadowStatus::Partial { .. }
                )
            }) {
                report.options.retain(|o| o.option_index < 4);
                if let ApproachChoice::Other(_) = choice {
                    if let Some(grade) = self.other_grades.get(&outcome.slot) {
                        report.options.push(other_evaluation(grade));
                    }
                }
                report.decision = Some(assess_decision(&report.options.clone(), choice));
            }
            self.report_keys.insert(outcome.slot, key);
            self.reports.insert(outcome.slot, report);
            newly += 1;
        }
        // A selection may arrive after its evaluation completed; assess
        // stored reports whenever the latest choice becomes available.
        let mut changed = newly > 0;
        for (slot, report) in self.reports.iter_mut() {
            if !matches!(
                report.status,
                ShadowStatus::Complete | ShadowStatus::Partial { .. }
            ) {
                continue;
            }
            let last_choice = session
                .events()
                .iter()
                .rev()
                .find_map(|event| match &event.kind {
                    EventKind::ApproachSelected { index, choice } if index == slot => {
                        Some(choice.clone())
                    }
                    _ => None,
                });
            let Some(choice) = last_choice else { continue };
            if report.decision.as_ref().is_some_and(|d| d.choice == choice) {
                continue;
            }
            report.options.retain(|o| o.option_index < 4);
            if let ApproachChoice::Other(_) = choice {
                if let Some(grade) = self.other_grades.get(slot) {
                    report.options.push(other_evaluation(grade));
                }
            }
            report.decision = Some(assess_decision(&report.options.clone(), choice));
            changed = true;
        }
        if changed {
            self.persist_dirty = true;
        }
        if !self.persist_dirty {
            return 0;
        }
        let all: Vec<ShadowReport> = self.reports.values().cloned().collect();
        if layout.write_shadow_evaluations(&all).is_err() {
            return 0;
        }
        self.persist_dirty = false;
        for key in self.report_keys.values() {
            self.persisted.insert(key.clone());
        }
        newly
    }

    /// Drops every pending receiver; in-flight tasks are aborted by
    /// the supervisor kill switch at session end.
    pub fn shutdown(&mut self) {
        self.pending.clear();
    }

    /// Accepted reports keyed by slot index.
    pub fn reports(&self) -> &BTreeMap<usize, ShadowReport> {
        &self.reports
    }

    /// One accepted report for `slot`, if any.
    pub fn report(&self, slot: usize) -> Option<&ShadowReport> {
        self.reports.get(&slot)
    }

    /// How many evaluations are still in flight.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Whether `(slot, fingerprint)` was already persisted.
    pub fn is_persisted(&self, slot: usize, fingerprint: &str) -> bool {
        self.persisted.contains(&(slot, fingerprint.to_string()))
    }
}
