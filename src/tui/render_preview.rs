//! Question preview screen: structured data, statement
//! wrapping/classification, scroll metrics, and the full-screen
//! renderer used by the expert selection screens.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::statement_text::wrap_preview_text;
pub use crate::tui::statement_text::{
    preview_body_rows, preview_line_kind, PreviewRow, PreviewRowKind,
};

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
    pub position: usize,
    pub total: usize,
    pub picked: bool,
    pub body: Option<String>,
    /// Why the statement failed to load, when it did. The expert
    /// sees the real reason; failures are never cached as a body.
    pub load_error: Option<String>,
}

/// Content width for the preview: the full area inside the outer
/// border, minus a two-column reading padding on each side so
/// text never hugs the frame.
fn preview_content_width(frame_width: u16) -> usize {
    frame_width.saturating_sub(6).max(1) as usize
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
    tick: usize,
) {
    let area = frame.area();
    let outer = Block::default()
        .title(title_line())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(FRAME_COLOR));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let content_width = preview_content_width(area.width);
    let content = Rect {
        x: inner.x.saturating_add(2),
        width: inner.width.saturating_sub(4),
        ..inner
    };
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
    let body_lines: Vec<Line> = match (&preview.load_error, &preview.body) {
        (Some(reason), _) => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "the question could not be loaded.",
                    Style::default()
                        .fg(ESC_KEY_COLOR)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::raw(String::new())),
            ];
            lines.extend(
                preview_body_rows(reason, content_width)
                    .iter()
                    .map(|row| Line::from(Span::raw(row.text.clone()))),
            );
            lines.push(Line::from(Span::raw(String::new())));
            lines.push(Line::from(Span::styled(
                "step away and back, or reopen the preview, to try again.",
                Style::default()
                    .fg(TEXT_COLOR)
                    .add_modifier(Modifier::ITALIC),
            )));
            lines
        }
        (None, None) => vec![Line::from(Span::styled(
            format!("loading the question{}", ".".repeat(tick % 3 + 1)),
            Style::default()
                .fg(TEXT_COLOR)
                .add_modifier(Modifier::ITALIC),
        ))],
        (None, Some(_)) => rows
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
        key_span("space", ENTER_KEY_COLOR),
        Span::raw(" select  "),
        key_span("left/right", PAIR_COLOR),
        Span::raw(" question  "),
        key_span("b", PAIR_COLOR),
        Span::raw(" back to list  "),
        key_span("up/down", PAIR_COLOR),
        Span::raw(" scroll"),
    ];
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
