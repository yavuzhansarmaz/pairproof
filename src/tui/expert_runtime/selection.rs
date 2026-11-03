//! Expert runtime question selection: medium/hard pool fetching,
//! list navigation (cursor and pages), picking, and commit.

use super::build::widened_note;
use super::*;

/// Questions shown per selection page; left/right change the page.
pub const SELECTION_PAGE_SIZE: usize = 10;

fn required(screen: SelectionScreen) -> usize {
    match screen {
        SelectionScreen::Medium => 3,
        SelectionScreen::Hard => 2,
    }
}

/// Picks needed on a screen: the target, or the whole smaller pool.
fn effective_required(list: &SelectionList, screen: SelectionScreen) -> usize {
    required(screen).min(list.options.len())
}

/// One choosable list: offered pool, picks, and cursor index.
#[derive(Default)]
pub(crate) struct SelectionList {
    pub(crate) options: Vec<QuestionRef>,
    pub(crate) picks: Vec<QuestionRef>,
    pub(crate) cursor: usize,
    pub(crate) preview_cache: std::collections::HashMap<String, String>,
    pub(crate) preview_error: Option<(String, String)>,
}

/// All expert-selection staging; kept in one struct on the runtime.
#[derive(Default)]
pub(crate) struct SelectionState {
    pending_medium: Option<Vec<QuestionRef>>,
    note: Option<String>,
    medium_failed_at: Option<u64>,
    hard_failed_at: Option<u64>,
    medium: SelectionList,
    hard: SelectionList,
}

impl ExpertRuntime {
    pub(crate) fn list(&self, screen: SelectionScreen) -> &SelectionList {
        match screen {
            SelectionScreen::Medium => &self.selection.medium,
            SelectionScreen::Hard => &self.selection.hard,
        }
    }

    pub(crate) fn list_mut(&mut self, screen: SelectionScreen) -> &mut SelectionList {
        match screen {
            SelectionScreen::Medium => &mut self.selection.medium,
            SelectionScreen::Hard => &mut self.selection.hard,
        }
    }

    /// Commits expert medium and hard picks through the core session.
    pub fn expert_select(
        &mut self,
        medium: Vec<QuestionRef>,
        hard: Vec<QuestionRef>,
        now_ms: u64,
    ) -> Result<(), ExpertRuntimeError> {
        self.orch.set_expert_questions(medium, hard, now_ms)?;
        self.selection.pending_medium = None;
        self.maybe_trigger_load();
        Ok(())
    }

    /// Fetches the medium pool for the selection list. A loaded pool
    /// returns without touching the catalog again.
    pub async fn fetch_medium(&mut self, now_ms: u64) -> Result<usize, ExpertRuntimeError> {
        self.fetch_options(SelectionScreen::Medium, now_ms).await
    }

    /// Fetches the hard pool for the selection list, excluding the
    /// session's easy questions and the picked medium questions.
    pub async fn fetch_hard(&mut self, now_ms: u64) -> Result<usize, ExpertRuntimeError> {
        self.fetch_options(SelectionScreen::Hard, now_ms).await
    }

    async fn fetch_options(
        &mut self,
        screen: SelectionScreen,
        now_ms: u64,
    ) -> Result<usize, ExpertRuntimeError> {
        if !self.list(screen).options.is_empty() {
            return Ok(self.list(screen).options.len());
        }
        let client = self
            .pool
            .as_ref()
            .ok_or_else(|| ExpertRuntimeError::Message("question pool unavailable".into()))?;
        let difficulty = match screen {
            SelectionScreen::Medium => Difficulty::Medium,
            SelectionScreen::Hard => Difficulty::Hard,
        };
        let topics: Vec<&str> = self.topics.iter().map(String::as_str).collect();
        let mut exclude: BTreeSet<String> = self
            .orch
            .session()
            .slots()
            .iter()
            .map(|s| s.question.title_slug.clone())
            .collect();
        if screen == SelectionScreen::Hard {
            if let Some(medium) = &self.selection.pending_medium {
                for question in medium {
                    exclude.insert(question.title_slug.clone());
                }
            }
        }
        match pool_with_fallback(
            client.as_ref(),
            difficulty,
            &topics,
            required(screen),
            &exclude,
        )
        .await
        {
            Ok((options, report)) => {
                let count = options.len();
                let list = self.list_mut(screen);
                // A pool at or below the target needs no choosing:
                // every question in it is picked automatically.
                list.picks = if count <= required(screen) {
                    options.clone()
                } else {
                    Vec::new()
                };
                list.options = options;
                list.cursor = 0;
                self.selection.note = widened_note(&report);
                match screen {
                    SelectionScreen::Medium => self.selection.medium_failed_at = None,
                    SelectionScreen::Hard => self.selection.hard_failed_at = None,
                }
                Ok(count)
            }
            Err(err) => {
                match screen {
                    SelectionScreen::Medium => self.selection.medium_failed_at = Some(now_ms),
                    SelectionScreen::Hard => self.selection.hard_failed_at = Some(now_ms),
                }
                Err(ExpertRuntimeError::Message(err.to_string()))
            }
        }
    }

