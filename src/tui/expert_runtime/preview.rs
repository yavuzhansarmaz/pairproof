//! Expert runtime question preview: statement fetch cache
//! and the full-screen preview lines.

use super::*;

impl ExpertRuntime {
    /// Fetch request for the preview statement of the question
    /// under the cursor: fetcher, slug, and session language. None
    /// when that statement is already cached for the preview.
    pub fn preview_request(
        &self,
        screen: SelectionScreen,
    ) -> Option<(Arc<dyn StatementFetcher>, String, crate::core::Language)> {
        let list = self.list(screen);
        let question = list.options.get(list.cursor)?;
        if list.preview_slug.as_deref() == Some(question.title_slug.as_str())
            && list.preview_text.is_some()
        {
            return None;
        }
        Some((
            self.fetcher.clone(),
            question.title_slug.clone(),
            self.orch.session().language(),
        ))
    }

    /// Moves the preview to the previous/next question in the
    /// list; returns true when the cursor actually moved (it
    /// clamps at both ends instead of wrapping).
    pub fn step_preview_question(&mut self, screen: SelectionScreen, delta: i32) -> bool {
        let before = self.selection_cursor(screen);
        self.move_selection_cursor(screen, delta);
        self.selection_cursor(screen) != before
    }

    /// Caches a fetched preview statement (plain text) for `slug`.
    pub fn store_preview(&mut self, screen: SelectionScreen, slug: String, text: String) {
        let list = self.list_mut(screen);
        list.preview_slug = Some(slug);
        list.preview_text = Some(text);
    }

    /// Structured preview data for the redesigned preview screen:
    /// metadata plus the fetched statement (`None` while loading).
    pub fn question_preview(
        &self,
        screen: SelectionScreen,
    ) -> Option<crate::tui::render::QuestionPreviewData> {
        let list = self.list(screen);
        let question = list.options.get(list.cursor)?;
        let difficulty = match question.difficulty {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
        };
        let body = match (&list.preview_slug, &list.preview_text) {
            (Some(slug), Some(text)) if *slug == question.title_slug => {
                Some(crate::tui::shared::sanitize_display(text))
            }
            _ => None,
        };
        Some(crate::tui::render::QuestionPreviewData {
            title: crate::tui::shared::sanitize_display(&question.title),
            difficulty: difficulty.to_string(),
            acceptance: question
                .acceptance_pct
                .map(|pct| format!("{pct}%"))
                .unwrap_or_else(|| "unknown".to_string()),
            slug: crate::tui::shared::sanitize_display(&question.title_slug),
            position: list.cursor + 1,
            total: list.options.len(),
            picked: list
                .picks
                .iter()
                .any(|pick| pick.title_slug == question.title_slug),
            body,
        })
    }

    /// Full-screen preview lines for the question under the cursor:
    /// metadata, the fetched statement, plus how to get back.
    pub fn selection_preview_lines(&self, screen: SelectionScreen) -> Vec<String> {
        let list = self.list(screen);
        let Some(question) = list.options.get(list.cursor) else {
            return Vec::new();
        };
        let difficulty = match question.difficulty {
            Difficulty::Easy => "easy",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
        };
        let acceptance = question
            .acceptance_pct
            .map(|pct| format!("{pct}%"))
            .unwrap_or_else(|| "unknown".to_string());
        let picked = if list
            .picks
            .iter()
            .any(|pick| pick.title_slug == question.title_slug)
        {
            "selected"
        } else {
            "not selected"
        };
        let mut lines = vec![
            crate::tui::shared::sanitize_display(&question.title),
            String::new(),
            format!("difficulty: {difficulty}"),
            format!("acceptance: {acceptance}"),
            format!("slug: {}", question.title_slug),
            format!(
                "question {} of {} ({picked})",
                list.cursor + 1,
                list.options.len()
            ),
            String::new(),
        ];
        match (&list.preview_slug, &list.preview_text) {
            (Some(slug), Some(text)) if *slug == question.title_slug => {
                lines.extend(text.lines().map(str::to_string));
            }
            _ => lines.push("loading the question...".to_string()),
        }
        lines.push(String::new());
        lines.push("up/down scroll, b back to the list".to_string());
        lines
            .iter()
            .map(|line| crate::tui::shared::sanitize_display(line))
            .collect()
    }
}
