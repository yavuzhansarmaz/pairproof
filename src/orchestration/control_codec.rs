//! Session-binding, final-ack, heartbeat, and snapshot wire codecs.

use serde::{Deserialize, Serialize};

use crate::orchestration::control::{
    CandidateSnapshot, ControlError, FINAL_ACK_VERSION, HEARTBEAT_VERSION, MAX_CONTROL_READ_BYTES,
    SESSION_VERSION, SNAPSHOT_VERSION,
};

/// Expert-published session binding in `outbox/session.json`. The
/// connection string carries no session id, so the candidate learns
/// the binding only by reading this file after SSH authentication.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding { pub v: u32, pub session_id: String }
/// Serializes the session binding for `session_id`.
#[rustfmt::skip]
pub fn session_binding_bytes(session_id: &str) -> Vec<u8> { serde_json::to_vec(&SessionBinding { v: SESSION_VERSION, session_id: session_id.into() }).unwrap_or_default() }
/// Parses a session binding, rejecting a wrong version or empty id.
#[rustfmt::skip]
pub fn parse_session_binding(data: &[u8]) -> Result<String, ControlError> {
    if data.len() > MAX_CONTROL_READ_BYTES { return Err(ControlError::Oversized); }
    let binding: SessionBinding = serde_json::from_slice(data)?;
    if binding.v != SESSION_VERSION { return Err(ControlError::WrongVersion(u64::from(binding.v))); }
    let id = binding.session_id.trim();
    if id.is_empty() || id.len() > 128 || id.bytes().any(|b| b.is_ascii_control()) { return Err(ControlError::WrongSession); }
    Ok(binding.session_id)
}

/// Candidate ack in `inbox/final-ack.json`: candidate saw the terminal final rating, exactly once.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalAck { pub v: u32, pub session_id: String, pub rating_seen: bool }
/// Outcome of the expert bounded wait for the final-snapshot ack.
#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalAckOutcome { Received, Timeout }
/// Serializes the final ack for `session_id` (`rating_seen: true`).
#[rustfmt::skip]
pub fn final_ack_bytes(session_id: &str) -> Vec<u8> { serde_json::to_vec(&FinalAck { v: FINAL_ACK_VERSION, session_id: session_id.into(), rating_seen: true }).unwrap_or_default() }
/// Parses a final ack, rejecting a wrong version, session, or unseen rating.
#[rustfmt::skip]
pub fn parse_final_ack(data: &[u8], expected_session: &str) -> Result<FinalAck, ControlError> { if data.len() > MAX_CONTROL_READ_BYTES { return Err(ControlError::Oversized); } let ack: FinalAck = serde_json::from_slice(data)?; if ack.v != FINAL_ACK_VERSION { Err(ControlError::WrongVersion(u64::from(ack.v))) } else if ack.session_id != expected_session { Err(ControlError::WrongSession) } else if !ack.rating_seen { Err(ControlError::Malformed("rating not seen".into())) } else { Ok(ack) } }
/// Returns whether `data` is a valid final ack for `expected_session`.
#[rustfmt::skip]
pub fn is_final_ack(data: &[u8], expected_session: &str) -> bool { parse_final_ack(data, expected_session).is_ok() }
/// Candidate liveness heartbeat: session-bound `at_ms` in candidate clock.
#[rustfmt::skip]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat { pub v: u32, pub session_id: String, pub at_ms: u64 }
/// Serializes a heartbeat for `session_id` stamped at `at_ms`.
#[rustfmt::skip]
pub fn heartbeat_bytes(session_id: &str, at_ms: u64) -> Vec<u8> { serde_json::to_vec(&Heartbeat { v: HEARTBEAT_VERSION, session_id: session_id.into(), at_ms }).unwrap_or_default() }
/// Parses a heartbeat, rejecting a wrong version or session binding.
#[rustfmt::skip]
pub fn parse_heartbeat(data: &[u8], expected_session: &str) -> Result<Heartbeat, ControlError> { if data.len() > MAX_CONTROL_READ_BYTES { return Err(ControlError::Oversized); } let hb: Heartbeat = serde_json::from_slice(data)?; if hb.v != HEARTBEAT_VERSION { Err(ControlError::WrongVersion(u64::from(hb.v))) } else if hb.session_id != expected_session { Err(ControlError::WrongSession) } else { Ok(hb) } }
/// Parses a snapshot and rejects a wrong version or session binding.
#[rustfmt::skip]
pub fn parse_snapshot(data: &[u8], expected_session: &str) -> Result<CandidateSnapshot, ControlError> { if data.len() > MAX_CONTROL_READ_BYTES { return Err(ControlError::Oversized); } let snap: CandidateSnapshot = serde_json::from_slice(data)?; if snap.v != SNAPSHOT_VERSION { Err(ControlError::WrongVersion(u64::from(snap.v))) } else if snap.session_id != expected_session { Err(ControlError::WrongSession) } else { Ok(snap) } }
