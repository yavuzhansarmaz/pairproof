//! Byte snapshots and bulk-insert (paste) detection for synced files.
//! Threshold: bytes >= 2048 OR lines >= 50 yields one candidate flag.

/// Byte growth that alone marks a bulk-insert candidate.
pub const PASTE_BYTE_THRESHOLD: u64 = 2048;
/// Newline growth that alone marks a bulk-insert candidate.
pub const PASTE_LINE_THRESHOLD: u64 = 50;

/// Byte snapshot of one watched file between two polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    len: u64,
    newlines: u64,
}

impl Snapshot {
    /// Captures byte length and `b'\n'` count of the content.
    pub fn from_bytes(content: &[u8]) -> Self {
        Self {
            len: content.len() as u64,
            newlines: content.iter().filter(|b| **b == b'\n').count() as u64,
        }
    }

    /// Byte length of the snapshotted content.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// True when the snapshotted content is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Count of `b'\n'` bytes in the snapshotted content.
    pub fn newline_count(&self) -> u64 {
        self.newlines
    }
}

/// A bulk insert between two snapshots; candidacy only, not authorship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasteFlag {
    /// Growth in bytes, `current.len() - previous.len()` saturating at zero.
    pub inserted_bytes: u64,
    /// Growth in `b'\n'` count, saturating at zero.
    pub inserted_lines: u64,
}

// Only expert-confirmed flags affect scoring (Phase 8+).
// Threshold rationale: hand-typing 2KB or 50 lines between two polls is
// implausible, while a pasted LeetCode solution is typically 1-3KB / 30-80
// lines. AI writes go through docker-exec write_file, NOT through sync, so
// sync-observed bulk inserts are candidate-side edits. A large AI-sized
// rewrite above threshold is ALSO a candidate: two snapshots cannot tell who
// authored the bytes, so the detector reports size, never authorship.
// Scoring counts only CONFIRMED flags ("per confirmed bulk paste");
// confirmation is expert judgment in Phase 8+.
// Limits: same-size rewrites are invisible (byte delta zero), a delete-then-
// paste within one poll interval nets out, and bytes are counted, not chars.
//
// Polling/watch wiring belongs to Phases 9/10; this file is detector only.
pub fn detect_bulk_insert(previous: &Snapshot, current: &Snapshot) -> Option<PasteFlag> {
    let inserted_bytes = current.len.saturating_sub(previous.len);
    let inserted_lines = current.newlines.saturating_sub(previous.newlines);
    if inserted_bytes >= PASTE_BYTE_THRESHOLD || inserted_lines >= PASTE_LINE_THRESHOLD {
        Some(PasteFlag {
            inserted_bytes,
            inserted_lines,
        })
    } else {
        None
    }
}
