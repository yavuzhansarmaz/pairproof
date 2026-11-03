//! Candidate solving screen: two panes in the expert-preview
//! design language. The question pane gets a bold title line,
//! navy section headings, a two-column reading padding, a pinned
//! focus footer, and its own scroll offset; the Coach pane keeps
//! its existing rendering with its own independent scroll offset
//! and a pinned `message:` footer. Pane focus, the draggable
//! split, and every key action are unchanged — this module only
//! draws.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::render::{
    body_line, title_line, FRAME_COLOR, HEADING_COLOR, PAIR_COLOR, PROOF_COLOR, TEXT_COLOR,
};
use crate::tui::render_preview::{preview_line_kind, PreviewRowKind};
use crate::tui::viewport::{clamp_wrapped_scroll, wrapped_rows, wrapped_window};

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

/// Visible window of `lines`: `(global_index, line)` pairs from
/// the window start covering `local_scroll + visible + 1` rows,
/// plus the local scroll to apply to the Paragraph.
fn window_lines(
    lines: &[String],
    width: usize,
    scroll: usize,
    visible: usize,
) -> (Vec<(usize, &String)>, usize) {
    let (start, local) = wrapped_window(lines, width, scroll, visible);
    let need = local + visible + 1;
    let mut taken = Vec::new();
    let mut covered = 0usize;
    for (offset, line) in lines.iter().skip(start).enumerate() {
        if covered >= need && !taken.is_empty() {
            break;
        }
        covered += wrapped_rows(line, width);
        taken.push((start + offset, line));
    }
    (taken, local)
}

/// Draws the candidate solving screen as two panes: question and
/// approaches on the left, the Coach chat on the right, split at
/// `question_pct` percent (draggable in the TTY loop, 30..=70). The
/// focused pane's border uses the purple accent, the other the
/// frame navy. The question pane scrolls by `question_scroll`
/// and the Coach pane by `coach_scroll`, independently; both are
/// clamped to the content so overscroll never blanks a pane. The
/// question pane's `focus:` hint line and the Coach pane's
/// `message:` line are each pinned to the bottom of their pane.
pub fn draw_candidate_screen(
    frame: &mut Frame,
    left: &[String],
    right: &[String],
    chat_focused: bool,
    question_pct: u16,
    question_scroll: usize,
    coach_scroll: usize,
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
    let q_width = usize::from(rows[0].width);
    let q_visible = usize::from(rows[0].height);
    let effective_question =
        clamp_wrapped_scroll(content_lines, q_width, question_scroll, q_visible);
    let (q_window, q_local) = window_lines(content_lines, q_width, effective_question, q_visible);
    let content: Vec<Line> = q_window
        .iter()
        .map(|(index, line)| question_line(line, *index == 0))
        .collect();
    frame.render_widget(
        Paragraph::new(content)
            .style(Style::default().fg(TEXT_COLOR))
            .wrap(ratatui::widgets::Wrap { trim: true })
            .scroll((q_local.min(u16::MAX as usize) as u16, 0)),
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
    let (coach_content, coach_footer): (&[String], Option<&str>) =
        match right.last().map(String::as_str) {
            Some(last) if last.starts_with("message:") => (&right[..right.len() - 1], Some(last)),
            _ => (right, None),
        };
    let coach_rows = if coach_footer.is_some() {
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
    let c_width = usize::from(coach_rows[0].width);
    let c_visible = usize::from(coach_rows[0].height);
    let effective_coach = clamp_wrapped_scroll(coach_content, c_width, coach_scroll, c_visible);
    let (c_window, c_local) = window_lines(coach_content, c_width, effective_coach, c_visible);
    frame.render_widget(
        Paragraph::new(
            c_window
                .iter()
                .map(|(_, line)| body_line(line))
                .collect::<Vec<_>>(),
        )
        .style(Style::default().fg(TEXT_COLOR))
        .wrap(ratatui::widgets::Wrap { trim: true })
        .scroll((c_local.min(u16::MAX as usize) as u16, 0)),
        coach_rows[0],
    );
    if let Some(footer) = coach_footer {
        frame.render_widget(
            Paragraph::new(body_line(footer)).style(Style::default().fg(TEXT_COLOR)),
            coach_rows[1],
        );
    }
}
