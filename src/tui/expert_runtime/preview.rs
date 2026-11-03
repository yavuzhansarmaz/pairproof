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

    /// Caches a fetched preview statement (plain text) for `slug`.
    pub fn store_preview(&mut self, screen: SelectionScreen, slug: String, text: String) {
        let list = self.list_mut(screen);
        list.preview_slug = Some(slug);
        list.preview_text = Some(text);
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
