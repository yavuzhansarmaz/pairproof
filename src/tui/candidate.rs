//! Candidate screens: connection, rules, split question and chat view, final score.

/// Returns the Phase 9 placeholder message for candidates.
pub fn placeholder_message() -> &'static str {
    "Candidate flow is not implemented yet (Phase 9). \
     Please use the expert flow or check back later."
}

/// Returns the generic candidate-safe sync message.
pub fn candidate_sync_message() -> &'static str {
    "Sync problem \u{2014} ask the expert"
}
