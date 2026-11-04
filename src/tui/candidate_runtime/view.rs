//! Candidate snapshot view: snapshot-only render data.

use super::*;

/// First message shown on the Coach side of the solving screen.
pub const COACH_WELCOME: &str = "Talking with me won't affect your score. \
    You can ask me anything about the question on the left, in any language you like.";

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateView {
    pub phase: String,
    pub sync_phase: String,
    pub question_title: Option<String>,
    pub question_index: Option<usize>,
    pub statement: Option<String>,
    pub options: Vec<SnapshotOption>,
    pub nav: Vec<SnapshotNav>,
    pub selected: Option<SnapshotSelection>,
    pub latest_test: Option<SnapshotTest>,
    pub chat_replies: Vec<String>,
    pub chat_partial: Option<String>,
    pub final_breakdown: Option<SnapshotFinal>,
    pub recovery: Option<SnapshotRecovery>,
}
impl CandidateView {
    pub fn from_snapshot(snapshot: Option<&CandidateSnapshot>) -> Self {
        let Some(s) = snapshot else {
            return Self {
                phase: String::new(),
                sync_phase: String::new(),
                question_title: None,
                question_index: None,
                statement: None,
                options: Vec::new(),
                nav: Vec::new(),
                selected: None,
                latest_test: None,
                chat_replies: Vec::new(),
                chat_partial: None,
                final_breakdown: None,
                recovery: None,
            };
        };
        Self {
            phase: s.phase.clone(),
            sync_phase: s.sync_phase.clone(),
            question_title: s.current.as_ref().map(|q| q.title.clone()),
            question_index: s.current.as_ref().map(|q| q.index),
            statement: s
                .current
                .as_ref()
                .and_then(|q| q.statement.as_ref())
                .map(|text| crate::tui::shared::html_to_text(text)),
            options: s.options.clone(),
            nav: s.nav.clone(),
            selected: s.selected.clone(),
            latest_test: s.latest_test,
            chat_replies: s.chat_replies.clone(),
            chat_partial: s.chat_partial.clone(),
            final_breakdown: s.final_breakdown.clone(),
            recovery: s.recovery.clone(),
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase.as_str(),
            "Completed" | "EndedByExpert" | "TechnicalFailure"
        )
    }
    /// True while the first statement is still on its way: no
    /// statement yet, and nothing final or failed to show instead.
    /// The TTY loop shows the shared loading screen in this state.
    pub fn waiting_for_statement(&self) -> bool {
        self.statement.is_none()
            && !self.is_terminal()
            && self.final_breakdown.is_none()
            && !self.recovery.as_ref().is_some_and(|r| r.technical_failure)
    }
    /// Status-box lines, shown in the fixed box above the Coach:
    /// the submitted approach, latest test, phase, and sync state.
    pub fn status_lines(&self) -> Vec<String> {
        let approach = match &self.selected {
            Some(SnapshotSelection::Preset(n)) => format!("preset {}", n + 1),
            Some(SnapshotSelection::Other(text)) => {
                if text.is_empty() {
                    "other (blank)".to_string()
                } else {
                    format!("other: {text}")
                }
            }
            None => "none".to_string(),
        };
        let test = match self.latest_test {
            Some(t) => format!("{} passed, {} failed", t.passed, t.failed),
            None => match self.latest_run().and_then(|run| run.counted_tests()) {
                Some((passed, failed)) => format!("{passed} passed, {failed} failed"),
                None => "no test run yet".to_string(),
            },
        };
        let sync = if self.sync_phase.is_empty() {
            "sync: unknown".to_string()
        } else {
            self.sync_phase.clone()
        };
        [
            format!("selected approach: {approach}"),
            format!("latest test: {test}"),
            format!("phase: {}", crate::tui::monitor::human_status(&self.phase)),
            sync,
        ]
        .iter()
        .map(|line| sanitize_display(line))
        .collect()
    }

    /// Latest coding or execution summary recorded for this question view.
    pub fn latest_run(&self) -> Option<crate::orchestration::control::SnapshotRun> {
        crate::orchestration::control_snapshot::view_run_for(
            self.question_index?,
            self.question_title.as_deref()?,
        )
    }

