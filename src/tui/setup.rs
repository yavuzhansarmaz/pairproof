//! Setup/context controller beneath rendering; no terminal I/O here.

use std::collections::HashSet;
use thiserror::Error;

use crate::core::types::Difficulty;
use crate::core::{Language, QuestionRef};
use crate::ingest::{
    bound_text, prepare_documents, propose_context, propose_from_extraction, ExpertDocuments,
    IngestError, LanguageProposal, ProposedContext,
};
use crate::leetcode::selection::{auto_pick_easy, SelectionError};
use crate::leetcode::LeetCodeClient;
use crate::llm::extraction::ExtractionResult;
use crate::llm::MetaClient;
use crate::tui::shared::sanitize_display;

/// Errors from the setup controller; transitions fail as values.
#[derive(Debug, Error)]
pub enum SetupError {
    /// Proposal was requested before the Meta disclosure was acknowledged.
    #[error("acknowledge the Meta API disclosure before requesting a proposal")]
    DisclosureNotAcknowledged,
    /// Document preparation or proposal extraction failed.
    #[error(transparent)]
    Ingest(#[from] IngestError),
    /// Auto-easy selection did not contain exactly five questions.
    #[error("expected {expected} easy questions, got {got}")]
    InvalidQuestionCount { expected: usize, got: usize },
    /// An auto-easy question was not easy difficulty.
    #[error("question '{slug}' is not easy")]
    WrongDifficulty { slug: String },
    /// Two auto-easy questions shared a slug.
    #[error("duplicate question slug: {0}")]
    DuplicateSlug(String),
    /// An auto-easy question had blank identifying fields.
    #[error("invalid question: {0}")]
    InvalidQuestion(String),
    /// An operation needed a proposal that has not been applied.
    #[error("no proposal has been applied yet")]
    NoProposal,
    /// Live auto-easy selection against LeetCode failed.
    #[error("auto easy selection failed: {0}")]
    Selection(#[from] SelectionError),
}

/// Expert paste draft plus the disclosure acknowledgement flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupDraft {
    pub jd: String,
    pub additional: String,
    pub disclosure_acknowledged: bool,
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::Python => "Python",
        Language::JavaScript => "JavaScript",
        Language::Sql => "SQL",
    }
}

/// Deterministic setup state machine beneath the rendering layer.
#[derive(Debug, Clone, Default)]
pub struct SetupController {
    draft: SetupDraft,
    proposal: Option<ProposedContext>,
    selected_language: Option<Language>,
    easy: Vec<QuestionRef>,
}

/// Backwards-readable alias for the setup controller state.
pub type SetupState = SetupController;

