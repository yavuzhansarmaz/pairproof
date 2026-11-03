//! Pure viewport: pinned summary lines survive short terminals.

/// Builds the explicit marker shown when content lines are hidden.
pub fn truncation_marker(hidden: usize) -> String {
    format!("... {hidden} more lines \u{2014} scroll/resize for full text")
}

/// Returns true for rating and completion lines that stay pinned.
pub fn is_pinned_summary(line: &str) -> bool {
    let line = line.trim_start();
    [
        "final rating:",
        "live rating:",
        "solved:",
        "finished:",
        "skipped:",
        "paste flags:",
    ]
    .iter()
    .any(|prefix| line.starts_with(prefix))
}

fn expand(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| line.split('\n').map(str::to_string))
        .collect()
}

/// Splits lines into (content, pinned) preserving order in each part.
pub fn split_pinned(lines: &[String]) -> (Vec<String>, Vec<String>) {
    let mut content = Vec::new();
    let mut pinned = Vec::new();
    for line in expand(lines) {
        if is_pinned_summary(&line) {
            pinned.push(line);
        } else {
            content.push(line);
        }
    }
    (content, pinned)
}

/// Returns visible lines: pinned always present when they fit the height.
pub fn visible_lines(content: &[String], height: usize, pinned: &[String]) -> Vec<String> {
    if height == 0 {
        return Vec::new();
    }
    let content = expand(content);
    let pinned = expand(pinned);
    if content.len() + pinned.len() <= height {
        return content.into_iter().chain(pinned).collect();
    }
    if pinned.is_empty() {
        if height == 1 {
            return vec![truncation_marker(content.len())];
        }
        let shown = height - 1;
        let hidden = content.len() - shown;
        let mut out: Vec<String> = content.into_iter().take(shown).collect();
        out.push(truncation_marker(hidden));
        return out;
    }
    if height <= pinned.len() {
        if content.is_empty() || height == 1 {
            return pinned.into_iter().take(height).collect();
        }
        let mut out = vec![truncation_marker(content.len())];
        out.extend(pinned.into_iter().take(height - 1));
        return out;
    }
    let avail = height - pinned.len();
    if avail == 1 {
        let mut out = vec![truncation_marker(content.len())];
        out.extend(pinned);
        return out;
    }
    let shown = avail - 1;
    let hidden = content.len() - shown;
    let mut out: Vec<String> = content.into_iter().take(shown).collect();
    out.push(truncation_marker(hidden));
    out.extend(pinned);
    out
}

/// Wrapped rows for one line at `width` columns (greedy word
/// wrap, matching `Wrap { trim: true }` closely enough for ASCII
/// pane text). Width 0 counts every line as 1 row.
pub fn wrapped_rows(line: &str, width: usize) -> usize {
    let width = width.max(1);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return 1;
    }
    let mut rows = 1usize;
    let mut col = 0usize;
    for word in trimmed.split_whitespace() {
        let mut word_len = unicode_width::UnicodeWidthStr::width(word);
        if col > 0 && col + 1 + word_len > width {
            rows += 1;
            col = 0;
        }
        if col > 0 {
            col += 1;
        }
        while word_len > width {
            rows += 1;
            word_len -= width;
        }
        col += word_len;
    }
    rows
}

/// Total wrapped rows for `lines` at `width` columns.
pub fn wrapped_row_count(lines: &[String], width: usize) -> usize {
    lines.iter().map(|line| wrapped_rows(line, width)).sum()
}

/// Wrapped rows counted only until the total reaches `cap`: the
/// exact total when it is below `cap`, otherwise a value >= `cap`.
pub fn wrapped_row_count_up_to(lines: &[String], width: usize, cap: usize) -> usize {
    let mut total = 0usize;
    for line in lines {
        total = total.saturating_add(wrapped_rows(line, width));
        if total >= cap {
            return total;
        }
    }
    total
}

/// Clamps a wrapped-row `scroll` so the pane is never blanked:
/// a scroll still inside the content is returned unchanged, and
/// an overscroll lands on the last full page.
pub fn clamp_wrapped_scroll(
    lines: &[String],
    width: usize,
    scroll: usize,
    visible: usize,
) -> usize {
    let limit = scroll.saturating_add(visible);
    let total = wrapped_row_count_up_to(lines, width, limit.saturating_add(1));
    if total > limit {
        scroll
    } else {
        scroll.min(total.saturating_sub(visible))
    }
}

/// Window into `lines` for wrapped-row `scroll` and `visible`
/// visible rows: returns `(start_index, local_scroll)` where
/// `start_index` is the first line whose wrapped rows are not
/// entirely above `scroll`, and `local_scroll` is the wrapped-row
/// offset of `scroll` inside that line. Callers take lines from
/// `start_index` while cumulative wrapped rows cover
/// `local_scroll + visible + 1` rows.
pub fn wrapped_window(
    lines: &[String],
    width: usize,
    scroll: usize,
    _visible: usize,
) -> (usize, usize) {
    let mut rows_before = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let rows = wrapped_rows(line, width);
        if scroll < rows_before + rows {
            return (index, scroll - rows_before);
        }
        rows_before += rows;
    }
    if lines.is_empty() {
        (0, 0)
    } else {
        (lines.len(), 0)
    }
}

/// Applies [`visible_lines`] after pinning summary lines found in `lines`.
pub fn visible_with_pinned(lines: &[String], height: usize) -> Vec<String> {
    let expanded = expand(lines);
    if expanded.len() <= height {
        return expanded;
    }
    let (content, pinned) = split_pinned(lines);
    visible_lines(&content, height, &pinned)
}