    /// Returns true once the pool for `screen` has been fetched.
    pub fn has_options(&self, screen: SelectionScreen) -> bool {
        !self.list(screen).options.is_empty()
    }

    /// Number of questions currently picked on `screen`.
    pub fn pick_count(&self, screen: SelectionScreen) -> usize {
        self.list(screen).picks.len()
    }

    /// Number of questions offered on `screen`.
    pub fn option_count(&self, screen: SelectionScreen) -> usize {
        self.list(screen).options.len()
    }

    /// True when the pool held at most the target count, so every
    /// offered question was picked automatically at fetch time.
    pub fn auto_selected(&self, screen: SelectionScreen) -> bool {
        let list = self.list(screen);
        !list.options.is_empty()
            && list.options.len() <= required(screen)
            && list.picks.len() == list.options.len()
    }

    /// Absolute cursor index into the pool for `screen`.
    pub fn selection_cursor(&self, screen: SelectionScreen) -> usize {
        self.list(screen).cursor
    }

    /// Moves the cursor by `delta` questions, clamped to the pool.
    pub fn move_selection_cursor(&mut self, screen: SelectionScreen, delta: i32) {
        let list = self.list_mut(screen);
        if list.options.is_empty() {
            return;
        }
        let last = list.options.len() - 1;
        list.cursor = (list.cursor as i64 + i64::from(delta)).clamp(0, last as i64) as usize;
    }

    /// Moves the cursor by whole pages, keeping its row on the page.
    pub fn move_selection_page(&mut self, screen: SelectionScreen, delta: i32) {
        let list = self.list_mut(screen);
        if list.options.is_empty() {
            return;
        }
        let pages = list.options.len().div_ceil(SELECTION_PAGE_SIZE);
        let page = list.cursor / SELECTION_PAGE_SIZE;
        let target = (page as i64 + i64::from(delta)).clamp(0, pages as i64 - 1) as usize;
        let row = list.cursor % SELECTION_PAGE_SIZE;
        list.cursor = (target * SELECTION_PAGE_SIZE + row).min(list.options.len() - 1);
    }

    /// Toggles the question under the cursor. Picking beyond the
    /// required count is refused (deselect first); returns whether
    /// the picks changed.
    pub fn toggle_selection_pick(&mut self, screen: SelectionScreen) -> bool {
        let list = self.list_mut(screen);
        // A pool at or below the target is fully auto-selected and
        // locked: dropping a pick would lose an irreplaceable question.
        if list.options.len() <= required(screen) {
            return false;
        }
        let need = effective_required(list, screen);
        let Some(question) = list.options.get(list.cursor).cloned() else {
            return false;
        };
        if let Some(at) = list
            .picks
            .iter()
            .position(|pick| pick.title_slug == question.title_slug)
        {
            list.picks.remove(at);
            return true;
        }
        if list.picks.len() >= need {
            return false;
        }
        list.picks.push(question);
        true
    }

    /// Stages the picked medium questions; the effective count
    /// (target, or the whole pool when smaller) is required.
    pub fn confirm_medium(&mut self) -> Result<(), ExpertRuntimeError> {
        let list = &self.selection.medium;
        let need = effective_required(list, SelectionScreen::Medium);
        if list.picks.len() != need {
            return Err(ExpertRuntimeError::Message(format!(
                "select {} more medium question{} from the list ({} of {need} selected)",
                need - list.picks.len(),
                if need - list.picks.len() == 1 {
                    ""
                } else {
                    "s"
                },
                list.picks.len()
            )));
        }
        self.selection.pending_medium = Some(list.picks.clone());
        Ok(())
    }

