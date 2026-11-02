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
pub use crate::tui::setup_panel::{draw_setup_screen, setup_box_areas, SetupPanel};

/// Pastel yellow used for the `?` help key on every screen.
pub const HELP_KEY_COLOR: Color = Color::Rgb(250, 221, 141);
/// Pastel red used for the `esc` quit key on every screen.
pub const ESC_KEY_COLOR: Color = Color::Rgb(249, 160, 160);
/// Pastel green used for the `enter` key on every screen.
pub const ENTER_KEY_COLOR: Color = Color::Rgb(168, 230, 182);

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

pub(crate) fn title_line() -> Line<'static> {
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
    let width = (longest as u16).saturating_add(4).clamp(44, 104);
    let height = (lines.len() as u16).saturating_add(2);
    centered_rect(area, width, height)
}

/// Splits a line into spans, colouring standalone key tokens:
/// `enter`, `[PASS]` and the disclosure `d` pastel green,
/// `b` and `m` pastel navy,
/// `?` pastel yellow, and `esc` pastel red. Keys inside words
/// stay white.
pub(crate) fn help_key_spans(text: &str) -> Vec<Span<'static>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut raw_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let prev_ok = |i: usize| i == 0 || matches!(bytes[i - 1], b' ' | b',' | b'(');
        let next_ok =
            |end: usize| end == bytes.len() || matches!(bytes[end], b' ' | b',' | b'.' | b')');
        let is_copy =
            text[i..].starts_with("[COPY]") && (i == 0 || bytes[i - 1] == b' ') && next_ok(i + 6);
        let is_pass =
            text[i..].starts_with("[PASS]") && (i == 0 || bytes[i - 1] == b' ') && next_ok(i + 6);
        let is_enter = text[i..].starts_with("enter") && prev_ok(i) && next_ok(i + 5);
        let is_esc = text[i..].starts_with("esc")
            && prev_ok(i)
            && (i + 3 == bytes.len() || matches!(bytes[i + 3], b' ' | b','));
        let is_back = bytes[i] == b'b' && prev_ok(i) && next_ok(i + 1);
        let is_disclosure = text[i..].starts_with("d read the disclosure") && prev_ok(i);
        let is_meta = bytes[i] == b'm' && prev_ok(i) && next_ok(i + 1);
        let is_help = bytes[i] == b'?'
            && i > 0
            && bytes[i - 1] == b' '
            && i + 1 < bytes.len()
            && bytes[i + 1] == b' ';
        if is_copy
            || is_pass
            || is_enter
            || is_esc
            || is_back
            || is_meta
            || is_disclosure
            || is_help
        {
            if raw_start < i {
                spans.push(Span::raw(text[raw_start..i].to_string()));
            }
            let (token, color, len) = if is_copy {
                ("[COPY]", ENTER_KEY_COLOR, 6)
            } else if is_pass {
                ("[PASS]", ENTER_KEY_COLOR, 6)
            } else if is_enter {
                ("enter", ENTER_KEY_COLOR, 5)
            } else if is_esc {
                ("esc", ESC_KEY_COLOR, 3)
            } else if is_back {
                ("b", PAIR_COLOR, 1)
            } else if is_meta {
                ("m", PAIR_COLOR, 1)
            } else if is_disclosure {
                ("d", ENTER_KEY_COLOR, 1)
            } else {
                ("?", HELP_KEY_COLOR, 1)
            };
            let style = if is_copy {
                Style::default()
                    .fg(ENTER_KEY_COLOR)
                    .bg(Color::Rgb(41, 52, 84))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(token, style));
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

/// Centred loading lines with animated dots (1-3, cycling), shown
/// while a background request runs so the screen never looks frozen.
pub fn loading_lines(tick: usize) -> Vec<String> {
    let dots = ".".repeat(tick % 3 + 1);
    vec![format!("working on your request, loading{dots}")]
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

/// Returns true when `(column, row)` clicks on a `[COPY]` button
/// line inside the centred panel; which string it copies is the
/// caller's business (the string is never displayed).
pub fn is_copy_button_at(lines: &[String], area: Rect, column: u16, row: u16) -> bool {
    let panel = panel_area(area, lines, true);
    for (index, line) in lines.iter().enumerate() {
        let Some(pos) = line.find("[COPY]") else {
            continue;
        };
        let button_row = panel.y.saturating_add(1).saturating_add(index as u16);
        let indent = line[..pos].chars().count() as u16;
        let button_start = panel.x.saturating_add(1).saturating_add(indent);
        let button_end = button_start.saturating_add(6);
        if row == button_row && column >= button_start && column < button_end {
            return true;
        }
    }
    false
}

/// Draws the centred loading panel; every line is soft white, with
/// no navy heading line.
pub fn draw_loading_screen(frame: &mut Frame, lines: &[String]) {
    let area = panel_area(frame.area(), lines, true);
    let text: Vec<Line> = lines
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(TEXT_COLOR))))
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