    /// Current code revision for this question (0 before generation).
    pub fn code_revision(&self) -> u64 {
        self.latest_run().map_or(0, |r| r.code_revision)
    }

    /// Current execution sequence for this question.
    pub fn run_seq(&self) -> u64 {
        self.latest_run().map_or(0, |r| r.seq)
    }

    /// Status lines, with a local "writing the code" line while a
    /// submitted approach has not appeared in the snapshot yet.
    pub fn status_lines_with_pending(
        &self,
        pending: Option<&crate::tui::candidate_stage::PendingApproach>,
    ) -> Vec<String> {
        let mut lines = self.status_lines();
        let Some(pending) = pending else {
            return lines;
        };
        if self.question_index != Some(pending.index)
            || self.selected.as_ref() == Some(&pending.selection)
        {
            return lines;
        }
        if let Some(first) = lines.first_mut() {
            *first = sanitize_display(&format!(
                "selected approach: {}, writing the code",
                pending.label()
            ));
        }
        lines
    }

    /// Status lines incorporating the active question flow phase.
    pub fn status_lines_with_flow(
        &self,
        pending: Option<&crate::tui::candidate_stage::PendingApproach>,
        flow: Option<&crate::tui::candidate_stage::QuestionFlowState>,
    ) -> Vec<String> {
        let mut lines = self.status_lines_with_pending(pending);
        if let Some(label) = flow.and_then(|f| f.status_phase_label()) {
            if let Some(phase_line) = lines.get_mut(2) {
                *phase_line = sanitize_display(&format!("phase: {label}"));
            }
        }
        lines
    }

    /// Left-pane lines without staging (the server selection is
    /// shown marked, with the cursor on it).
    pub fn question_lines(&self) -> Vec<String> {
        self.question_lines_staged(None)
    }

    /// Left-pane lines, simple top-to-bottom: the question first,
    /// then its statement, then the approach list the candidate
    /// marks with Space and submits with Enter. `stage` carries the
    /// local cursor/mark; without it the server selection is shown.
    pub fn question_lines_staged(
        &self,
        stage: Option<&crate::tui::candidate_stage::QuestionStage>,
    ) -> Vec<String> {
        use crate::tui::candidate_stage::StagedChoice;
        let mut lines = Vec::new();
        let total = self.nav.len();
        match (&self.question_index, &self.question_title) {
            (Some(i), Some(title)) if total > 0 => {
                lines.push(format!("Q{} of {}: {}", i + 1, total, title))
            }
            (Some(i), Some(title)) => lines.push(format!("Q{}: {}", i + 1, title)),
            _ => lines.push("no active question.".to_string()),
        }
        if let Some(entry) = self
            .question_index
            .and_then(|i| self.nav.iter().find(|n| n.index == i))
        {
            if entry.state != "Active" {
                lines.push(format!("status: {}", entry.state.to_ascii_lowercase()));
            }
        }
        lines.push(String::new());
        match &self.statement {
            Some(s) => lines.extend(s.lines().map(str::to_string)),
            None => lines.push("statement is not available yet.".to_string()),
        }
        lines.push(String::new());
        lines.push("Approaches".to_string());
        if self.options.is_empty() {
            lines.push("loading".to_string());
        } else {
            let (cursor, marked) = match stage {
                Some(stage) => (stage.cursor, stage.choice.clone()),
                None => match &self.selected {
                    Some(SnapshotSelection::Preset(n)) => {
                        (*n as usize, Some(StagedChoice::Preset(*n)))
                    }
                    Some(SnapshotSelection::Other(_)) => {
                        (self.options.len(), Some(StagedChoice::Other))
                    }
                    None => (0, None),
                },
            };
            for (i, o) in self.options.iter().enumerate() {
                let mark = if marked == Some(StagedChoice::Preset(i as u8)) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let pointer = if cursor == i { ">" } else { " " };
                lines.push(format!(
                    "{pointer} {mark} {}: {} - {}",
                    i + 1,
                    o.title,
                    o.summary
                ));
            }
            let mark = if marked == Some(StagedChoice::Other) {
                "[x]"
            } else {
                "[ ]"
            };
            let pointer = if cursor == self.options.len() {
                ">"
            } else {
                " "
            };
            lines.push(format!(
                "{pointer} {mark} {}: other - describe your own approach",
                self.options.len() + 1
            ));
        }
        if let Some(r) = &self.recovery {
            if r.active {
                lines.push("technical problem - service temporarily unavailable.".to_string());
                lines.push(format!(
                    "retrying... about {} seconds remaining.",
                    r.remaining_ms.saturating_add(999) / 1000
                ));
                lines.push("your progress is preserved.".to_string());
            }
        }
        if let Some(f) = &self.final_breakdown {
            lines.push("Final breakdown".to_string());
            lines.push(format!("final rating: {:.2}", f.rating));
            for q in &f.per_question {
                let grade = q
                    .grade
                    .map_or("not scored".to_string(), |g| format!("{g:.1}"));
                let mut line = format!("Q{}: grade {}", q.slot_index + 1, grade);
                if let Some(r) = &q.rationale {
                    line.push_str(&format!(": {r}"));
                }
                lines.push(line);
            }
        } else if self.phase == "TechnicalFailure"
            || self.recovery.as_ref().is_some_and(|r| r.technical_failure)
        {
            lines.push("the session ended as a technical failure.".to_string());
            lines.push("no evaluation was produced; the interview can be rescheduled.".to_string());
        } else if self.is_terminal() {
            lines.push("the expert's client closed.".to_string());
            lines.push("the session is over.".to_string());
        }
        lines.iter().map(|line| sanitize_display(line)).collect()
    }
    /// Right-pane lines: the Coach welcome first, then its replies.
    pub fn chat_lines(&self) -> Vec<String> {
        self.chat_transcript(&[])
    }

