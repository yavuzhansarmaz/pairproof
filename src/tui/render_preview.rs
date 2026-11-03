//! Question preview screen: structured data, statement
//! wrapping/classification, scroll metrics, and the full-screen
//! renderer used by the expert selection screens.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::render::{
    key_span, title_line, ENTER_KEY_COLOR, ESC_KEY_COLOR, FRAME_COLOR, HEADING_COLOR, PAIR_COLOR,
    TEXT_COLOR,
};

/// Structured question-preview data for the expert selection
/// screens: question metadata plus the plain-text statement, when
/// it has arrived (`None` body means still loading).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionPreviewData {
    pub title: String,
    pub difficulty: String,
    pub acceptance: String,
    pub slug: String,
    pub position: usize,
    pub total: usize,
    pub picked: bool,
    pub body: Option<String>,
}

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
fn wrap_preview_text(
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

/// Content width for the preview: the full area inside the outer
/// border, edge to edge with no side margins.
fn preview_content_width(frame_width: u16) -> usize {
    frame_width.saturating_sub(2).max(1) as usize
}

fn preview_title_rows(title: &str, width: usize) -> usize {
    wrap_preview_text(title, width, "", "").len().clamp(1, 2)
}

/// Scroll geometry for one preview frame, shared by the frame
/// planner (which clamps the scroll offset) and the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewMetrics {
    pub total_rows: usize,
    pub body_rows: usize,
    pub max_scroll: usize,
}

/// Computes total wrapped rows, visible body rows, and the largest
/// useful scroll offset for `preview` at a `width` x `height` frame.
pub fn preview_metrics(preview: &QuestionPreviewData, width: u16, height: u16) -> PreviewMetrics {
    let content_width = preview_content_width(width);
    let total_rows = preview
        .body
        .as_deref()
        .map(|body| preview_body_rows(body, content_width).len())
        .unwrap_or(0);
    // Outer borders (2) + header + title + meta + separator + footer.
    let overhead = 2 + 1 + preview_title_rows(&preview.title, content_width) + 1 + 1 + 1;
    let body_rows = (height as usize).saturating_sub(overhead).max(1);
    PreviewMetrics {
        total_rows,
        body_rows,
        max_scroll: total_rows.saturating_sub(body_rows),
    }
}

