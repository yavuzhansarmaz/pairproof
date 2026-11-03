//! Expert Setup paste collector: the two text boxes, word-wise
//! backspace, and Tab focus switching.

use crate::core::Language;
use crate::ingest::ProposedContext;
use crate::llm::MetaClient;
use crate::tui::setup::{SetupController, SetupError};

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
    /// when a JD is present, clearing any previously written notes
    /// so re-entering after going back starts the notes fresh.
    /// Returns false when there is no JD yet.
    pub fn confirm_job_description(&mut self) -> bool {
        if !self.is_ready() { return false; }
        self.controller.set_additional("");
        self.notes_received = false;
        self.notes_scroll = 0;
        self.focus = ExpertSetupFocus::AdditionalNotes;
        true
    }
    /// Deletes the last character of the focused box, if any;
    /// emptying the job description also un-marks it as received.
    pub fn backspace_focused(&mut self) {
        match self.focus {
            ExpertSetupFocus::JobDescription => {
                let mut text = self.controller.draft().jd.clone();
                text.pop();
                self.jd_received = !text.is_empty();
                self.controller.set_jd(&text);
            }
            ExpertSetupFocus::AdditionalNotes => {
                let mut text = self.controller.draft().additional.clone();
                text.pop();
                self.notes_received = !text.is_empty();
                self.controller.set_additional(&text);
            }
        }
    }
    /// Switches focus between the two boxes (Tab); texts are kept.
    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            ExpertSetupFocus::JobDescription => ExpertSetupFocus::AdditionalNotes,
            ExpertSetupFocus::AdditionalNotes => ExpertSetupFocus::JobDescription,
        };
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