    /// Coach pane as a conversation. `>` lines are the coach, drawn
    /// on the left, one sentence per line, with a blank line between
    /// paragraphs. `<` lines are the candidate: the text is drawn on
    /// the right and the `<` sits at the right edge.
    /// A blank line separates turns. `sent` is what the candidate
    /// already submitted, in order, paired with each coach reply.
    pub fn chat_transcript(&self, sent: &[String]) -> Vec<String> {
        let mut lines = Vec::new();
        push_turn(&mut lines, ">", COACH_WELCOME);
        let turns = sent.len().max(self.chat_replies.len());
        for i in 0..turns {
            if let Some(text) = sent.get(i) {
                push_turn(&mut lines, "<", text);
            }
            if let Some(reply) = self.chat_replies.get(i) {
                push_turn(&mut lines, ">", reply);
            }
        }
        if let Some(partial) = &self.chat_partial {
            push_turn(&mut lines, ">", partial);
        }
        lines
    }
    /// Centred notice when the expert quits, or the client disappears
    /// before a final breakdown arrives. The candidate leaves the
    /// work panes; Enter closes.
    pub fn expert_closed_lines() -> Vec<String> {
        vec![
            "Expert closed".to_string(),
            String::new(),
            "the expert's client closed.".to_string(),
            "the session is over.".to_string(),
            String::new(),
            "enter to close".to_string(),
        ]
    }

    /// Centred final screen shown when the expert ends the session:
    /// only what the candidate may see — the final rating, how many
    /// were solved/finished/skipped, and the grade per question.
    /// No question text, no approach internals, no live lines.
    /// A terminal snapshot with no breakdown yet means the expert
    /// closed before grading finished.
    pub fn final_screen_lines(&self) -> Vec<String> {
        if self.final_breakdown.is_none() && self.phase != "TechnicalFailure" {
            return Self::expert_closed_lines();
        }
        let mut lines = vec!["Final breakdown".to_string()];
        if let Some(f) = &self.final_breakdown {
            let solved = f
                .per_question
                .iter()
                .filter(|q| q.grade.is_some_and(|g| g >= 5.0))
                .count();
            let finished = self.nav.iter().filter(|n| n.state == "Finished").count();
            let skipped = self.nav.iter().filter(|n| n.state == "Skipped").count();
            let (finished, skipped) = if self.nav.is_empty() {
                (f.per_question.len(), 0)
            } else {
                (finished, skipped)
            };
            lines.push(String::new());
            lines.push(format!("final rating: {:.2} / 10", f.rating));
            lines.push(String::new());
            lines.push(format!("solved: {solved}"));
            lines.push(format!("finished: {finished}"));
            lines.push(format!("skipped: {skipped}"));
            lines.push(String::new());
            lines.push("Questions".to_string());
            for q in &f.per_question {
                let title = if q.title.is_empty() {
                    self.nav
                        .iter()
                        .find(|n| n.index == q.slot_index)
                        .map(|n| n.title.clone())
                        .unwrap_or_default()
                } else {
                    q.title.clone()
                };
                let label = match q.grade {
                    Some(g) if g >= 5.0 => format!("{g:.1} - solved"),
                    Some(g) => format!("{g:.1}"),
                    None if q.state == "Skipped" => "skipped".to_string(),
                    None => "not scored".to_string(),
                };
                lines.push(String::new());
                lines.push(format!("Q{}  {title}", q.slot_index + 1));
                lines.push(format!("    {label}"));
                if let Some(r) = &q.rationale {
                    lines.push(format!("    {r}"));
                }
            }
        } else {
            lines.push("the session ended as a technical failure.".to_string());
            lines.push("no evaluation was produced; the interview can be rescheduled.".to_string());
        }
        lines.push(String::new());
        lines.push("enter to exit".to_string());
        lines.iter().map(|line| sanitize_display(line)).collect()
    }