/// Draws the full-screen question preview: pinned header (brand
/// section label, wrapped title, one-line metadata), a scrollable
/// wrapped statement body with styled section headings, and a
/// pinned footer that always shows how to scroll and go back.
pub fn draw_question_preview_screen(
    frame: &mut Frame,
    preview: &QuestionPreviewData,
    scroll: usize,
) {
    let area = frame.area();
    let outer = Block::default()
        .title(title_line())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(FRAME_COLOR));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let content_width = preview_content_width(area.width);
    let content = inner;
    let title_rows = preview_title_rows(&preview.title, content_width) as u16;
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(title_rows),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(0),
            ratatui::layout::Constraint::Length(1),
        ])
        .split(content);
    // Header: section label left, position right.
    let header = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            ratatui::layout::Constraint::Min(0),
            ratatui::layout::Constraint::Length(16),
        ])
        .split(chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Question preview",
            Style::default()
                .fg(HEADING_COLOR)
                .add_modifier(Modifier::BOLD),
        ))),
        header[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{} of {}", preview.position, preview.total),
            Style::default().fg(TEXT_COLOR),
        )))
        .alignment(ratatui::layout::Alignment::Right),
        header[1],
    );
    frame.render_widget(
        Paragraph::new(preview.title.clone())
            .style(Style::default().fg(TEXT_COLOR).add_modifier(Modifier::BOLD))
            .wrap(ratatui::widgets::Wrap { trim: true }),
        chunks[1],
    );
    let meta_label = |text: &str| {
        Span::styled(
            text.to_string(),
            Style::default()
                .fg(HEADING_COLOR)
                .add_modifier(Modifier::BOLD),
        )
    };
    let mut meta = vec![
        meta_label("difficulty"),
        Span::raw(format!(" {}  ", preview.difficulty)),
        meta_label("acceptance"),
        Span::raw(format!(" {}  ", preview.acceptance)),
        meta_label("slug"),
        Span::raw(format!(" {}  ", preview.slug)),
    ];
    if preview.picked {
        meta.push(Span::styled(
            "selected",
            Style::default()
                .fg(ENTER_KEY_COLOR)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        meta.push(Span::raw("not selected"));
    }
    frame.render_widget(
        Paragraph::new(Line::from(meta)).style(Style::default().fg(TEXT_COLOR)),
        chunks[2],
    );
    let separator =
        Paragraph::new("\u{2500}".repeat(content_width)).style(Style::default().fg(FRAME_COLOR));
    frame.render_widget(separator, chunks[3]);
    // Body: wrapped rows, sliced to the scroll window. The footer
    // below is pinned, so it stays visible at any scroll offset.
    // Rows are wrapped exactly once here and reused for the scroll
    // indicator, so a frame never re-wraps the statement.
    let rows = preview
        .body
        .as_deref()
        .map(|body| preview_body_rows(body, content_width))
        .unwrap_or_default();
    let total_rows = rows.len();
    let effective_scroll = scroll.min(total_rows.saturating_sub(chunks[4].height as usize));
    let body_lines: Vec<Line> = match &preview.body {
        None => vec![Line::from(Span::styled(
            "loading the question...",
            Style::default()
                .fg(TEXT_COLOR)
                .add_modifier(Modifier::ITALIC),
        ))],
        Some(_) => rows
            .iter()
            .skip(effective_scroll)
            .take(chunks[4].height as usize)
            .map(|row| match row.kind {
                PreviewRowKind::Heading => Line::from(Span::styled(
                    row.text.clone(),
                    Style::default()
                        .fg(HEADING_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                PreviewRowKind::Bullet => {
                    if let Some(item) = row.text.strip_prefix("- ") {
                        Line::from(vec![
                            Span::styled(
                                "- ",
                                Style::default().fg(PAIR_COLOR).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(item.to_string()),
                        ])
                    } else {
                        Line::from(Span::raw(row.text.clone()))
                    }
                }
                PreviewRowKind::Body | PreviewRowKind::Blank => {
                    Line::from(Span::raw(row.text.clone()))
                }
            })
            .collect(),
    };
    frame.render_widget(
        Paragraph::new(body_lines).style(Style::default().fg(TEXT_COLOR)),
        chunks[4],
    );
    // On narrow terminals the scroll indicator gives its space to
    // the key hints, which must never be truncated away.
    let indicator_width = if content_width >= 80 { 18 } else { 0 };
    let footer = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            ratatui::layout::Constraint::Min(0),
            ratatui::layout::Constraint::Length(indicator_width),
        ])
        .split(chunks[5]);
    let mut footer_spans = vec![
        key_span("left/right", PAIR_COLOR),
        Span::raw(" question  "),
        key_span("up/down", PAIR_COLOR),
        Span::raw(" scroll  "),
    ];
    if content_width >= 96 {
        footer_spans.push(key_span("pgup/pgdn", PAIR_COLOR));
        footer_spans.push(Span::raw(" page  "));
    }
    footer_spans.extend([key_span("b", PAIR_COLOR), Span::raw(" back to list")]);
    if content_width >= 64 {
        footer_spans.push(Span::raw("  "));
        footer_spans.push(key_span("esc", ESC_KEY_COLOR));
        footer_spans.push(Span::raw(" quit"));
    }
    frame.render_widget(
        Paragraph::new(Line::from(footer_spans)).style(Style::default().fg(TEXT_COLOR)),
        footer[0],
    );
    let indicator = if total_rows > chunks[4].height as usize {
        let end = (effective_scroll + chunks[4].height as usize).min(total_rows);
        format!("{}-{} / {}", effective_scroll + 1, end, total_rows)
    } else if total_rows > 0 {
        "all shown".to_string()
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            indicator,
            Style::default().fg(HEADING_COLOR),
        )))
        .alignment(ratatui::layout::Alignment::Right),
        footer[1],
    );
}
