//! Hardened control-channel types shared by candidate and expert.
//! Intents are strictly allowlisted and snapshots carry only
//! candidate-safe projections; no hidden grading data has a field here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::ApproachChoice;
use crate::orchestration::finalize::final_visible_to_candidate;
use crate::orchestration::pipeline::QuestionPipeline;
use crate::orchestration::session::Orchestrator;
use crate::orchestration::sync_gate::SessionSyncPhase;
use crate::scoring::FinalBreakdown;

/// Control root outside the `/workspace` Mutagen sync root.
pub const CONTROL_ROOT: &str = "/pairproof-control";
/// Candidate-writable inbox directory inside the control root.
pub const INBOX_DIR: &str = "/pairproof-control/inbox";
/// Expert-authored, candidate read-only outbox directory.
pub const OUTBOX_DIR: &str = "/pairproof-control/outbox";
/// Fixed inbox filename; the only intent file the expert ever reads.
pub const INTENT_FILE: &str = "intents.jsonl";
/// Fixed outbox filename for the atomically published snapshot.
pub const SNAPSHOT_FILE: &str = "snapshot.json";
/// Fixed outbox filename for the expert-published session binding.
pub const SESSION_FILE: &str = "session.json";
/// Candidate final-snapshot ack file (`inbox/final-ack.json`, v1).
pub const FINAL_ACK_FILE: &str = "final-ack.json";
pub const FINAL_ACK_REL: &str = "inbox/final-ack.json";
pub const FINAL_ACK_PATH: &str = "/pairproof-control/inbox/final-ack.json";
pub const FINAL_ACK_VERSION: u32 = 1;
/// Candidate liveness heartbeat file (`inbox/heartbeat.json`, v1).
pub const HEARTBEAT_FILE: &str = "heartbeat.json";
pub const HEARTBEAT_REL: &str = "inbox/heartbeat.json";
pub const HEARTBEAT_PATH: &str = "/pairproof-control/inbox/heartbeat.json";
pub const HEARTBEAT_VERSION: u32 = 1;
/// Current session-binding wire version.
pub const SESSION_VERSION: u32 = 1;
/// Expert-owned durable processed-intent ledger filename.
pub const PROCESSED_FILE: &str = "processed.json";
/// Root-relative path of the intent log.
pub const INTENT_REL: &str = "inbox/intents.jsonl";
/// Root-relative path of the snapshot file.
pub const SNAPSHOT_REL: &str = "outbox/snapshot.json";
/// Root-relative path of the session-binding file.
pub const SESSION_REL: &str = "outbox/session.json";
/// Container path of the expert-published session binding.
pub const SESSION_PATH: &str = "/pairproof-control/outbox/session.json";
/// Current intent wire version.
pub const INTENT_VERSION: u32 = 1;
/// Current snapshot wire version.
pub const SNAPSHOT_VERSION: u32 = 1;
/// Maximum serialized bytes of one intent line.
pub const MAX_INTENT_BYTES: usize = 8192;
/// Maximum bytes the expert reads from any one control file.
pub const MAX_CONTROL_READ_BYTES: usize = 256 * 1024;
const MAX_TEXT_BYTES: usize = 4096;
#[rustfmt::skip]
const FIXED_RELS: [&str; 5] = [INTENT_REL, SNAPSHOT_REL, SESSION_REL, PROCESSED_FILE, FINAL_ACK_REL];

/// Typed control-channel failure; payloads never carry hidden data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    #[error("unsupported control version {0}")]
    WrongVersion(u64),
    #[error("control message bound to another session")]
    WrongSession,
    #[error("invalid intent id")]
    InvalidId,
    #[error("intent exceeds {MAX_INTENT_BYTES} bytes")]
    Oversized,
    #[error("malformed control data: {0}")]
    Malformed(String),
    #[error("intent kind is not allowlisted: {0}")]
    ForbiddenKind(String),
    #[error("invalid intent payload: {0}")]
    InvalidPayload(String),
    #[error("sequence gap: expected {expected}, got {got}")]
    SequenceGap { expected: u64, got: u64 },
    #[error("control path escapes the control root: {0}")]
    PathEscape(String),
    #[error("control path is a symlink: {0}")]
    SymlinkRejected(String),
    #[error("control io error: {0}")]
    Io(String),
}
#[rustfmt::skip]
impl From<std::io::Error> for ControlError { fn from(err: std::io::Error) -> Self { Self::Io(err.kind().to_string()) } }
#[rustfmt::skip]
impl From<serde_json::Error> for ControlError { fn from(err: serde_json::Error) -> Self { Self::Malformed(err.to_string()) } }