    /// Sanitized lines; a rating renders only from a carried breakdown.
    pub fn lines(&self) -> Vec<String> {
        if self.phase.is_empty() {
            return vec!["waiting for the expert.".to_string()];
        }
        let mut lines = self.question_lines();
        lines.extend(self.status_lines());
        if !self.chat_replies.is_empty() || self.chat_partial.is_some() {
            lines.extend(self.chat_lines());
        }
        lines
    }
}

fn push_turn(lines: &mut Vec<String>, marker: &str, text: &str) {
    let clean = sanitize_display(text);
    if clean.trim().is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    if marker == ">" {
        // A coach reply is one string, often one long paragraph.
        // Break it into sentences and keep the model's own paragraph
        // gaps, so the pane is not a single wrapped block.
        lines.extend(coach_display_lines(&clean));
    } else {
        lines.push(format!("{marker} {clean}"));
    }
}

/// Coach reply as `>` lines: one sentence each, blank line between
/// paragraphs the model already separated.
fn coach_display_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    let mut out = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut started = false;
    for line in normalized.split('\n') {
        if line.trim().is_empty() {
            if !paragraph.is_empty() {
                push_coach_paragraph(&mut out, &paragraph, started);
                started = true;
                paragraph.clear();
            }
        } else {
            paragraph.push(line);
        }
    }
    if !paragraph.is_empty() {
        push_coach_paragraph(&mut out, &paragraph, started);
    }
    out
}

fn push_coach_paragraph(out: &mut Vec<String>, lines: &[&str], gap_before: bool) {
    let mut sentences = Vec::new();
    for line in lines {
        sentences.extend(split_sentences(line));
    }
    if sentences.is_empty() {
        return;
    }
    if gap_before {
        out.push(String::new());
    }
    for sentence in sentences {
        out.push(format!("> {sentence}"));
    }
}

/// Splits one line on sentence endings. A decimal (`3.14`) and a
/// lowercase continuation (`e.g. the`) stay intact. A partial reply
/// with no ending yet stays one piece.
fn split_sentences(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '.' | '?' | '!') {
            let decimal = chars[i] == '.'
                && i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit());
            if !decimal {
                let mut end = i + 1;
                while end < chars.len() && matches!(chars[end], '"' | '\'' | '”' | '’' | ')' | ']')
                {
                    end += 1;
                }
                let boundary = end >= chars.len() || chars[end].is_whitespace();
                if boundary {
                    let next = chars[end..].iter().find(|c| !c.is_whitespace()).copied();
                    // A new sentence may start with a capital or a number
                    // (`3.14` itself is not a boundary: both sides are digits).
                    if next.is_none_or(|c| c.is_uppercase() || c.is_ascii_digit()) {
                        let sentence: String = chars[start..end].iter().collect();
                        let sentence = sentence.trim();
                        if !sentence.is_empty() {
                            parts.push(sentence.to_string());
                        }
                        start = end;
                    }
                }
            }
        }
        i += 1;
    }
    let tail: String = chars[start..].iter().collect();
    let tail = tail.trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}