    /// Commits the picked hard questions with the staged medium set;
    /// the effective count (target, or whole pool) is required.
    pub async fn complete_selection(&mut self, now_ms: u64) -> Result<(), ExpertRuntimeError> {
        let hard = &self.selection.hard;
        let need = effective_required(hard, SelectionScreen::Hard);
        if hard.picks.len() != need {
            return Err(ExpertRuntimeError::Message(format!(
                "select {} more hard question{} from the list ({} of {need} selected)",
                need - hard.picks.len(),
                if need - hard.picks.len() == 1 {
                    ""
                } else {
                    "s"
                },
                hard.picks.len()
            )));
        }
        let hard = hard.picks.clone();
        let medium = self.selection.pending_medium.clone().ok_or_else(|| {
            ExpertRuntimeError::Message("medium questions are not selected yet".into())
        })?;
        self.expert_select(medium, hard, now_ms)
    }

    /// Replaces the picks with a random draw; returns the pick count.
    pub fn randomize_picks(&mut self, screen: SelectionScreen) -> usize {
        use rand::seq::SliceRandom;
        let need = effective_required(self.list(screen), screen);
        let list = self.list_mut(screen);
        // Pagination is display-only: draw from all offered questions.
        let mut shuffled = list.options.clone();
        shuffled.shuffle(&mut rand::thread_rng());
        list.picks = shuffled.into_iter().take(need).collect();
        list.picks.len()
    }

    /// Renders the current page of the selection list for `screen`.
    pub fn selection_lines(&self, screen: SelectionScreen) -> Vec<String> {
        let list = self.list(screen);
        if list.options.is_empty() {
            return Vec::new();
        }
        let pages = list.options.len().div_ceil(SELECTION_PAGE_SIZE);
        let page = list.cursor / SELECTION_PAGE_SIZE;
        let mut lines = vec![format!(
            "page {}/{}, selected: {}/{}",
            page + 1,
            pages,
            list.picks.len(),
            effective_required(list, screen)
        )];
        if self.auto_selected(screen) {
            lines.push("all available questions are selected automatically.".to_string());
        }
        for (index, question) in list.options.iter().enumerate() {
            if index / SELECTION_PAGE_SIZE != page {
                continue;
            }
            let mark = if list
                .picks
                .iter()
                .any(|pick| pick.title_slug == question.title_slug)
            {
                "[x]"
            } else {
                "[ ]"
            };
            let cursor = if index == list.cursor { ">" } else { " " };
            let acceptance = question
                .acceptance_pct
                .map(|pct| format!(" (acceptance {pct}%)"))
                .unwrap_or_default();
            lines.push(crate::tui::shared::sanitize_display(&format!(
                "{cursor} {}. {mark} {}{acceptance}",
                index + 1,
                question.title
            )));
        }
        lines
    }

    /// Returns false while the screen backs off after a failed attempt.
    /// Pure in `now_ms`: no wall-clock reads, first attempt is immediate.
    pub fn selection_allowed(&self, screen: SelectionScreen, now_ms: u64) -> bool {
        let failed = match screen {
            SelectionScreen::Medium => self.selection.medium_failed_at,
            SelectionScreen::Hard => self.selection.hard_failed_at,
        };
        failed.is_none_or(|at| now_ms.saturating_sub(at) >= SELECTION_RETRY_MS)
    }
    /// Expert-visible note from the last widened selection, if any.
    pub fn selection_note(&self) -> Option<&str> {
        self.selection.note.as_deref()
    }
    /// Overrides the question pool source (test seam for selection).
    pub fn set_pool(&mut self, pool: Arc<dyn PoolClient>) {
        self.pool = Some(pool);
    }
    pub fn set_secret_pusher(
        &mut self,
        pusher: Arc<dyn crate::orchestration::secret_push::SecretPush>,
    ) {
        self.secret_pusher = Some(pusher);
    }
    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = Some(clock);
    }
}