/// The only actions a candidate may request. Expert-only decisions
/// (resolving a re-question, finalizing, grading, ending) have no variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentKind {
    SelectPreset { preset: u8 },
    SelectOther { text: String },
    Continue,
    Redirect { text: String },
    RequestTests,
    Finish,
    Skip,
    RequestReQuestion,
    ChatRequest { chat_id: String, text: String },
}
#[rustfmt::skip]
impl IntentKind {
    /// Wire name of this kind.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SelectPreset { .. } => "SelectPreset", Self::SelectOther { .. } => "SelectOther",
            Self::Continue => "Continue", Self::Redirect { .. } => "Redirect",
            Self::RequestTests => "RequestTests", Self::Finish => "Finish", Self::Skip => "Skip",
            Self::RequestReQuestion => "RequestRequestion", Self::ChatRequest { .. } => "ChatRequest",
        }
    }
}

/// One validated candidate intent.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent { pub v: u32, pub id: String, pub session_id: String, pub seq: u64, pub kind: IntentKind }
#[rustfmt::skip]
impl Intent {
    /// Creates an intent with a fresh random hex id.
    pub fn new(session_id: &str, seq: u64, kind: IntentKind) -> Self {
        Self { v: INTENT_VERSION, id: generate_intent_id(), session_id: session_id.into(), seq, kind }
    }
    /// Serializes to one JSONL line (without the trailing newline).
    pub fn to_line(&self) -> Result<String, ControlError> {
        let (kind, payload) = kind_payload(&self.kind);
        let wire = serde_json::json!({"v": self.v, "id": self.id, "session_id": self.session_id, "seq": self.seq, "kind": kind, "payload": payload});
        let line = serde_json::to_string(&wire)?;
        if line.len() > MAX_INTENT_BYTES { return Err(ControlError::Oversized); }
        Ok(line)
    }
    /// Validates version, id shape, session binding, and payload bounds.
    pub fn validate(&self, expected_session: &str) -> Result<(), ControlError> {
        if self.v != INTENT_VERSION { return Err(ControlError::WrongVersion(u64::from(self.v))); }
        if self.session_id != expected_session { return Err(ControlError::WrongSession); }
        let id_ok = (16..=64).contains(&self.id.len()) && self.id.bytes().all(|b| b.is_ascii_hexdigit());
        if !id_ok { return Err(ControlError::InvalidId); }
        if self.seq == 0 { return Err(ControlError::InvalidPayload("seq starts at 1".into())); }
        validate_kind(&self.kind)
    }
}

