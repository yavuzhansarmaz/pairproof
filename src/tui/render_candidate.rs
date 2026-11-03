//! Candidate solving screen: two panes in the expert-preview
//! design language. The question pane gets a bold title line,
//! navy section headings, a two-column reading padding, a pinned
//! focus footer, and its own scroll offset; the Coach pane keeps
//! its existing rendering. Pane focus, the draggable split, and
//! every key action are unchanged — this module only draws.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::render::{
    body_line, title_line, FRAME_COLOR, HEADING_COLOR, PAIR_COLOR, PROOF_COLOR, TEXT_COLOR,
};
use crate::tui::render_preview::{preview_line_kind, PreviewRowKind};

/// Styles one question-pane line like the expert preview: the
/// first line is the bold title, section headings are navy bold,
/// bullets get a navy marker, and `selected approach:` /
/// `latest test:` lines get a navy label.
fn question_line(line: &str, first: bool) -> Line<'static> {
    if first {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(TEXT_COLOR).add_modifier(Modifier::BOLD),
        ));
    }
    match preview_line_kind(line) {
        PreviewRowKind::Heading => Line::from(Span::styled(
            line.trim().to_string(),
            Style::default()
                .fg(HEADING_COLOR)
                .add_modifier(Modifier::BOLD),
        )),
        PreviewRowKind::Bullet => {
            if let Some(item) = line.trim().strip_prefix("- ") {
                Line::from(vec![
                    Span::styled(
                        "- ",
                        Style::default().fg(PAIR_COLOR).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(item.to_string()),
                ])
            } else {
                body_line(line)
            }
        }
        _ => {
            for label in ["selected approach:", "latest test:", "phase:"] {
                if let Some(rest) = line.strip_prefix(label) {
                    return Line::from(vec![
                        Span::styled(
                            label.to_string(),
                            Style::default()
                                .fg(HEADING_COLOR)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(rest.to_string()),
                    ]);
                }
            }
            body_line(line)
        }
    }
}

fn pane_block(title: Line<'static>, focused: bool) -> Block<'static> {
    let border = if focused { PROOF_COLOR } else { FRAME_COLOR };
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
}

/// Draws the candidate solving screen as two panes: question and
/// approaches on the left, the Coach chat on the right, split at
/// `question_pct` percent (draggable in the TTY loop, 30..=70). The
/// focused pane's border uses the purple accent, the other the
/// frame navy. The question pane scrolls by `question_scroll`
/// wrapped rows; its `focus:` hint line is pinned to the bottom.
pub fn draw_candidate_screen(
    frame: &mut Frame,
    left: &[String],
    right: &[String],
    chat_focused: bool,
    question_pct: u16,
    question_scroll: usize,
) {
    let chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            ratatui::layout::Constraint::Percentage(question_pct),
            ratatui::layout::Constraint::Percentage(100 - question_pct),
        ])
        .split(frame.area());

    // Question pane: pinned `focus:` footer, padded scrollable body.
    let block = pane_block(title_line(), !chat_focused);
    let inner = block.inner(chunks[0]);
    frame.render_widget(block, chunks[0]);
    let padded = Rect {
        x: inner.x.saturating_add(2),
        width: inner.width.saturating_sub(4),
        ..inner
    };
    let (content_lines, footer_line): (&[String], Option<&str>) =
        match left.last().map(String::as_str) {
            Some(last) if last.starts_with("focus:") => (&left[..left.len() - 1], Some(last)),
            _ => (left, None),
        };
    let rows = if footer_line.is_some() {
        ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Min(0),
                ratatui::layout::Constraint::Length(1),
            ])
            .split(padded)
    } else {
        ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([ratatui::layout::Constraint::Min(0)])
            .split(padded)
    };
    let content: Vec<Line> = content_lines
        .iter()
        .enumerate()
        .map(|(index, line)| question_line(line, index == 0))
        .collect();
    frame.render_widget(
        Paragraph::new(content)
            .style(Style::default().fg(TEXT_COLOR))
            .wrap(ratatui::widgets::Wrap { trim: true })
            .scroll((question_scroll.min(u16::MAX as usize) as u16, 0)),
        rows[0],
    );
    if let Some(footer) = footer_line {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer.to_string(),
                Style::default().fg(HEADING_COLOR),
            )))
            .style(Style::default().fg(TEXT_COLOR)),
            rows[1],
        );
    }

    // Coach pane: same padding, existing line styling and wrap.
    let block = pane_block(
        Line::from(Span::styled(
            "Coach",
            Style::default()
                .fg(HEADING_COLOR)
                .add_modifier(Modifier::BOLD),
        )),
        chat_focused,
    );
    let inner = block.inner(chunks[1]);
    frame.render_widget(block, chunks[1]);
    let padded = Rect {
        x: inner.x.saturating_add(2),
        width: inner.width.saturating_sub(4),
        ..inner
    };
    frame.render_widget(
        Paragraph::new(right.iter().map(|line| body_line(line)).collect::<Vec<_>>())
            .style(Style::default().fg(TEXT_COLOR))
            .wrap(ratatui::widgets::Wrap { trim: true }),
        padded,
    );
}
