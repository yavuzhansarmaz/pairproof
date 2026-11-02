//! Score-to-event evidence links relative to the artifact bundle.
use crate::core::{EventKind, Session, SessionEvent};
use crate::scoring::{FinalBreakdown, PerQuestionFinal, ScoreComponent, ScoreLine};

/// Strength of the link between a score component and recorded events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    EventBacked,
    Partial,
    NoEventEvidence,
}

/// Reference to one line of the bundle-relative events file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventRef {
    pub seq: u64,
}
impl EventRef {
    /// Returns the 1-based line number for this event sequence.
    pub fn line_no(&self) -> u64 {
        self.seq + 1
    }
}

/// Evidence for one scored component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreEvidence {
    pub provenance: Provenance,
    pub refs: Vec<EventRef>,
    pub note: Option<&'static str>,
}

/// Evidence split for one per-question report line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerQuestionEvidence {
    pub grade: ScoreEvidence,
    pub tokens: ScoreEvidence,
    pub time: ScoreEvidence,
    pub test: ScoreEvidence,
}

/// Evidence split for the summary section of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryEvidence {
    pub finished: ScoreEvidence,
    pub skipped: ScoreEvidence,
    pub solved: ScoreEvidence,
    pub paste_flags: ScoreEvidence,
    pub total_time: ScoreEvidence,
    pub ai_correction_total: ScoreEvidence,
    pub rating: ScoreEvidence,
}

fn slot_of(k: &EventKind) -> Option<usize> {
    match k {
        EventKind::QuestionStarted { index }
        | EventKind::ApproachSelected { index, .. }
        | EventKind::TokensLocked { index, .. }
        | EventKind::Continued { index }
        | EventKind::Redirected { index, .. }
        | EventKind::ApproachChanged { index }
        | EventKind::TestsRequested { index }
        | EventKind::ReQuestionRequested { index }
        | EventKind::ReQuestionResolved { index, .. }
        | EventKind::QuestionFinished { index }
        | EventKind::QuestionSkipped { index }
        | EventKind::TestRunCompleted { index, .. }
        | EventKind::PasteFlagged { index, .. } => Some(*index),
        EventKind::ChatBoundaryViolated { index } => *index,
        _ => None,
    }
}
fn refs_where(s: &Session, slot: usize, p: impl Fn(&EventKind) -> bool) -> Vec<EventRef> {
    s.events()
        .iter()
        .filter(|e| slot_of(&e.kind) == Some(slot) && p(&e.kind))
        .map(|e| EventRef { seq: e.seq })
        .collect()
}
fn refs_all(s: &Session, p: impl Fn(&EventKind) -> bool) -> Vec<EventRef> {
    s.events()
        .iter()
        .filter(|e| p(&e.kind))
        .map(|e| EventRef { seq: e.seq })
        .collect()
}
fn find_ev(s: &Session, slot: usize, p: impl Fn(&EventKind) -> bool) -> Option<&SessionEvent> {
    s.events()
        .iter()
        .find(|e| slot_of(&e.kind) == Some(slot) && p(&e.kind))
}
fn is_close(k: &EventKind) -> bool {
    matches!(
        k,
        EventKind::QuestionFinished { .. } | EventKind::QuestionSkipped { .. }
    )
}
fn sorted(mut r: Vec<EventRef>) -> Vec<EventRef> {
    r.sort_by_key(|x| x.seq);
    r.dedup();
    r
}
fn ev(p: Provenance, refs: Vec<EventRef>, note: Option<&'static str>) -> ScoreEvidence {
    ScoreEvidence {
        provenance: p,
        refs,
        note,
    }
}
fn backed(refs: Vec<EventRef>) -> ScoreEvidence {
    if refs.is_empty() {
        ev(
            Provenance::NoEventEvidence,
            refs,
            Some("not recorded as an event"),
        )
    } else {
        ev(Provenance::EventBacked, refs, None)
    }
}
fn partial(refs: Vec<EventRef>, note: &'static str) -> ScoreEvidence {
    if refs.is_empty() {
        ev(Provenance::NoEventEvidence, refs, Some(note))
    } else {
        ev(Provenance::Partial, refs, Some(note))
    }
}
fn none(note: &'static str) -> ScoreEvidence {
    ev(Provenance::NoEventEvidence, Vec::new(), Some(note))
}
fn parse_after(d: &str, label: &str) -> Option<usize> {
    let parts: Vec<&str> = d.split_whitespace().collect();
    parts
        .windows(2)
        .find(|w| w[0] == label)
        .and_then(|w| w[1].parse().ok())
}
fn tokens_refs(s: &Session, slot: usize) -> Vec<EventRef> {
    let mut refs = Vec::new();
    let locks = refs_where(s, slot, |k| matches!(k, EventKind::TokensLocked { .. }));
    if let Some(r) = locks.into_iter().last() {
        refs.push(r);
    }
    if let Some(e) = find_ev(s, slot, is_close) {
        refs.push(EventRef { seq: e.seq });
    }
    sorted(refs)
}

