//! Session Setup screen rendering: the two scrollable text boxes.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::render::{
    help_key_spans, title_line, FRAME_COLOR, HEADING_COLOR, PROOF_COLOR, TEXT_COLOR,
};

/// Content for the Session Setup screen's two text boxes.
pub struct SetupPanel<'a> {
    pub job_description: &'a str,
    pub additional_notes: &'a str,
    pub job_focused: bool,
    pub job_scroll: usize,
    pub notes_scroll: usize,
    pub job_height: u16,
    pub notes_height: u16,
}

/// Returns the (job, notes) box rectangles for a terminal area and
/// the given visible-line heights, matching `draw_setup_screen`.
pub fn setup_box_areas(area: Rect, job_height: u16, notes_height: u16) -> (Rect, Rect) {
    let chunks = setup_chunks(area, job_height, notes_height);
    (chunks[2], chunks[3])
}

fn setup_chunks(area: Rect, job_height: u16, notes_height: u16) -> Vec<Rect> {
    let outer = Block::default().borders(Borders::ALL);
    let inner = outer.inner(area);
    let constraints = vec![
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(job_height + 2),
        ratatui::layout::Constraint::Length(notes_height + 2),
        ratatui::layout::Constraint::Length(1),
    ];
    ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints(constraints)
        .split(inner)
        .to_vec()
}

fn wrapped_lines(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    // Strip CR/control bytes first: a pasted CRLF text rendered raw
    // overwrites itself on the terminal and garbles words.
    let clean = crate::tui::shared::sanitize_display(text).replace('\t', "    ");
    let mut out = Vec::new();
    for line in clean.split('\n') {
        if line.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            let word_len = word.chars().count();
            if word_len > width {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                let chars: Vec<char> = word.chars().collect();
                for chunk in chars.chunks(width) {
                    out.push(chunk.iter().collect());
                }
                continue;
            }
            if current.is_empty() {
                current = word.to_string();
            } else if current.chars().count() + 1 + word_len <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                out.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn visible_box_lines(
    text: &str,
    placeholder: &str,
    width: usize,
    height: usize,
    scroll: usize,
) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![placeholder.to_string()];
    }
    let lines = wrapped_lines(text, width);
    let start = scroll.min(lines.len().saturating_sub(1));
    lines.iter().skip(start).take(height).cloned().collect()
}

fn text_box(frame: &mut Frame, area: Rect, title: &str, focused: bool, lines: Vec<String>) {
    let border = if focused { PROOF_COLOR } else { FRAME_COLOR };
    let style = Style::default().fg(TEXT_COLOR);
    let widget = Paragraph::new(lines.join("\n")).style(style).block(
        Block::default()
            .title(Span::styled(
                title.to_string(),
                Style::default().fg(border).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border)),
    );
    frame.render_widget(widget, area);
}

/// Draws the Session Setup screen at full size: a heading, the
/// job-description box (5 visible lines) and the additional-notes
/// box (2 visible lines), each scrollable, then status and footer.
pub fn draw_setup_screen(frame: &mut Frame, panel: &SetupPanel) {
    let area = frame.area();
    let outer = Block::default()
        .title(title_line())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(FRAME_COLOR));
    frame.render_widget(outer, area);
    let chunks = setup_chunks(area, panel.job_height, panel.notes_height);
    let heading = Paragraph::new("Session setup").style(
        Style::default()
            .fg(HEADING_COLOR)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_widget(heading, chunks[0]);
    let hint = "PairProof turns the pasted role into interview topics, level, and language.";
    frame.render_widget(
        Paragraph::new(Line::from(help_key_spans(hint))).style(Style::default().fg(TEXT_COLOR)),
        chunks[1],
    );
    let width = chunks[2].width.saturating_sub(2) as usize;
    text_box(
        frame,
        chunks[2],
        " Job description (required) ",
        panel.job_focused,
        visible_box_lines(
            panel.job_description,
            "(paste the job description here)",
            width,
            panel.job_height as usize,
            panel.job_scroll,
        ),
    );
    text_box(
        frame,
        chunks[3],
        " Additional notes (optional) ",
        !panel.job_focused,
        visible_box_lines(
            panel.additional_notes,
            "(enter skips)",
            width,
            panel.notes_height as usize,
            panel.notes_scroll,
        ),
    );
    let footer = if panel.job_focused {
        "up/down scroll, drag bottom border to resize, enter continue, b back, esc quit"
    } else {
        "up/down scroll, drag bottom border to resize, enter review, b back, esc quit"
    };
    frame.render_widget(
        Paragraph::new(Line::from(help_key_spans(footer))).style(Style::default().fg(TEXT_COLOR)),
        chunks[chunks.len() - 1],
    );
}