impl SetupController {
    /// Creates an empty controller with no disclosure acknowledgement.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns disclosure text shown before any text is sent to Meta.
    pub fn disclosure_message() -> &'static str {
        "your job description and additional notes (each bounded to 50,000 \
         characters) will be sent to the Meta API to propose interview \
         topics, level, and language. acknowledge before continuing."
    }

    /// Returns the current draft.
    pub fn draft(&self) -> &SetupDraft {
        &self.draft
    }

    /// Replaces the job-description paste.
    pub fn set_jd(&mut self, text: &str) {
        self.draft.jd = text.to_string();
    }

    /// Replaces the optional additional-notes paste.
    pub fn set_additional(&mut self, text: &str) {
        self.draft.additional = text.to_string();
    }

    /// Records that the expert acknowledged the Meta API disclosure.
    pub fn acknowledge_disclosure(&mut self) {
        self.draft.disclosure_acknowledged = true;
    }

    /// Prepares bounded documents; blank JD fails without any API call.
    pub fn validate_for_proposal(&self) -> Result<ExpertDocuments, SetupError> {
        Ok(prepare_documents(&self.draft.jd, &self.draft.additional)?)
    }

    /// Returns (jd_truncated, additional_truncated) for the current draft.
    pub fn truncation_flags(&self) -> (bool, bool) {
        match self.validate_for_proposal() {
            Ok(docs) => (docs.jd.truncated, docs.additional.truncated),
            Err(_) => (
                bound_text(self.draft.jd.trim()).truncated,
                bound_text(self.draft.additional.trim()).truncated,
            ),
        }
    }

    /// Returns true only when disclosure is acknowledged and JD is non-blank.
    pub fn can_request_proposal(&self) -> bool {
        self.draft.disclosure_acknowledged && !self.draft.jd.trim().is_empty()
    }

    /// Requests a proposal via the Meta client after the disclosure gate.
    pub async fn request_proposal(
        &mut self,
        client: &MetaClient,
    ) -> Result<ProposedContext, SetupError> {
        if !self.draft.disclosure_acknowledged {
            return Err(SetupError::DisclosureNotAcknowledged);
        }
        let docs = self.validate_for_proposal()?;
        let proposed = propose_context(client, &docs).await?;
        self.apply_proposal(proposed.clone());
        Ok(proposed)
    }

    /// Applies an injected extraction result without any network call.
    pub fn apply_extraction(
        &mut self,
        result: ExtractionResult,
    ) -> Result<ProposedContext, SetupError> {
        let docs = self.validate_for_proposal()?;
        let proposed = propose_from_extraction(&docs, result)?;
        self.apply_proposal(proposed.clone());
        Ok(proposed)
    }

    /// Stores a proposal and preselects its language when supported.
    pub fn apply_proposal(&mut self, proposed: ProposedContext) {
        self.selected_language = proposed.language.proposed();
        self.proposal = Some(proposed);
    }

    /// Returns the stored proposal, if one has been applied.
    pub fn proposal(&self) -> Option<&ProposedContext> {
        self.proposal.as_ref()
    }

    /// Confirms the session language chosen by the expert.
    pub fn confirm_language(&mut self, language: Language) {
        self.selected_language = Some(language);
    }

    /// Overrides the proposed session language.
    pub fn override_language(&mut self, language: Language) {
        self.selected_language = Some(language);
    }

    /// Returns the currently selected language, if any.
    pub fn selected_language(&self) -> Option<Language> {
        self.selected_language
    }

    /// Warns when extraction proposed an unsupported language.
    pub fn language_warning(&self) -> Option<String> {
        let proposal = self.proposal.as_ref()?;
        if let LanguageProposal::Unsupported(name) = &proposal.language {
            if self.selected_language.is_none() {
                return Some(format!(
                    "Unsupported language '{}': pick a supported one \
                     (Python, JavaScript, SQL)",
                    sanitize_display(name)
                ));
            }
        }
        None
    }

    /// Word-wraps one display line at 80 columns; continuation lines
    /// are indented two spaces so long reasons stay on screen.
    fn wrap_line(line: &str) -> Vec<String> {
        const WIDTH: usize = 80;
        let mut out = Vec::new();
        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.is_empty() {
                current = word.to_string();
            } else if current.chars().count() + 1 + word.chars().count() <= WIDTH {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current = format!("  {word}");
            }
        }
        if !current.is_empty() || out.is_empty() {
            out.push(current);
        }
        out
    }

    /// Builds sanitized display lines; never includes the raw pastes.
    pub fn proposal_lines(&self) -> Vec<String> {
        let Some(proposal) = &self.proposal else {
            return Vec::new();
        };
        let topics = proposal
            .topics
            .iter()
            .map(|topic| sanitize_display(topic))
            .collect::<Vec<_>>()
            .join(", ");
        let mut lines = vec![
            format!("Level: {}", sanitize_display(&proposal.level)),
            format!("Topics: {topics}"),
        ];
        match &proposal.language {
            LanguageProposal::Proposed(language) => {
                lines.push(format!("Language: {}", language_name(*language)));
            }
            LanguageProposal::Unsupported(name) => {
                lines.push(format!(
                    "Language: {} (unsupported)",
                    sanitize_display(name)
                ));
            }
        }
        if !proposal.language_reason.trim().is_empty() {
            lines.push(format!(
                "Language reason: {}",
                sanitize_display(&proposal.language_reason)
            ));
        }
        if let Some(selected) = self.selected_language {
            lines.push(format!("Selected language: {}", language_name(selected)));
        }
        if proposal.jd_truncated {
            lines.push("Job description was truncated to 50,000 characters.".to_string());
        }
        if proposal.additional_truncated {
            lines.push("Additional notes were truncated to 50,000 characters.".to_string());
        }
        lines
            .iter()
            .flat_map(|line| Self::wrap_line(line))
            .collect()
    }

    /// Validates and stores the auto-selected easy set without preview.
    pub fn accept_auto_easy(&mut self, questions: Vec<QuestionRef>) -> Result<(), SetupError> {
        if questions.len() != 5 {
            return Err(SetupError::InvalidQuestionCount {
                expected: 5,
                got: questions.len(),
            });
        }
        let mut seen: HashSet<&str> = HashSet::new();
        for question in &questions {
            if question.title_slug.trim().is_empty() || question.title.trim().is_empty() {
                return Err(SetupError::InvalidQuestion(question.title_slug.clone()));
            }
            if question.difficulty != Difficulty::Easy {
                return Err(SetupError::WrongDifficulty {
                    slug: question.title_slug.clone(),
                });
            }
            if !seen.insert(question.title_slug.as_str()) {
                return Err(SetupError::DuplicateSlug(question.title_slug.clone()));
            }
        }
        self.easy = questions;
        Ok(())
    }

    /// Auto-selects five easy questions for the proposal topics live.
    pub async fn auto_select_easy(
        &mut self,
        client: &LeetCodeClient,
        rng: &mut impl rand::Rng,
    ) -> Result<(), SetupError> {
        let proposal = self.proposal.as_ref().ok_or(SetupError::NoProposal)?;
        let topic_refs: Vec<&str> = proposal.topics.iter().map(String::as_str).collect();
        let picked = auto_pick_easy(client, &topic_refs, rng).await?;
        self.accept_auto_easy(picked)
    }

    /// Returns the accepted auto-easy questions for session creation.
    pub fn easy_questions(&self) -> &[QuestionRef] {
        &self.easy
    }

    /// Returns true once a validated auto-easy set has been accepted.
    pub fn has_auto_easy(&self) -> bool {
        self.easy.len() == 5
    }
}

