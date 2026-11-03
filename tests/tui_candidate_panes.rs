//! Candidate solving-pane geometry and Connect loading rules:
//! split defaults/clamps, divider hit-testing, the deferred
//! connect submit, and which screens centre before going live.

use pairproof::tui::candidate::CandidateScreen;
use pairproof::tui::candidate_runtime::{
    connect_submit_shows_loading, divider_hit, screen_centered, split_pct_for_column,
    DEFAULT_QUESTION_PCT,
};

#[test]
fn split_defaults_sixty_forty_and_clamps_both_panes_to_thirty() {
    assert_eq!(DEFAULT_QUESTION_PCT, 60);
    assert_eq!(split_pct_for_column(60, 100), 60);
    assert_eq!(split_pct_for_column(10, 100), 30);
    assert_eq!(split_pct_for_column(95, 100), 70);
    assert_eq!(split_pct_for_column(0, 0), 60);
}

#[test]
fn connect_submit_shows_loading_only_for_nonempty_connect_enter() {
    assert!(connect_submit_shows_loading(
        CandidateScreen::Connect,
        12,
        false
    ));
    assert!(!connect_submit_shows_loading(
        CandidateScreen::Connect,
        0,
        false
    ));
    assert!(!connect_submit_shows_loading(
        CandidateScreen::Connect,
        12,
        true
    ));
    assert!(!connect_submit_shows_loading(
        CandidateScreen::Rules,
        12,
        false
    ));
}

#[test]
fn divider_hit_covers_the_border_column() {
    assert!(divider_hit(60, 100, 60));
    assert!(divider_hit(59, 100, 60));
    assert!(divider_hit(61, 100, 60));
    assert!(!divider_hit(50, 100, 60));
}

#[test]
fn connect_and_rules_centre_before_live() {
    assert!(screen_centered(false, false, CandidateScreen::Connect));
    assert!(screen_centered(false, false, CandidateScreen::Rules));
    assert!(!screen_centered(false, false, CandidateScreen::Waiting));
}

#[test]
fn live_screens_use_full_area_unless_help_is_open() {
    assert!(!screen_centered(false, true, CandidateScreen::Rules));
    assert!(!screen_centered(false, true, CandidateScreen::Solving));
    assert!(screen_centered(true, true, CandidateScreen::Solving));
}

// --- Question pane design (shared with the expert preview) ---

use pairproof::tui::render::draw_candidate_screen;
use pairproof::tui::render::HEADING_COLOR;

fn long_left_lines() -> Vec<String> {
    let mut lines = vec![
        "Q1: Two Sum".to_string(),
        "Given an array of integers nums and an integer target, return indices.".to_string(),
        String::new(),
        "Example 1:".to_string(),
        "Input: nums = [2,7,11,15], target = 9".to_string(),
        "Output: [0,1]".to_string(),
    ];
    for i in 0..20 {
        lines.push(format!("filler line {i} explaining the approach in detail"));
    }
    lines.push("Constraints:".to_string());
    lines.push("- 2 <= nums.length <= 10000".to_string());
    lines.push("selected approach: none".to_string());
    lines.push("focus: work/other/redirect/chat (tab switches)".to_string());
    lines
}

fn drawn(scroll: usize, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let left = long_left_lines();
    let right = vec!["coach: hello".to_string(), "message: ".to_string()];
    terminal
        .draw(|frame| draw_candidate_screen(frame, &left, &right, false, 60, scroll))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let area = *buffer.area();
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn candidate_question_pane_has_padding_headings_and_pinned_footer() {
    let buffer = drawn(0, 80, 24);
    let text = buffer_text(&buffer);
    // The focus hint is pinned to the bottom row of the pane even
    // though the statement is much longer than the screen.
    let area = *buffer.area();
    let last_inner_row = (0..area.width)
        .map(|x| buffer[(x, area.height - 2)].symbol())
        .collect::<String>();
    assert!(last_inner_row.contains("focus:"), "{last_inner_row}");
    // Title starts after border + two-column padding.
    let title_row = text
        .lines()
        .find(|line| line.contains("Q1:"))
        .expect("title row");
    assert_eq!(
        title_row.split("Q1:").next().unwrap_or("").chars().count(),
        3,
        "{title_row}"
    );
    // Section headings use the shared navy heading colour.
    let area = *buffer.area();
    let mut heading_found = false;
    'outer: for y in 0..area.height {
        for x in 0..area.width {
            if buffer[(x, y)].fg == HEADING_COLOR && buffer[(x, y)].symbol() == "E" {
                let word: String = (0..7).map(|i| buffer[(x + i, y)].symbol()).collect();
                if word == "Example" {
                    heading_found = true;
                    break 'outer;
                }
            }
        }
    }
    assert!(heading_found, "Example heading must be navy");
}

#[test]
fn candidate_question_pane_scrolls_and_keeps_footer() {
    let buffer = drawn(45, 80, 24);
    let text = buffer_text(&buffer);
    assert!(text.contains("Constraints:"), "{text}");
    assert!(!text.contains("Given an array"), "{text}");
    let area = *buffer.area();
    let last_inner_row = (0..area.width)
        .map(|x| buffer[(x, area.height - 2)].symbol())
        .collect::<String>();
    assert!(last_inner_row.contains("focus:"), "{last_inner_row}");
}
