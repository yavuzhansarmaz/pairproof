//! Plain-text statement rows: section classification and word
//! wrapping shared by the expert question preview and the
//! candidate question pane.

/// One display row of a question-preview body, after wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewRowKind {
    Heading,
    Body,
    Bullet,
    Blank,
}

/// A wrapped preview row plus how it should be styled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRow {
    pub text: String,
    pub kind: PreviewRowKind,
}

fn is_preview_heading(trimmed: &str) -> bool {
    let lower = trimmed.to_ascii_lowercase();
    let section = lower.starts_with("example")
        || lower.starts_with("constraints")
        || lower.starts_with("note")
        || lower.starts_with("explanation")
        || lower.starts_with("follow up")
        || lower.starts_with("follow-up")
        || lower.starts_with("hint");
    section && (trimmed.ends_with(':') || !lower.contains(' '))
}

/// Word-wraps `text` to `width` columns; continuation rows use
/// `rest_prefix` so bullets keep a hanging indent. Words longer
/// than the available width are hard-split.
pub fn wrap_preview_text(
    text: &str,
    width: usize,
    first_prefix: &str,
    rest_prefix: &str,
) -> Vec<String> {
    let width = width.max(8);
    let mut rows = Vec::new();
    let mut prefix = first_prefix;
    let mut current = String::new();
    let avail = |prefix: &str| width.saturating_sub(prefix.chars().count()).max(1);
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if word_len > avail(prefix) && current.is_empty() {
            let chars: Vec<char> = word.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let take = avail(prefix).min(chars.len() - start);
                rows.push(format!(
                    "{prefix}{}",
                    chars[start..start + take].iter().collect::<String>()
                ));
                start += take;
                prefix = rest_prefix;
            }
            continue;
        }
        if current.is_empty() {
            current = word.to_string();
        } else if current.chars().count() + 1 + word_len <= avail(prefix) {
            current.push(' ');
            current.push_str(word);
        } else {
            rows.push(format!("{prefix}{current}"));
            prefix = rest_prefix;
            current = word.to_string();
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(format!("{prefix}{current}"));
    }
    rows
}

/// Classifies one plain-text line for styling: section headings
/// (`Example 1:`, `Constraints:`, ...), `- ` bullets, blanks, and
/// ordinary body text. Shared by the expert preview and the
/// candidate question pane so both style statements identically.
pub fn preview_line_kind(text: &str) -> PreviewRowKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        PreviewRowKind::Blank
    } else if is_preview_heading(trimmed) {
        PreviewRowKind::Heading
    } else if trimmed.starts_with("- ") || trimmed.starts_with("\u{2022} ") {
        PreviewRowKind::Bullet
    } else {
        PreviewRowKind::Body
    }
}

/// Classifies and wraps a plain-text statement into display rows:
/// section lines (`Example 1:`, `Constraints:`, ...) become
/// headings, `- ` items become hanging-indent bullets, and every
/// other line is word-wrapped so long question and example text
/// fits the pane instead of running off screen.
pub fn preview_body_rows(body: &str, width: usize) -> Vec<PreviewRow> {
    let width = width.max(8);
    let mut rows = Vec::new();
    for raw in body.lines() {
        let line = crate::tui::shared::sanitize_display(raw);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            rows.push(PreviewRow {
                text: String::new(),
                kind: PreviewRowKind::Blank,
            });
        } else if is_preview_heading(trimmed) {
            rows.push(PreviewRow {
                text: trimmed.to_string(),
                kind: PreviewRowKind::Heading,
            });
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("\u{2022} "))
        {
            for row in wrap_preview_text(item, width, "- ", "  ") {
                rows.push(PreviewRow {
                    text: row,
                    kind: PreviewRowKind::Bullet,
                });
            }
        } else {
            for row in wrap_preview_text(trimmed, width, "", "") {
                rows.push(PreviewRow {
                    text: row,
                    kind: PreviewRowKind::Body,
                });
            }
        }
    }
    rows
}