/// Returns evidence for one final-breakdown score line.
pub fn score_line_evidence(session: &Session, line: &ScoreLine) -> ScoreEvidence {
    let slot = line.slot_index;
    match line.component {
        ScoreComponent::Approach => {
            let refs = refs_where(session, slot, |k| {
                matches!(k, EventKind::ApproachSelected { .. })
            });
            partial(refs, "approach quality is not recorded in events")
        }
        ScoreComponent::Tokens => partial(
            tokens_refs(session, slot),
            "actual token total is not recorded as an event",
        ),
        ScoreComponent::Intervention => {
            let refs = refs_where(session, slot, |k| {
                matches!(
                    k,
                    EventKind::Redirected { .. } | EventKind::ApproachChanged { .. }
                )
            });
            let implied = match (
                parse_after(&line.detail, "redirects"),
                parse_after(&line.detail, "changes"),
            ) {
                (Some(r), Some(c)) => Some(r + c),
                _ => parse_after(&line.detail, "interventions"),
            };
            match implied {
                Some(n) if n == refs.len() && !refs.is_empty() => backed(refs),
                _ => partial(
                    refs,
                    "intervention count in events does not match the score line",
                ),
            }
        }
        ScoreComponent::UnnecessaryChange => {
            let refs = refs_where(session, slot, |k| {
                matches!(k, EventKind::ApproachChanged { .. })
            });
            partial(refs, "abandoned approach quality is not recorded in events")
        }
        ScoreComponent::Paste => {
            let refs = refs_where(session, slot, |k| {
                matches!(k, EventKind::PasteFlagged { .. })
            });
            let expected = -0.30 * refs.len() as f64;
            if !refs.is_empty() && (line.delta - expected).abs() < 1e-9 {
                backed(refs)
            } else {
                partial(
                    refs,
                    "paste flag count in events does not match the score line",
                )
            }
        }
    }
}
fn pq_time(s: &Session, q: &PerQuestionFinal) -> ScoreEvidence {
    let slot = q.slot_index;
    let started = find_ev(s, slot, |k| matches!(k, EventKind::QuestionStarted { .. }));
    let closed = find_ev(s, slot, is_close);
    match (started, closed, q.time_ms) {
        (Some(a), Some(c), Some(ms)) => {
            let refs = sorted(vec![EventRef { seq: a.seq }, EventRef { seq: c.seq }]);
            if c.at_ms.saturating_sub(a.at_ms) == ms {
                backed(refs)
            } else {
                partial(
                    refs,
                    "question time in events does not match the recorded time",
                )
            }
        }
        (Some(a), Some(c), None) => partial(
            sorted(vec![EventRef { seq: a.seq }, EventRef { seq: c.seq }]),
            "question time is only partially recorded in events",
        ),
        (Some(a), None, _) => partial(
            vec![EventRef { seq: a.seq }],
            "question time is only partially recorded in events",
        ),
        (None, Some(c), _) => partial(
            vec![EventRef { seq: c.seq }],
            "question time is only partially recorded in events",
        ),
        (None, None, _) => none("question time is not recorded in events"),
    }
}
fn pq_test(s: &Session, q: &PerQuestionFinal) -> ScoreEvidence {
    let slot = q.slot_index;
    let all = refs_where(s, slot, |k| matches!(k, EventKind::TestRunCompleted { .. }));
    match q.latest_test {
        Some(t) => {
            let matching = refs_where(
                s,
                slot,
                |k| matches!(k, EventKind::TestRunCompleted { passed, failed, .. } if *passed == t.passed && *failed == t.failed),
            );
            if let Some(r) = matching.into_iter().last() {
                backed(vec![r])
            } else if !all.is_empty() {
                partial(
                    all,
                    "test summary does not match the latest recorded test event",
                )
            } else {
                none("test result is not recorded as an event")
            }
        }
        None if all.is_empty() => none("test result is not recorded as an event"),
        None => partial(
            all,
            "test summary does not match the latest recorded test event",
        ),
    }
}

