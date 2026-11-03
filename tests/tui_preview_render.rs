//! Question preview renderer: wrapping, section classification,
//! pinned footer, and full-width layout on a test backend.

use pairproof::tui::render::{
    draw_question_preview_screen, preview_body_rows, preview_metrics, PreviewRowKind,
    QuestionPreviewData,
};

fn sample_preview() -> QuestionPreviewData {
    QuestionPreviewData {
        title: "Two Sum".to_string(),
        difficulty: "medium".to_string(),
        acceptance: "52%".to_string(),
        slug: "two-sum".to_string(),
        position: 3,
        total: 20,
        picked: true,
        body: Some("Given an array of integers nums and an integer target, return indices of the two numbers such that they add up to target using every element exactly once.\n\nExample 1:\nInput: nums = [2,7,11,15], target = 9\nOutput: [0,1]\nExplanation: Because nums[0] + nums[1] == 9, we return [0, 1].\n\nConstraints:\n- 2 <= nums.length <= 10000\n- Only one valid answer exists and it must use each element at most once in the final returned pair of indices.".to_string()),
    }
}

fn drawn_text(preview: &QuestionPreviewData, scroll: usize, width: u16, height: u16) -> String {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| draw_question_preview_screen(frame, preview, scroll))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let area = *buffer.area();
    let mut rows = Vec::new();
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buffer[(x, y)].symbol());
        }
        rows.push(row.trim_end().to_string());
    }
    rows.join("\n")
}

#[test]
fn preview_body_rows_classify_sections_and_wrap_long_lines() {
    let rows = preview_body_rows(
        "A very long question sentence that must wrap inside a narrow preview pane.\n\nExample 1:\nInput: nums = [2,7]\n\nConstraints:\n- 2 <= nums.length <= 10",
        32,
    );
    assert!(rows.len() > 6, "{rows:?}");
    assert!(rows.iter().all(|row| row.text.chars().count() <= 32));
    assert!(rows
        .iter()
        .any(|row| row.kind == PreviewRowKind::Heading && row.text == "Example 1:"));
    assert!(rows
        .iter()
        .any(|row| row.kind == PreviewRowKind::Heading && row.text == "Constraints:"));
    assert!(rows
        .iter()
        .any(|row| row.kind == PreviewRowKind::Bullet && row.text.starts_with("- ")));
}

#[test]
fn preview_wraps_question_and_example_and_pins_footer() {
    let text = drawn_text(&sample_preview(), 0, 60, 20);
    assert!(text.contains("Question preview"), "{text}");
    assert!(text.contains("Two Sum"), "{text}");
    assert!(text.contains("difficulty"), "{text}");
    assert!(text.contains("Constraints:"), "{text}");
    assert!(text.contains("back to list"), "{text}");
    assert!(text.contains("3 of 20"), "{text}");
}

#[test]
fn preview_scrolled_to_end_still_shows_footer() {
    let preview = sample_preview();
    let metrics = preview_metrics(&preview, 60, 14);
    assert!(metrics.max_scroll > 0, "{metrics:?}");
    let text = drawn_text(&preview, metrics.max_scroll, 60, 14);
    assert!(text.contains("back to list"), "{text}");
    assert!(text.contains("Only one valid answer"), "{text}");
}

#[test]
fn preview_uses_the_full_terminal_width() {
    let preview = sample_preview();
    let backend = ratatui::backend::TestBackend::new(140, 20);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| draw_question_preview_screen(frame, &preview, 0))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let area = *buffer.area();
    let mut longest = 0;
    let mut given_at = None;
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buffer[(x, y)].symbol());
        }
        if row.contains("Given") {
            given_at = Some(row.split("Given").next().unwrap_or("").chars().count());
        }
        longest = longest.max(row.trim_end().chars().count());
    }
    // No centred column: body text starts right inside the
    // border and wrapped rows run past the old 104-column cap.
    assert_eq!(given_at, Some(1), "body must start at the left edge");
    assert!(longest > 110, "rows must use the full width: {longest}");
    let text: String = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("left/right"), "{text}");
}