/// Canonical non-secret random hex: 16 `OsRng` bytes as 32 lowercase hex chars.
/// Not a credential, so it does not belong in `connection::secret`.
#[rustfmt::skip]
pub(crate) fn random_hex_16() -> String {
    let mut bytes = [0u8; 16]; OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Generates 16 random bytes as 32 lowercase hex characters.
#[rustfmt::skip]
pub fn generate_intent_id() -> String { random_hex_16() }

#[rustfmt::skip]
fn kind_payload(kind: &IntentKind) -> (&'static str, Value) {
    match kind {
        IntentKind::SelectPreset { preset } => ("SelectPreset", serde_json::json!({"preset": preset})),
        IntentKind::SelectOther { text } => ("SelectOther", serde_json::json!({"text": text})),
        IntentKind::Continue => ("Continue", serde_json::json!({})),
        IntentKind::Redirect { text } => ("Redirect", serde_json::json!({"text": text})),
        IntentKind::RequestTests => ("RequestTests", serde_json::json!({})),
        IntentKind::Finish => ("Finish", serde_json::json!({})),
        IntentKind::Skip => ("Skip", serde_json::json!({})),
        IntentKind::RequestReQuestion => ("RequestRequestion", serde_json::json!({})),
        IntentKind::ChatRequest { chat_id, text } => ("ChatRequest", serde_json::json!({"chat_id": chat_id, "text": text})),
    }
}

#[rustfmt::skip]
fn check_text(text: &str, field: &str) -> Result<(), ControlError> { if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES { return Err(ControlError::InvalidPayload(format!("bad {field}"))); } Ok(()) }
#[rustfmt::skip]
fn validate_kind(kind: &IntentKind) -> Result<(), ControlError> {
    match kind {
        IntentKind::SelectPreset { preset } if *preset > 3 => Err(ControlError::InvalidPayload("preset out of range".into())),
        IntentKind::SelectOther { text } | IntentKind::Redirect { text } => check_text(text, "text"),
        IntentKind::ChatRequest { chat_id, text } => {
            if chat_id.is_empty() || chat_id.len() > 128 { return Err(ControlError::InvalidPayload("bad chat_id".into())); }
            check_text(text, "text")
        }
        _ => Ok(()),
    }
}

#[rustfmt::skip]
fn payload_text(payload: &Value, field: &str) -> Result<String, ControlError> {
    payload.get(field).and_then(Value::as_str).map(str::to_string)
        .ok_or_else(|| ControlError::InvalidPayload(format!("missing {field}")))
}
#[rustfmt::skip]
fn parse_kind(name: &str, payload: &Value) -> Result<IntentKind, ControlError> {
    let kind = match name {
        "SelectPreset" => {
            let raw = payload.get("preset").and_then(Value::as_u64)
                .ok_or_else(|| ControlError::InvalidPayload("missing preset".into()))?;
            let preset = u8::try_from(raw).map_err(|_| ControlError::InvalidPayload("bad preset".into()))?;
            IntentKind::SelectPreset { preset }
        }
        "SelectOther" => IntentKind::SelectOther { text: payload_text(payload, "text")? },
        "Continue" => IntentKind::Continue,
        "Redirect" => IntentKind::Redirect { text: payload_text(payload, "text")? },
        "RequestTests" => IntentKind::RequestTests,
        "Finish" => IntentKind::Finish,
        "Skip" => IntentKind::Skip,
        "RequestRequestion" | "RequestReQuestion" => IntentKind::RequestReQuestion,
        "ChatRequest" => IntentKind::ChatRequest { chat_id: payload_text(payload, "chat_id")?, text: payload_text(payload, "text")? },
        other => return Err(ControlError::ForbiddenKind(other.to_string())),
    };
    validate_kind(&kind)?;
    Ok(kind)
}

/// Parses and validates one complete intent line for `expected_session`.
#[rustfmt::skip]
pub fn parse_intent_line(line: &str, expected_session: &str) -> Result<Intent, ControlError> {
    if line.len() > MAX_INTENT_BYTES { return Err(ControlError::Oversized); }
    let value: Value = serde_json::from_str(line)?;
    let obj = value.as_object().ok_or_else(|| ControlError::Malformed("not an object".into()))?;
    let v = obj.get("v").and_then(Value::as_u64).ok_or_else(|| ControlError::Malformed("missing v".into()))?;
    if v != u64::from(INTENT_VERSION) { return Err(ControlError::WrongVersion(v)); }
    let id = obj.get("id").and_then(Value::as_str).ok_or_else(|| ControlError::Malformed("missing id".into()))?.to_string();
    let session_id = obj.get("session_id").and_then(Value::as_str)
        .ok_or_else(|| ControlError::Malformed("missing session_id".into()))?.to_string();
    let seq = obj.get("seq").and_then(Value::as_u64).ok_or_else(|| ControlError::Malformed("missing seq".into()))?;
    let name = obj.get("kind").and_then(Value::as_str).ok_or_else(|| ControlError::Malformed("missing kind".into()))?;
    let payload = obj.get("payload").cloned().unwrap_or_else(|| Value::Object(Default::default()));
    let intent = Intent { v: INTENT_VERSION, id, session_id, seq, kind: parse_kind(name, &payload)? };
    intent.validate(expected_session)?;
    Ok(intent)
}

/// Result of parsing an intent log: valid intents plus typed rejections.
/// A trailing line without `\n` is an incomplete tail and is not consumed.
#[derive(Debug, Default)]
pub struct ParsedIntents {
    pub intents: Vec<Intent>,
    pub rejected: Vec<ControlError>,
}
/// Parses complete JSONL lines; malformed lines are skipped safely.
#[rustfmt::skip]
pub fn parse_intent_bytes(data: &[u8], expected_session: &str) -> ParsedIntents {
    let mut out = ParsedIntents::default();
    let complete = match data.iter().rposition(|b| *b == b'\n') { Some(pos) => &data[..=pos], None => return out };
    for line in String::from_utf8_lossy(complete).lines() {
        if line.trim().is_empty() { continue; }
        match parse_intent_line(line, expected_session) {
            Ok(intent) => out.intents.push(intent),
            Err(err) => out.rejected.push(err),
        }
    }
    out
}

/// Durable exactly-once ledger, persisted atomically after each apply.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessedState { pub session_id: String, pub ids: BTreeSet<String>, pub chat_ids: BTreeSet<String>, pub last_seq: u64 }
#[rustfmt::skip]
impl ProcessedState {
    pub fn new(session_id: &str) -> Self { Self { session_id: session_id.into(), ids: BTreeSet::new(), chat_ids: BTreeSet::new(), last_seq: 0 } }
    pub fn mark(&mut self, intent: &Intent) { self.ids.insert(intent.id.clone()); if let IntentKind::ChatRequest { chat_id, .. } = &intent.kind { self.chat_ids.insert(chat_id.clone()); } self.last_seq = intent.seq; }
}

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
pub struct SnapshotFinalQuestion { pub slot_index: usize, pub grade: Option<f64>, pub rationale: Option<String> }
/// Candidate-visible final breakdown projection (rating and grades only).
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotFinal { pub rating: f64, pub per_question: Vec<SnapshotFinalQuestion> }

/// Versioned, session-bound candidate snapshot. Deliberately has no
/// field for quality, best index, expected tokens, or grading internals.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateSnapshot {
    pub v: u32, pub session_id: String, pub phase: String, pub sync_phase: String,
    pub current: Option<SnapshotQuestion>, pub options: Vec<SnapshotOption>,
    pub selected: Option<String>, pub latest_test: Option<SnapshotTest>,
    pub chat_replies: Vec<String>, pub final_breakdown: Option<SnapshotFinal>,
}

