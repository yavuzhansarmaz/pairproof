//! Pure viewport: pinned summary lines survive short terminals.

/// Builds the explicit marker shown when content lines are hidden.
pub fn truncation_marker(hidden: usize) -> String {
    format!("... {hidden} more lines \u{2014} scroll/resize for full text")
}

/// Returns true for rating and completion lines that stay pinned.
pub fn is_pinned_summary(line: &str) -> bool {
    let line = line.trim_start();
    [
        "Final rating:",
        "Live rating:",
        "Solved:",
        "Finished:",
        "Skipped:",
        "Paste flags:",
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

/// Applies [`visible_lines`] after pinning summary lines found in `lines`.
pub fn visible_with_pinned(lines: &[String], height: usize) -> Vec<String> {
    let expanded = expand(lines);
    if expanded.len() <= height {
        return expanded;
    }
    let (content, pinned) = split_pinned(lines);
    visible_lines(&content, height, &pinned)
}
