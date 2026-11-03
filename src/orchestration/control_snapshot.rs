//! Candidate snapshot projections and their builder: only
//! candidate-safe data (titles, statement, approach titles and
//! summaries, recorded selection, latest test, navigation list)
//! ever crosses to the candidate here.

use serde::{Deserialize, Serialize};

use crate::core::ApproachChoice;
use crate::orchestration::control::SNAPSHOT_VERSION;
use crate::orchestration::finalize::final_visible_to_candidate;
use crate::orchestration::health::ServiceKind;
use crate::orchestration::pipeline::QuestionPipeline;
use crate::orchestration::session::Orchestrator;
use crate::orchestration::sync_gate::SessionSyncPhase;
use crate::scoring::FinalBreakdown;

/// Candidate-visible question projection.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotQuestion { pub index: usize, pub title: String, pub slug: String, pub statement: Option<String> }
/// Candidate-visible approach option: title and summary only.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotOption { pub title: String, pub summary: String }
/// Candidate-visible latest test summary.
#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotTest { pub passed: u32, pub failed: u32, pub at_ms: u64 }
/// Final per-question result, present only inside a final snapshot.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotFinalQuestion { pub slot_index: usize, #[serde(default)] pub title: String, #[serde(default)] pub state: String, pub grade: Option<f64>, pub rationale: Option<String> }
/// Candidate-visible final breakdown projection (rating and grades only).
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotFinal { pub rating: f64, pub per_question: Vec<SnapshotFinalQuestion> }

/// Candidate-safe recovery projection; no reason/body/status strings.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRecovery { pub active: bool, pub service: Option<String>, pub remaining_ms: u64, pub generation: u64, pub technical_failure: bool }
#[rustfmt::skip]
impl SnapshotRecovery {
    pub fn service_name(service: ServiceKind) -> &'static str { match service { ServiceKind::Meta => "Meta", ServiceKind::LeetCode => "LeetCode" } }
    pub fn from_service(active: bool, service: Option<ServiceKind>, remaining_ms: u64, generation: u64, technical_failure: bool) -> Self { Self { active, service: service.map(|s| Self::service_name(s).to_string()), remaining_ms, generation, technical_failure } }
}
#[rustfmt::skip]
pub fn snapshot_service_name(service: ServiceKind) -> &'static str { SnapshotRecovery::service_name(service) }

/// One question in the candidate's navigation list: index, title,
/// and lifecycle status only (no statement or approach internals).
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotNav { pub index: usize, pub title: String, pub state: String }

/// Candidate-visible approach selection with an explicit kind; the
/// displayed marker must never be inferred from custom text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotSelection {
    Preset(u8),
    Other(String),
}

/// Versioned, session-bound candidate snapshot. Deliberately has no
/// field for quality, best index, expected tokens, or grading internals.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateSnapshot {
    pub v: u32, pub session_id: String, pub phase: String, pub sync_phase: String,
    pub current: Option<SnapshotQuestion>, pub options: Vec<SnapshotOption>,
    #[serde(default)] pub nav: Vec<SnapshotNav>,
    pub selected: Option<SnapshotSelection>, pub latest_test: Option<SnapshotTest>,
    pub chat_replies: Vec<String>, pub final_breakdown: Option<SnapshotFinal>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub chat_partial: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub recovery: Option<SnapshotRecovery>,
}

/// Builds a snapshot only from candidate-safe projections. The final
/// breakdown is included only in Completed/EndedByExpert sessions.
#[rustfmt::skip]
pub fn build_snapshot(orch: &Orchestrator, pipe: &QuestionPipeline, sync_phase: SessionSyncPhase,
    chat_replies: &[String], breakdown: Option<&FinalBreakdown>) -> CandidateSnapshot {
    build_snapshot_with_recovery(orch, pipe, sync_phase, chat_replies, breakdown, None)
}
#[rustfmt::skip]
pub fn build_snapshot_with_recovery(orch: &Orchestrator, pipe: &QuestionPipeline, sync_phase: SessionSyncPhase,
    chat_replies: &[String], breakdown: Option<&FinalBreakdown>, recovery: Option<SnapshotRecovery>) -> CandidateSnapshot {
    let slot = orch.session().current_slot();
    let current = orch.current_question().map(|q| SnapshotQuestion {
        index: orch.current_index().unwrap_or_default(), title: q.title.clone(), slug: q.title_slug.clone(),
        statement: pipe.current_statement(orch).map(|f| f.statement) });
    let selected = slot.and_then(|s| s.approach.as_ref()).map(|choice| match choice {
        ApproachChoice::Preset(n) => SnapshotSelection::Preset(*n), ApproachChoice::Other(text) => SnapshotSelection::Other(text.clone()) });
    let latest_test = slot.and_then(|s| s.latest_test).map(|t| SnapshotTest { passed: t.passed, failed: t.failed, at_ms: t.at_ms });
    let options = pipe.candidate_options().into_iter().map(|o| SnapshotOption { title: o.title, summary: o.summary }).collect();
    let final_breakdown = if final_visible_to_candidate(orch.status()) {
        breakdown.map(|b| SnapshotFinal { rating: b.rating, per_question: b.per_question.iter()
            .map(|q| { let slot = orch.session().slots().iter().find(|s| s.index == q.slot_index); SnapshotFinalQuestion { slot_index: q.slot_index, title: slot.map(|s| s.question.title.clone()).unwrap_or_default(), state: slot.map(|s| format!("{:?}", s.status)).unwrap_or_default(), grade: q.grade, rationale: q.rationale.clone() } }).collect() })
    } else { None };
    let nav = orch.session().slots().iter().map(|slot| SnapshotNav {
        index: slot.index, title: slot.question.title.clone(), state: format!("{:?}", slot.status) }).collect();
    CandidateSnapshot { v: SNAPSHOT_VERSION, session_id: orch.session_id().to_string(),
        phase: format!("{:?}", orch.status()), sync_phase: sync_phase.label().to_string(), current,
        options, nav, selected, latest_test, chat_replies: chat_replies.to_vec(), final_breakdown, chat_partial: None, recovery }
}
