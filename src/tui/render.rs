//! Shared terminal rendering: pastel navy/purple panel and layout.
//! Both TTY loops draw through here so colours and framing stay
//! identical on expert and candidate screens.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

/// Pastel navy used for the outer frame, one shade deeper so it
/// sits comfortably next to the pastel purple.
pub const FRAME_COLOR: Color = Color::Rgb(102, 138, 208);
/// Pastel navy used for the `Pair` half of the brand title.
pub const PAIR_COLOR: Color = Color::Rgb(120, 165, 230);
/// Pastel purple used for the `Proof` half of the brand title.
/// Purple is reserved for `Proof` only; everything else is navy.
pub const PROOF_COLOR: Color = Color::Rgb(196, 181, 253);
/// Soft white used for body text.
pub const TEXT_COLOR: Color = Color::Rgb(237, 242, 255);
/// Pastel navy used for the first (heading) line of a panel.
pub const HEADING_COLOR: Color = Color::Rgb(120, 165, 230);
/// Pastel yellow used for the `?` help key on every screen.
pub const HELP_KEY_COLOR: Color = Color::Rgb(250, 221, 141);
/// Pastel red used for the `esc` quit key on every screen.
pub const ESC_KEY_COLOR: Color = Color::Rgb(249, 160, 160);

fn brand_line() -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "Pair",
            Style::default().fg(PAIR_COLOR).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Proof",
            Style::default()
                .fg(PROOF_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn title_line() -> Line<'static> {
    brand_line()
}

/// Returns a rectangle of at most `max_width` x `max_height`, centred
/// inside `area`. On small terminals it shrinks to fit `area`.
pub fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.min(max_width);
    let height = area.height.min(max_height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Picks the panel area: full area, or a centred compact rectangle
/// sized to the content when `centered` is set (main menu).
pub fn panel_area(area: Rect, lines: &[String], centered: bool) -> Rect {
    if !centered {
        return area;
    }
    let longest = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let width = (longest as u16).saturating_add(4).clamp(44, 92);
    let height = (lines.len() as u16).saturating_add(2);
    centered_rect(area, width, height)
}

/// Splits a line into spans, colouring a standalone `?` help key
/// pastel yellow and a standalone `esc` quit key pastel red.
/// Question marks and words inside sentences stay white.
fn help_key_spans(text: &str) -> Vec<Span<'static>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut raw_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let prev_ok = |i: usize| i == 0 || matches!(bytes[i - 1], b' ' | b',');
        let is_esc = text[i..].starts_with("esc")
            && prev_ok(i)
            && (i + 3 == bytes.len() || matches!(bytes[i + 3], b' ' | b','));
        let is_help = bytes[i] == b'?'
            && i > 0
            && bytes[i - 1] == b' '
            && i + 1 < bytes.len()
            && bytes[i + 1] == b' ';
        if is_esc || is_help {
            if raw_start < i {
                spans.push(Span::raw(text[raw_start..i].to_string()));
            }
            let (token, color, len) = if is_esc {
                ("esc", ESC_KEY_COLOR, 3)
            } else {
                ("?", HELP_KEY_COLOR, 1)
            };
            spans.push(Span::styled(
                token,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ));
            i += len;
            raw_start = i;
        } else {
            i += text[i..].chars().next().map_or(1, |c| c.len_utf8());
        }
    }
    if raw_start < text.len() {
        spans.push(Span::raw(text[raw_start..].to_string()));
    }
    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }
    spans
}

fn plain_line(line: &str) -> Line<'static> {
    Line::from(help_key_spans(line))
}

