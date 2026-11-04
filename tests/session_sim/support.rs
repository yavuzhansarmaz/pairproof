use std::collections::BTreeSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use pairproof::core::{ApproachChoice, Difficulty, EventKind, QuestionRef, Session};

pub(super) const JD: &str =
    "Senior backend engineer. Python. hash tables, arrays, and graphs. mid level.";
pub(super) const META_WAIT_SECS: u64 = 180;
pub(super) const SYNC_WAIT_SECS: u64 = 90;

#[rustfmt::skip]
pub(super) fn now_ms() -> u64 {
    u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0))
        .unwrap_or(u64::MAX)
}

#[rustfmt::skip]
pub(super) fn chk(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

pub(super) fn event_kinds(session: &Session) -> Vec<&'static str> {
    session
        .events()
        .iter()
        .map(|e| match &e.kind {
            EventKind::SessionCreated => "SessionCreated",
            EventKind::CandidateConnected => "CandidateConnected",
            EventKind::ApproachSelected { .. } => "ApproachSelected",
            EventKind::TestsRequested { .. } => "TestsRequested",
            EventKind::QuestionFinished { .. } => "QuestionFinished",
            EventKind::QuestionSkipped { .. } => "QuestionSkipped",
            EventKind::ReQuestionRequested { .. } => "ReQuestionRequested",
            EventKind::ReQuestionResolved { .. } => "ReQuestionResolved",
            EventKind::Continued { .. } => "Continued",
            EventKind::Redirected { .. } => "Redirected",
            EventKind::ApproachChanged { .. } => "ApproachChanged",
            EventKind::QuestionReopened { .. } => "QuestionReopened",
            EventKind::TestRunCompleted { .. } => "TestRunCompleted",
            EventKind::CandidateDisconnected => "CandidateDisconnected",
            _ => "Other",
        })
        .collect()
}

pub(super) fn has_kind(session: &Session, name: &str) -> bool {
    event_kinds(session).contains(&name)
}

pub(super) fn selected_presets(session: &Session) -> BTreeSet<u8> {
    session
        .events()
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ApproachSelected {
                choice: ApproachChoice::Preset(n),
                ..
            } => Some(*n),
            _ => None,
        })
        .collect()
}

pub(super) fn other_texts(session: &Session) -> Vec<String> {
    session
        .events()
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ApproachSelected {
                choice: ApproachChoice::Other(text),
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn unused_medium(session: &Session) -> Result<QuestionRef, String> {
    const SLUGS: &[&str] = &[
        "clone-graph",
        "letter-combinations-of-a-phone-number",
        "generate-parentheses",
        "number-of-islands",
        "course-schedule",
        "kth-smallest-element-in-a-bst",
        "binary-tree-level-order-traversal",
    ];
    let used: BTreeSet<&str> = session
        .slots()
        .iter()
        .map(|slot| slot.question.title_slug.as_str())
        .collect();
    let slug = SLUGS
        .iter()
        .copied()
        .find(|slug| !used.contains(slug))
        .ok_or_else(|| format!("no unused medium slug; used={used:?}"))?;
    Ok(QuestionRef {
        title_slug: slug.into(),
        title: slug.into(),
        difficulty: Difficulty::Medium,
        acceptance_pct: None,
    })
}

pub(super) fn solution_synced(workspace: &Path, index: usize) -> bool {
    pairproof::tui::candidate_sync::has_synced_solution(workspace, index)
}
