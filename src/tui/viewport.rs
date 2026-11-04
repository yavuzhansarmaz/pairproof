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

/// A final-breakdown frame. The questions and the centred rating
/// table are one page; `scroll` is the first visible row.
pub struct FinalFrame {
    pub lines: Vec<String>,
    pub scroll: usize,
    pub max_scroll: usize,
    /// Rows in one page, used as the page-up / page-down step.
    pub page: usize,
}

/// True for the exit line of a final breakdown screen.
fn is_exit_line(line: &str) -> bool {
    let line = line.trim();
    line == "enter close" || line == "enter to close" || line == "enter to exit"
}

/// Drops the hole left where the rating lines used to sit, and
/// trims blank lines off either end of the scrolling body.
fn collapse_blanks(lines: &mut Vec<String>) {
    let mut out = Vec::new();
    for line in lines.drain(..) {
        if line.trim().is_empty()
            && out
                .last()
                .is_some_and(|prev: &String| prev.trim().is_empty())
        {
            continue;
        }
        out.push(line);
    }
    while out.first().is_some_and(|line| line.trim().is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|line| line.trim().is_empty()) {
        out.pop();
    }
    *lines = out;
}

/// One cell, centred in `width` columns.
fn table_cell(text: &str, width: usize) -> String {
    let len = text.chars().count();
    let pad = width.saturating_sub(len);
    let left = pad / 2;
    let right = pad - left;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(right))
}

/// Horizontal rule of a summary table.
fn table_rule(widths: &[usize], left: &str, mid: &str, right: &str) -> String {
    let mut line = String::from(left);
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            line.push_str(mid);
        }
        line.push_str(&"─".repeat(width + 2));
    }
    line.push_str(right);
    line
}

/// One row of a summary table.
fn table_row(cells: &[String]) -> String {
    let mut line = String::from("│");
    for (index, cell) in cells.iter().enumerate() {
        if index > 0 {
            line.push('│');
        }
        line.push(' ');
        line.push_str(cell);
        line.push(' ');
    }
    line.push('│');
    line
}

/// Turns `final rating:` / `solved:` / `finished:` lines into a
/// one-row table. Labels lose a leading `final `.
pub fn summary_table(lines: &[String]) -> Vec<String> {
    let cols: Vec<(String, String)> = lines
        .iter()
        .filter_map(|line| {
            let (label, value) = line.trim().split_once(':')?;
            let label = label.trim().strip_prefix("final ").unwrap_or(label.trim());
            let value = value.trim();
            if label.is_empty() || value.is_empty() {
                return None;
            }
            Some((label.to_string(), value.to_string()))
        })
        .collect();
    if cols.is_empty() {
        return Vec::new();
    }
    let widths: Vec<usize> = cols
        .iter()
        .map(|(label, value)| label.chars().count().max(value.chars().count()))
        .collect();
    let headers: Vec<String> = cols
        .iter()
        .zip(&widths)
        .map(|((label, _), width)| table_cell(label, *width))
        .collect();
    let values: Vec<String> = cols
        .iter()
        .zip(&widths)
        .map(|((_, value), width)| table_cell(value, *width))
        .collect();
    vec![
        table_rule(&widths, "┌", "┬", "┐"),
        table_row(&headers),
        table_rule(&widths, "├", "┼", "┤"),
        table_row(&values),
        table_rule(&widths, "└", "┴", "┘"),
    ]
}

/// Line length that keeps the centred panel the same width on every
/// scroll row. [`crate::tui::render::panel_area`] sizes the card from
/// the longest visible line, so a long question makes it grow and a
/// short one makes it shrink. Zero means the card is already at its
/// minimum and short rows cannot change it.
fn panel_lock_len(area_width: u16, longest: usize) -> usize {
    let natural = (longest as u16).saturating_add(4);
    if natural < 44 {
        return 0;
    }
    let wanted = natural.min(104);
    let panel = if area_width == 0 {
        wanted
    } else {
        area_width.min(wanted)
    };
    if area_width > 0 && area_width < 44 {
        return 0;
    }
    usize::from(panel.saturating_sub(4))
}

/// Centres `line` in `width` columns and pads the row out to that
/// width. The exit line is not passed here: it stays left-aligned,
/// like every other screen's footer.
fn center_line(line: &str, width: usize) -> String {
    let len = line.chars().count();
    if width == 0 || len >= width {
        return line.to_string();
    }
    let left = (width - len) / 2;
    let right = width - len - left;
    format!("{}{line}{}", " ".repeat(left), " ".repeat(right))
}

/// Pads `line` with trailing spaces out to `lock` columns. The text
/// stays where it is; only the measured width changes.
fn hold_width(line: &str, lock: usize) -> String {
    let len = line.chars().count();
    if lock <= len {
        return line.to_string();
    }
    format!("{line}{}", " ".repeat(lock - len))
}

/// Scrolls a final breakdown as one page. Rating lines become a
/// table centred in the panel, placed after the questions, and the
/// whole page moves one row at a time with `scroll`. The exit line
/// stays left-aligned. Every row is padded to the same measured
/// width so the card does not grow or shrink while scrolling.
/// `area_width` is the terminal width. Zero means "no terminal yet".
pub fn final_frame(lines: &[String], height: usize, scroll: usize, area_width: u16) -> FinalFrame {
    if height == 0 {
        return FinalFrame {
            lines: Vec::new(),
            scroll: 0,
            max_scroll: 0,
            page: 1,
        };
    }
    let mut body = Vec::new();
    let mut summary = Vec::new();
    let mut footer = None;
    for line in expand(lines) {
        if is_exit_line(&line) {
            footer = Some("enter to exit".to_string());
        } else if is_pinned_summary(&line) {
            summary.push(line);
        } else {
            body.push(line);
        }
    }
    collapse_blanks(&mut body);
    let mut table = summary_table(&summary);
    let exit = if footer.is_some() || !summary.is_empty() {
        Some(footer.unwrap_or_else(|| "enter to exit".to_string()))
    } else {
        None
    };
    let longest = body
        .iter()
        .chain(table.iter())
        .chain(exit.iter())
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let lock = panel_lock_len(area_width, longest);
    for line in &mut table {
        *line = center_line(line, lock);
    }
    // One document: the questions and the centred rating move
    // together. `enter to exit` stays on the left. A short page keeps
    // the title on the first row and the rating on the last.
    let mut doc = body;
    doc.extend(table);
    if let Some(exit) = exit {
        doc.push(exit);
    }
    let max_scroll = doc.len().saturating_sub(height);
    let scroll = scroll.min(max_scroll);
    let window: Vec<String> = doc.into_iter().skip(scroll).take(height).collect();
    let mut visible = if window.len() < height && !window.is_empty() {
        let pad = height - window.len();
        let mut out = Vec::with_capacity(height);
        out.push(window[0].clone());
        out.extend(std::iter::repeat_n(String::new(), pad));
        out.extend(window.into_iter().skip(1));
        out
    } else {
        window
    };
    if lock > 0 {
        if let Some(first) = visible.first_mut() {
            *first = hold_width(first, lock);
        }
    }
    FinalFrame {
        lines: visible,
        scroll,
        max_scroll,
        page: height.max(1),
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
