//! Candidate solving screen: two panes in the expert-preview
//! design language. The question pane gets a bold title line,
//! navy section headings, a two-column reading padding, a pinned
//! key hint, and its own scroll offset. The Coach pane keeps its
//! own scroll offset. Coach lines stay on the left with a navy `>`.
//! Candidate lines sit on the right with a navy `<`, like a chat.
//! The green `>` input sits under a rule that stays put while the
//! transcript scrolls. Pane focus,
//! the draggable split, and every key action are unchanged — this
//! module only draws.

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::render::{
    body_line, help_key_spans, title_line, ENTER_KEY_COLOR, FRAME_COLOR, HEADING_COLOR, PAIR_COLOR,
    PROOF_COLOR, TEXT_COLOR,
};
use crate::tui::render_preview::{preview_line_kind, PreviewRowKind};
use crate::tui::viewport::{clamp_wrapped_scroll, wrapped_rows, wrapped_window};

/// Styles one question-pane line like the expert preview: the
/// first line is the bold title, section headings are navy bold,
/// bullets get a navy marker, and `selected approach:` /
/// `latest test:` lines get a navy label.
fn question_line(line: &str, first: bool) -> Line<'static> {
    if line == "Approaches" {
        return Line::from(Span::styled(
            line.to_string(),
            Style::default()
                .fg(HEADING_COLOR)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(rest) = line.strip_prefix("> ") {
        return Line::from(vec![
            Span::styled(
                "> ".to_string(),
                Style::default()
                    .fg(PROOF_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                rest.to_string(),
                Style::default().fg(TEXT_COLOR).add_modifier(Modifier::BOLD),
            ),
        ]);
    }
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
            if let Some(rest) = line.strip_prefix("status:") {
                return Line::from(vec![
                    Span::styled(
                        "status:".to_string(),
                        Style::default()
                            .fg(HEADING_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(rest.to_string()),
                ]);
            }
            body_line(line)
        }
    }
}

/// One coach-pane row. The coach stays on the left with a navy
/// `>`. The candidate sits on the right with a navy `<`. The
/// message text itself is white.
fn coach_bubble(line: &str) -> Line<'static> {
    if let Some(text) = line.strip_prefix("< ") {
        return Line::from(vec![
            Span::styled(
                "< ".to_string(),
                Style::default()
                    .fg(HEADING_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(text.to_string(), Style::default().fg(TEXT_COLOR)),
        ])
        .alignment(Alignment::Right);
    }
    if let Some(text) = line.strip_prefix("> ") {
        return Line::from(vec![
            Span::styled(
                "> ".to_string(),
                Style::default()
                    .fg(HEADING_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(text.to_string(), Style::default().fg(TEXT_COLOR)),
        ]);
    }
    body_line(line)
}

/// Splits the question pane into the scrolling body, an optional
/// work banner (`working:` / `ready:`), and the pinned key hint.
fn split_question_footer(lines: &[String]) -> (&[String], Option<&str>, Option<&str>) {
    let Some(last) = lines.last().map(String::as_str) else {
        return (lines, None, None);
    };
    if !last.starts_with("focus:") {
        return (lines, None, None);
    }
    let rest = &lines[..lines.len() - 1];
    if let Some(banner) = rest.last().map(String::as_str) {
        if banner.starts_with("working:") || banner.starts_with("ready:") {
            return (&rest[..rest.len() - 1], Some(banner), Some(last));
        }
    }
    (rest, None, Some(last))
}

/// The line pinned above the key hint while code is written, and
/// after it lands. `working:` is green so the wait is obvious.
fn work_banner_line(banner: &str) -> Line<'static> {
    let (working, text) = if let Some(text) = banner.strip_prefix("working:") {
        (true, text.trim_start())
    } else if let Some(text) = banner.strip_prefix("ready:") {
        (false, text.trim_start())
    } else {
        (false, banner)
    };
    let color = if working { ENTER_KEY_COLOR } else { TEXT_COLOR };
    Line::from(Span::styled(
        text.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

/// Key hint pinned under the question pane. The `focus:` prefix is
/// only a marker; the keys are coloured the same way as every
/// other screen's footer.
fn hint_line(footer: &str) -> Line<'static> {
    let text = footer.strip_prefix("focus:").unwrap_or(footer).trim_start();
    Line::from(help_key_spans(text))
}

/// Coach input, pinned under the transcript. The green `>` stands
/// in for the old `message:` label.
fn coach_input_line(footer: &str) -> Line<'static> {
    let text = footer
        .strip_prefix("message:")
        .unwrap_or(footer)
        .trim_start();
    Line::from(vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(ENTER_KEY_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(text.to_string(), Style::default().fg(TEXT_COLOR)),
    ])
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
/// question pane's key hint and the Coach pane's input, with the
/// rule above it, are each pinned to the bottom of their pane.
pub fn draw_candidate_screen(
    frame: &mut Frame,
    left: &[String],
    right: &[String],
    chat_focused: bool,
    question_pct: u16,
    question_scroll: usize,
    coach_scroll: usize,
) {
    draw_candidate_screen_with_status(
        frame,
        left,
        &[],
        right,
        chat_focused,
        question_pct,
        question_scroll,
        coach_scroll,
    )
}

/// Like [`draw_candidate_screen`], with a fixed status box above the
/// Coach pane: always `status lines + border` tall, never resizable,
/// showing the submitted approach, latest test, phase, and sync.
#[allow(clippy::too_many_arguments)]
pub fn draw_candidate_screen_with_status(
    frame: &mut Frame,
    left: &[String],
    status: &[String],
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

    // Question pane: pinned key hint, padded scrollable body.
    let block = pane_block(title_line(), !chat_focused);
    let inner = block.inner(chunks[0]);
    frame.render_widget(block, chunks[0]);
    let padded = Rect {
        x: inner.x.saturating_add(2),
        width: inner.width.saturating_sub(4),
        ..inner
    };
    let (content_lines, banner_line, footer_line) = split_question_footer(left);
    let mut constraints = vec![ratatui::layout::Constraint::Min(0)];
    if banner_line.is_some() {
        constraints.push(ratatui::layout::Constraint::Length(1));
    }
    if footer_line.is_some() {
        constraints.push(ratatui::layout::Constraint::Length(1));
    }
    let rows = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(constraints)
        .split(padded);
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
    let mut foot = 1usize;
    if let Some(banner) = banner_line {
        frame.render_widget(
            Paragraph::new(work_banner_line(banner)).style(Style::default().fg(TEXT_COLOR)),
            rows[foot],
        );
        foot += 1;
    }
    if let Some(footer) = footer_line {
        frame.render_widget(
            Paragraph::new(hint_line(footer)).style(Style::default().fg(TEXT_COLOR)),
            rows[foot],
        );
    }

    // Right column: a fixed-height status box (never resizable) on
    // top, the Coach pane filling the rest below it.
    let right_rows = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(if status.is_empty() {
                0
            } else {
                status.len() as u16 + 2
            }),
            ratatui::layout::Constraint::Min(0),
        ])
        .split(chunks[1]);
    if !status.is_empty() {
        let status_block = Block::default()
            .title(Line::from(Span::styled(
                "Status",
                Style::default()
                    .fg(HEADING_COLOR)
                    .add_modifier(Modifier::BOLD),
            )))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(FRAME_COLOR));
        let status_inner = status_block.inner(right_rows[0]);
        frame.render_widget(status_block, right_rows[0]);
        let status_lines: Vec<Line> = status
            .iter()
            .map(|line| match line.split_once(':') {
                Some((label, rest)) => Line::from(vec![
                    Span::styled(
                        format!("{label}:"),
                        Style::default()
                            .fg(HEADING_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(rest.to_string()),
                ]),
                None => body_line(line),
            })
            .collect();
        frame.render_widget(
            Paragraph::new(status_lines).style(Style::default().fg(TEXT_COLOR)),
            status_inner,
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
    let inner = block.inner(right_rows[1]);
    frame.render_widget(block, right_rows[1]);
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
                .map(|(_, line)| coach_bubble(line))
                .collect::<Vec<_>>(),
        )
        .style(Style::default().fg(TEXT_COLOR))
        .wrap(ratatui::widgets::Wrap { trim: true })
        .scroll((c_local.min(u16::MAX as usize) as u16, 0)),
        coach_rows[0],
    );
    if let Some(footer) = coach_footer {
        let rule = "\u{2500}".repeat(usize::from(coach_rows[1].width));
        frame.render_widget(
            Paragraph::new(rule).style(Style::default().fg(FRAME_COLOR)),
            coach_rows[1],
        );
        frame.render_widget(
            Paragraph::new(coach_input_line(footer)).style(Style::default().fg(TEXT_COLOR)),
            coach_rows[2],
        );
    }
}
