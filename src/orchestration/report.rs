//! Deterministic Markdown session report. Only caller-supplied
//! permitted chat excerpts are quoted; grades, counts, and times are
//! rendered solely from the supplied breakdown and session state.

use crate::core::{QuestionStatus, Session};
use crate::scoring::{FinalBreakdown, GradeOutcome, PerQuestionFinal};

/// One caller-supplied chat excerpt that may be quoted in the report.
/// Callers pass only permitted excerpts; the default is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatExcerpt {
    pub slot_index: usize,
    pub text: String,
}

fn slot_title(session: &Session, slot_index: usize) -> String {
    session
        .slots()
        .iter()
        .find(|s| s.index == slot_index)
        .map(|s| s.question.title.clone())
        .unwrap_or_else(|| format!("slot {slot_index}"))
}

fn not_scored_slots(breakdown: &FinalBreakdown, grades: &[GradeOutcome]) -> Vec<usize> {
    let mut slots: Vec<usize> = breakdown.not_scored.clone();
    for outcome in grades {
        if let GradeOutcome::NotScored { slot_index } = outcome {
            if !slots.contains(slot_index) {
                slots.push(*slot_index);
            }
        }
    }
    slots.sort_unstable();
    slots
}

fn per_question_line(session: &Session, q: &PerQuestionFinal) -> String {
    let title = slot_title(session, q.slot_index);
    let grade_part = match q.grade {
        Some(grade) => format!("grade {grade:.1}/10, correction {:+.3}", q.correction),
        None => "not scored, correction +0.000".to_string(),
    };
    let rationale_part = match &q.rationale {
        Some(r) => format!("rationale: {r}"),
        None => "rationale: not available".to_string(),
    };
    let tokens_part = match (q.expected_tokens, q.token_ratio) {
        (Some(expected), Some(ratio)) => {
            format!("tokens {}/{expected} (ratio {ratio:.3})", q.actual_tokens)
        }
        _ => format!(
            "tokens {} (no locked expectation recorded)",
            q.actual_tokens
        ),
    };
    let time_part = match q.time_ms {
        Some(ms) => format!("time {ms} ms"),
        None => "time not recorded".to_string(),
    };
    let test_part = match q.latest_test {
        Some(t) => format!("test summary: {} passed, {} failed", t.passed, t.failed),
        None => "test summary: no test summary recorded".to_string(),
    };
    format!(
        "- Q{} {title} ({:?}, {:?}) — {grade_part}; {tokens_part}; {time_part}; {test_part}; {rationale_part}",
        q.slot_index + 1,
        q.difficulty,
        q.status,
    )
}

fn suggestions(session: &Session, breakdown: &FinalBreakdown) -> Vec<String> {
    let mut out = Vec::new();
    let mut per: Vec<&PerQuestionFinal> = breakdown.per_question.iter().collect();
    per.sort_by_key(|q| q.slot_index);
    for q in per {
        let title = slot_title(session, q.slot_index);
        if q.status == QuestionStatus::Finished && q.grade.is_none() {
            out.push(format!(
                "- Follow up on {title}: this question was not scored, so ask the candidate to explain their approach and verification steps."
            ));
            continue;
        }
        if let Some(grade) = q.grade {
            if grade < 5.0 {
                out.push(format!(
                    "- Follow up on {title}: ask the candidate to walk through their approach and how they would improve the solution."
                ));
            }
        }
        if let Some(ratio) = q.token_ratio {
            if ratio > 1.2 {
                out.push(format!(
                    "- Follow up on {title}: discuss how the candidate chose and steered the approach, since recorded token use was above the locked expectation."
                ));
            }
        }
    }
    out
}

/// Generates the Markdown report in deterministic slot order.
pub fn generate_report(
    session: &Session,
    breakdown: &FinalBreakdown,
    grades: &[GradeOutcome],
    chat_excerpts: &[ChatExcerpt],
) -> String {
    let not_scored = not_scored_slots(breakdown, grades);
    let mut per: Vec<&PerQuestionFinal> = breakdown.per_question.iter().collect();
    per.sort_by_key(|q| q.slot_index);
    let mut excerpts: Vec<&ChatExcerpt> = chat_excerpts.iter().collect();
    excerpts.sort_by_key(|e| e.slot_index);
    let mut out = String::new();
    out.push_str("# PairProof Session Report\n\n");
    out.push_str(&format!("Session: {}\n\n", session.id()));
    out.push_str(&format!("Status: {:?}\n\n", session.status()));
    out.push_str("## Final breakdown\n\n");
    out.push_str(&format!(
        "Final rating: {:.2} / 10 (unclamped {:.2})\n\n",
        breakdown.rating, breakdown.unclamped
    ));
    out.push_str(&format!(
        "Solved: {}; finished: {}; skipped: {}\n\n",
        breakdown.solved_count, breakdown.finished_count, breakdown.skipped_count
    ));
    out.push_str(&format!(
        "AI correction total: {:+.3}\n\n",
        breakdown.ai_correction_total
    ));
    out.push_str(&format!("Total time: {} ms\n\n", breakdown.total_time_ms));
    out.push_str(&format!("Paste flags: {}\n\n", breakdown.paste_flag_count));
    if breakdown.lines.is_empty() {
        out.push_str("- No rating movements recorded.\n\n");
    } else {
        let mut lines = breakdown.lines.clone();
        lines.sort_by_key(|l| (l.slot_index, l.component));
        for line in &lines {
            out.push_str(&format!(
                "- Q{} {:?} {:+.2} — {}\n",
                line.slot_index + 1,
                line.component,
                line.delta,
                line.detail
            ));
        }
        out.push('\n');
    }
    out.push_str("## Per-question grades\n\n");
    if per.is_empty() {
        out.push_str("- No questions recorded.\n\n");
    } else {
        for q in &per {
            out.push_str(&per_question_line(session, q));
            out.push('\n');
        }
        out.push('\n');
    }
    for excerpt in &excerpts {
        out.push_str(&format!(
            "- Chat excerpt for Q{} ({}): {}\n",
            excerpt.slot_index + 1,
            slot_title(session, excerpt.slot_index),
            excerpt.text
        ));
    }
    if !excerpts.is_empty() {
        out.push('\n');
    }
    out.push_str("## Follow-up suggestions\n\n");
    let items = suggestions(session, breakdown);
    if items.is_empty() {
        out.push_str("- No follow-up suggestions from the recorded evidence.\n\n");
    } else {
        for item in &items {
            out.push_str(item);
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str("## Notes\n\n");
    out.push_str(
        "This report supports a human interviewer; it does not make a hiring decision.\n\n",
    );
    if not_scored.is_empty() {
        out.push_str("All finished questions were scored.\n\n");
    } else {
        let list: Vec<String> = not_scored.iter().map(|i| format!("Q{}", i + 1)).collect();
        out.push_str(&format!("Not scored: {}.\n\n", list.join(", ")));
    }
    out.push_str("Skipped questions are not scored.\n\n");
    if excerpts.is_empty() {
        out.push_str("No chat excerpts were included.\n");
    } else {
        out.push_str(
            "Only the permitted chat excerpts supplied for this report are quoted above.\n",
        );
    }
    out
}