fn key_span(key: &str, color: Color) -> Span<'static> {
    Span::styled(
        key.to_string(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// Styles one body line: role keys keep the brand colours (`e` is
/// purple like `Proof`, `c` is navy like `Pair`), the rest stays white.
fn body_line(line: &str) -> Line<'static> {
    if let Some(rest) = line.strip_prefix("  e  ") {
        return Line::from(vec![
            Span::raw("  "),
            key_span("e", PROOF_COLOR),
            Span::raw(format!("  {rest}")),
        ]);
    }
    if let Some(rest) = line.strip_prefix("  c  ") {
        return Line::from(vec![
            Span::raw("  "),
            key_span("c", PAIR_COLOR),
            Span::raw(format!("  {rest}")),
        ]);
    }
    if let Some(rest) = line.strip_prefix("e expert, c candidate") {
        let mut spans = vec![
            key_span("e", PROOF_COLOR),
            Span::raw(" expert, "),
            key_span("c", PAIR_COLOR),
        ];
        spans.extend(help_key_spans(&format!(" candidate{rest}")));
        return Line::from(spans);
    }
    if let Some(rest) = line.strip_prefix("  e expert") {
        return Line::from(vec![
            Span::raw("  "),
            key_span("e", PROOF_COLOR),
            Span::raw(format!(" expert{rest}")),
        ]);
    }
    if let Some(rest) = line.strip_prefix("  c candidate") {
        return Line::from(vec![
            Span::raw("  "),
            key_span("c", PAIR_COLOR),
            Span::raw(format!(" candidate{rest}")),
        ]);
    }
    if let Some(rest) = line.strip_prefix("  pairproof ") {
        let full = format!("pairproof {rest}");
        for command in [
            "pairproof uninstall --keep-data",
            "pairproof update --check",
            "pairproof uninstall",
            "pairproof update",
        ] {
            if let Some(tail) = full.strip_prefix(command) {
                return Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        command.to_string(),
                        Style::default()
                            .fg(PROOF_COLOR)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(tail.to_string()),
                ]);
            }
        }
        return Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(PROOF_COLOR),
        ));
    }
    plain_line(line)
}

/// Content for the Session Setup screen's two text boxes.
pub struct SetupPanel<'a> {
    pub job_description: &'a str,
    pub additional_notes: &'a str,
    pub job_focused: bool,
    pub job_scroll: usize,
    pub notes_scroll: usize,
    pub status: Option<&'a str>,
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
    ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(job_height + 2),
            ratatui::layout::Constraint::Length(notes_height + 2),
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Length(1),
        ])
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
    let hint = if panel.job_focused {
        "Paste the job description, then press Enter."
    } else {
        "Additional notes are optional. Enter reviews the proposal."
    };
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().fg(TEXT_COLOR)),
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
            "(Enter skips)",
            width,
            panel.notes_height as usize,
            panel.notes_scroll,
        ),
    );
    if let Some(status) = panel.status {
        frame.render_widget(
            Paragraph::new(format!("Status: {status}")).style(Style::default().fg(TEXT_COLOR)),
            chunks[4],
        );
    }
    let footer = if panel.job_focused {
        "Up/Down scroll, drag bottom border to resize, Enter continue, b back, esc quit"
    } else {
        "Up/Down scroll, drag bottom border to resize, Enter review, b back, esc quit"
    };
    frame.render_widget(
        Paragraph::new(Line::from(help_key_spans(footer))).style(Style::default().fg(TEXT_COLOR)),
        chunks[5],
    );
}

/// Draws `lines` inside a bordered panel. The frame title is
/// always the `PairProof` brand. The first line renders as a
/// pastel-navy heading, the rest as soft white.
pub fn draw_screen(frame: &mut Frame, lines: &[String], centered: bool) {
    let area = panel_area(frame.area(), lines, centered);
    let text: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(Span::styled(
                    line.clone(),
                    Style::default()
                        .fg(HEADING_COLOR)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                body_line(line)
            }
        })
        .collect();
    let widget = Paragraph::new(text)
        .style(Style::default().fg(TEXT_COLOR))
        .block(
            Block::default()
                .title(title_line())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(FRAME_COLOR)),
        );
    frame.render_widget(widget, area);
}