/// Which Session Setup text box currently receives pastes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpertSetupFocus {
    #[default]
    JobDescription,
    AdditionalNotes,
}

/// Expert Setup paste collector for the two text boxes. The job
/// description is required; additional notes are optional. Held by
/// the TTY loop, never by `ExpertApp`.
#[derive(Debug, Default)]
pub struct ExpertSetupPastes {
    controller: SetupController,
    jd_received: bool,
    notes_received: bool,
    focus: ExpertSetupFocus,
    jd_scroll: usize,
    notes_scroll: usize,
}

fn validate_paste(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Paste rejected: blank paste ignored.".to_string());
    }
    if text.chars().count() > crate::ingest::MAX_TEXT_CHARS {
        return Err("Paste rejected: exceeds 50,000 characters.".to_string());
    }
    Ok(())
}

#[rustfmt::skip]
impl ExpertSetupPastes {
    pub fn new() -> Self { Self::default() }
    pub fn jd(&self) -> &str { &self.controller.draft().jd }
    pub fn notes(&self) -> &str { &self.controller.draft().additional }
    pub fn is_complete(&self) -> bool { self.jd_received && self.notes_received }
    /// True once a non-blank job description is in place; notes are
    /// optional, so this alone is enough to build a proposal.
    pub fn is_ready(&self) -> bool { self.jd_received && !self.jd().trim().is_empty() }
    pub fn focus(&self) -> ExpertSetupFocus { self.focus }
    pub fn jd_scroll(&self) -> usize { self.jd_scroll }
    pub fn notes_scroll(&self) -> usize { self.notes_scroll }
    /// Syncs the Preflight disclosure acknowledgement into the controller.
    pub fn acknowledge_disclosure(&mut self) { self.controller.acknowledge_disclosure(); }
    /// Requests the proposal through the controller (Meta API call).
    pub async fn request_proposal(&mut self, client: &MetaClient) -> Result<ProposedContext, SetupError> { self.controller.request_proposal(client).await }
    /// Builds a detached collector from raw texts, for requesting a
    /// proposal off the TTY thread while the on-screen one stays put.
    pub fn from_texts(jd: &str, notes: &str) -> Self {
        let mut detached = Self::new();
        detached.controller.set_jd(jd);
        detached.controller.set_additional(notes);
        detached.jd_received = !jd.trim().is_empty();
        detached.notes_received = !notes.trim().is_empty();
        detached
    }
    /// Stores a proposal obtained by a detached collector.
    pub fn apply_proposal(&mut self, proposed: ProposedContext) { self.controller.apply_proposal(proposed); }
    /// Returns the stored proposal, if one has been applied.
    pub fn proposal(&self) -> Option<&ProposedContext> { self.controller.proposal() }
    /// Sanitized proposal lines for the Proposal screen; never raw pastes.
    pub fn proposal_lines(&self) -> Vec<String> { self.controller.proposal_lines() }
    /// Returns the language selected from the proposal, if any.
    pub fn selected_language(&self) -> Option<Language> { self.controller.selected_language() }
    /// Pastes into the focused box, replacing its previous text so a
    /// wrong paste can be corrected by pasting again.
    pub fn apply_focused_paste(&mut self, text: &str) -> Result<(), String> {
        validate_paste(text)?;
        match self.focus {
            ExpertSetupFocus::JobDescription => { self.controller.set_jd(text); self.jd_received = true; self.jd_scroll = 0; }
            ExpertSetupFocus::AdditionalNotes => { self.controller.set_additional(text); self.notes_received = true; self.notes_scroll = 0; }
        }
        Ok(())
    }
    /// Enter on the job-description box: moves to additional notes
    /// when a JD is present. Returns false when there is no JD yet.
    pub fn confirm_job_description(&mut self) -> bool {
        if !self.is_ready() { return false; }
        self.focus = ExpertSetupFocus::AdditionalNotes;
        true
    }
    /// Returns focus to the job-description box; texts are kept.
    pub fn back_to_job_description(&mut self) { self.focus = ExpertSetupFocus::JobDescription; }
    /// Moves focus to a box (mouse click); texts are kept.
    pub fn set_focus(&mut self, focus: ExpertSetupFocus) { self.focus = focus; }
    /// Scrolls the focused box by `delta` lines (clamped at zero).
    pub fn scroll_focused(&mut self, delta: i32) {
        let scroll = match self.focus { ExpertSetupFocus::JobDescription => &mut self.jd_scroll, ExpertSetupFocus::AdditionalNotes => &mut self.notes_scroll };
        *scroll = (*scroll as i64 + i64::from(delta)).max(0) as usize;
    }
}

/// Feeds one bracketed paste: first fills the JD, second fills the notes,
/// later ones replace the notes. Blank or oversized (>50,000 chars) pastes
/// are rejected without advancing. `Ok(true)` means the proposal is ready.
#[rustfmt::skip]
pub fn apply_expert_setup_paste(pastes: &mut ExpertSetupPastes, text: &str) -> Result<bool, String> {
    validate_paste(text)?;
    if !pastes.jd_received {
        pastes.controller.set_jd(text);
        pastes.jd_received = true;
        return Ok(false);
    }
    pastes.controller.set_additional(text);
    pastes.notes_received = true;
    Ok(true)
}