/// Returns evidence for the grade, tokens, time and tests of one question.
pub fn per_question_evidence(session: &Session, q: &PerQuestionFinal) -> PerQuestionEvidence {
    PerQuestionEvidence {
        grade: none("AI grading result; not recorded as an event"),
        tokens: partial(
            tokens_refs(session, q.slot_index),
            "actual token total is not recorded as an event",
        ),
        time: pq_time(session, q),
        test: pq_test(session, q),
    }
}
fn count_ev(r: Vec<EventRef>, w: usize, bad: &'static str, empty: &'static str) -> ScoreEvidence {
    if r.len() == w && !r.is_empty() {
        backed(r)
    } else if r.len() == w {
        none(empty)
    } else {
        partial(r, bad)
    }
}

/// Returns evidence for every summary value in the final breakdown.
pub fn summary_evidence(session: &Session, b: &FinalBreakdown) -> SummaryEvidence {
    let fin = refs_all(session, |k| matches!(k, EventKind::QuestionFinished { .. }));
    let skip = refs_all(session, |k| matches!(k, EventKind::QuestionSkipped { .. }));
    let paste = refs_all(session, |k| matches!(k, EventKind::PasteFlagged { .. }));
    let mut total_refs: Vec<EventRef> = Vec::new();
    let mut recomputed: u64 = 0;
    for slot in session.slots() {
        let a = find_ev(session, slot.index, |k| {
            matches!(k, EventKind::QuestionStarted { .. })
        });
        let c = find_ev(session, slot.index, is_close);
        if let (Some(a), Some(c)) = (a, c) {
            recomputed = recomputed.saturating_add(c.at_ms.saturating_sub(a.at_ms));
            total_refs.push(EventRef { seq: a.seq });
            total_refs.push(EventRef { seq: c.seq });
        }
    }
    let total_refs = sorted(total_refs);
    let total_time = if recomputed == b.total_time_ms && !total_refs.is_empty() {
        backed(total_refs)
    } else if total_refs.is_empty() {
        none("total time is not recorded in events")
    } else {
        partial(
            total_refs,
            "total time in events does not match the breakdown",
        )
    };
    let mut rating_refs: Vec<EventRef> = Vec::new();
    for line in &b.lines {
        rating_refs.extend(score_line_evidence(session, line).refs);
    }
    SummaryEvidence {
        finished: count_ev(
            fin,
            b.finished_count,
            "finished count in events does not match the breakdown",
            "no finished questions recorded in events",
        ),
        skipped: count_ev(
            skip,
            b.skipped_count,
            "skipped count in events does not match the breakdown",
            "no skipped questions recorded in events",
        ),
        solved: none("solved count is derived from AI grades; not recorded as an event"),
        paste_flags: count_ev(
            paste,
            b.paste_flag_count as usize,
            "paste flag count in events does not match the breakdown",
            "no paste flags recorded in events",
        ),
        total_time,
        ai_correction_total: none("AI correction total is not recorded as an event"),
        rating: ev(
            Provenance::Partial,
            sorted(rating_refs),
            Some("rating is derived from score lines; see per-line evidence"),
        ),
    }
}

/// Renders evidence as bundle-relative references or an honest absence.
pub fn render_evidence(e: &ScoreEvidence) -> String {
    let refs = e
        .refs
        .iter()
        .map(|r| format!("events.jsonl#seq={} (line {})", r.seq, r.line_no()))
        .collect::<Vec<_>>()
        .join(", ");
    let note = e.note.unwrap_or("not recorded as an event");
    if refs.is_empty() {
        return match e.provenance {
            Provenance::Partial => format!("Partial: {note}"),
            _ => format!("NoEventEvidence: {note}"),
        };
    }
    match e.note {
        Some(n) => format!("{:?} {refs} — {n}", e.provenance),
        None => format!("{:?} {refs}", e.provenance),
    }
}
