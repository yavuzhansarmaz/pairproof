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
    /// Left-pane lines: question, statement, approaches, and status.
    pub fn question_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match (&self.question_index, &self.question_title) {
            (Some(i), Some(t)) => lines.push(format!("Q{}: {}", i + 1, t)),
            _ => lines.push("no active question.".to_string()),
        }
        if self.options.is_empty() {
            lines.push("approaches are not available yet.".to_string());
        } else {
            lines.push(format!(
                "approaches: 1-{} choose, {} other (your own)",
                self.options.len(),
                self.options.len() + 1
            ));
            let selected_preset = match &self.selected {
                Some(SnapshotSelection::Preset(n)) => {
                    Some(*n as usize).filter(|n| *n < self.options.len())
                }
                _ => None,
            };
            for (i, o) in self.options.iter().enumerate() {
                let plain = format!("{}: {} - {}", i + 1, o.title, o.summary);
                if selected_preset == Some(i) {
                    lines.push(format!("> {plain}"));
                } else {
                    lines.push(plain);
                }
            }
            let other_plain = format!(
                "{}: other - describe your own approach",
                self.options.len() + 1
            );
            let other_selected = matches!(self.selected, Some(SnapshotSelection::Other(_)));
            if other_selected {
                lines.push(format!("> {other_plain}"));
            } else {
                lines.push(other_plain);
            }
        }
        match &self.statement {
            Some(s) => lines.extend(s.lines().map(str::to_string)),
            None => lines.push("statement is not available yet.".to_string()),
        }
        lines.push(format!(
            "selected approach: {}",
            match &self.selected {
                Some(SnapshotSelection::Preset(n)) => format!("preset {n}"),
                Some(SnapshotSelection::Other(text)) => text.clone(),
                None => "none".to_string(),
            }
        ));
        match self.latest_test {
            Some(t) => lines.push(format!(
                "latest test: {} passed, {} failed",
                t.passed, t.failed
            )),
            None => lines.push("latest test: no test run yet".to_string()),
        }
        if !self.sync_phase.is_empty() {
            lines.push(self.sync_phase.clone());
        }
        lines.push(format!("phase: {}", self.phase));
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
            lines.push("session ended.".to_string());
        }
        lines.iter().map(|line| sanitize_display(line)).collect()
    }
    /// Right-pane lines: the Coach welcome first, then its replies.
    pub fn chat_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("> {COACH_WELCOME}")];
        lines.extend(self.chat_replies.iter().map(|reply| format!("> {reply}")));
        if let Some(partial) = &self.chat_partial {
            if !sanitize_display(partial).trim().is_empty() {
                lines.push(format!("> {partial}"));
            }
        }
        lines.iter().map(|line| sanitize_display(line)).collect()
    }
    /// Sanitized lines; a rating renders only from a carried breakdown.
    pub fn lines(&self) -> Vec<String> {
        if self.phase.is_empty() {
            return vec!["waiting for the expert.".to_string()];
        }
        let mut lines = self.question_lines();
        if !self.chat_replies.is_empty() || self.chat_partial.is_some() {
            lines.extend(self.chat_lines());
        }
        lines
    }
}