/// Builds a snapshot only from candidate-safe projections. The final
/// breakdown is included only in Completed/EndedByExpert sessions.
#[rustfmt::skip]
pub fn build_snapshot(orch: &Orchestrator, pipe: &QuestionPipeline, sync_phase: SessionSyncPhase,
    chat_replies: &[String], breakdown: Option<&FinalBreakdown>) -> CandidateSnapshot {
    let slot = orch.session().current_slot();
    let current = orch.current_question().map(|q| SnapshotQuestion {
        index: orch.current_index().unwrap_or_default(), title: q.title.clone(), slug: q.title_slug.clone(),
        statement: pipe.current_statement(orch).map(|f| f.statement) });
    let selected = slot.and_then(|s| s.approach.as_ref()).map(|choice| match choice {
        ApproachChoice::Preset(n) => format!("preset {n}"), ApproachChoice::Other(text) => text.clone() });
    let latest_test = slot.and_then(|s| s.latest_test).map(|t| SnapshotTest { passed: t.passed, failed: t.failed, at_ms: t.at_ms });
    let options = pipe.candidate_options().into_iter().map(|o| SnapshotOption { title: o.title, summary: o.summary }).collect();
    let final_breakdown = if final_visible_to_candidate(orch.status()) {
        breakdown.map(|b| SnapshotFinal { rating: b.rating, per_question: b.per_question.iter()
            .map(|q| SnapshotFinalQuestion { slot_index: q.slot_index, grade: q.grade, rationale: q.rationale.clone() }).collect() })
    } else { None };
    CandidateSnapshot { v: SNAPSHOT_VERSION, session_id: orch.session_id().to_string(),
        phase: format!("{:?}", orch.status()), sync_phase: sync_phase.label().to_string(), current,
        options, selected, latest_test, chat_replies: chat_replies.to_vec(), final_breakdown }
}

pub use crate::orchestration::control_codec::{
    final_ack_bytes, heartbeat_bytes, is_final_ack, parse_final_ack, parse_heartbeat,
    parse_session_binding, parse_snapshot, session_binding_bytes, FinalAck, FinalAckOutcome,
    Heartbeat, SessionBinding,
};

/// Resolves a fixed control-relative path under `root`, rejecting
/// traversal, non-fixed names, and any symlink component.
#[rustfmt::skip]
pub fn resolve_control_file(root: &Path, rel: &str) -> Result<PathBuf, ControlError> {
    if !FIXED_RELS.contains(&rel) || rel.starts_with('/') || rel.contains('\\') {
        return Err(ControlError::PathEscape(rel.to_string()));
    }
    let joined = root.join(rel);
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut cursor = root.to_path_buf();
    for part in Path::new(rel).components() {
        cursor.push(part.as_os_str());
        if let Ok(meta) = std::fs::symlink_metadata(&cursor) {
            if meta.file_type().is_symlink() { return Err(ControlError::SymlinkRejected(rel.to_string())); }
        }
    }
    let parent = joined.parent().unwrap_or(root);
    if let Ok(canon_parent) = parent.canonicalize() {
        if !canon_parent.starts_with(&canon_root) { return Err(ControlError::PathEscape(rel.to_string())); }
    }
    if let Ok(canon) = joined.canonicalize() {
        if !canon.starts_with(&canon_root) { return Err(ControlError::PathEscape(rel.to_string())); }
    }
    Ok(joined)
}
