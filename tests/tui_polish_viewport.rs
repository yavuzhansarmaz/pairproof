use std::collections::BTreeMap;

use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::orchestration::control::{SnapshotFinal, SnapshotFinalQuestion};
use pairproof::scoring::{GradeOutcome, QuestionGrade};
use pairproof::tui::candidate_final::{
    candidate_final, candidate_final_lines, candidate_final_lines_for_height,
    candidate_final_pinned_lines, CandidateFinal,
};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::monitor::FinalView;
use pairproof::tui::render::{
    centered_rect, panel_area, ESC_KEY_COLOR, HELP_KEY_COLOR, PAIR_COLOR, PROOF_COLOR,
};
use pairproof::tui::viewport::{
    is_pinned_summary, split_pinned, truncation_marker, visible_lines, visible_with_pinned,
};

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: format!("Title {slug}"),
        difficulty,
        acceptance_pct: None,
    }
}

fn qs(prefix: &str, n: usize, difficulty: Difficulty) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| q(&format!("{prefix}{i}"), difficulty))
        .collect()
}

fn begun() -> Session {
    let mut s = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    s.mark_waiting_for_candidate(2).expect("waiting");
    s.candidate_connected(3).expect("connected");
    s.start(4).expect("start");
    s
}

fn completed() -> Session {
    let mut session = begun();
    session
        .set_expert_questions(
            qs("m", 3, Difficulty::Medium),
            qs("h", 2, Difficulty::Hard),
            5,
        )
        .expect("expert questions");
    for i in 0..10u64 {
        session.finish_question(10 + i).expect("finish");
    }
    session.mark_completed(100).expect("completed");
    session
}

fn graded(slot: usize, grade: f64) -> GradeOutcome {
    GradeOutcome::Graded(
        QuestionGrade::new(slot, grade, format!("rationale {slot}"), 0.2).expect("grade"),
    )
}

#[test]
fn final_rating_remains_visible_on_short_terminal() {
    let session = completed();
    let result =
        candidate_final(&session, &BTreeMap::new(), &[graded(0, 9.0)]).expect("candidate final");
    let CandidateFinal::Breakdown(view) = &result else {
        panic!("expected breakdown");
    };
    let rating_line = format!("final rating: {:.2}", view.rating);
    for height in [12_usize, 8, 5] {
        let visible = candidate_final_lines_for_height(&result, height);
        assert!(visible.len() <= height, "{visible:?}");
        assert!(visible.contains(&rating_line), "h={height}: {visible:?}");
        assert!(
            visible.iter().any(|l| l.contains("more lines")),
            "h={height}: {visible:?}"
        );
    }
    let full = candidate_final_lines(&result);
    assert!(full.len() > 12, "{full:?}");
    let visible = visible_with_pinned(&full, 8);
    assert!(visible.contains(&rating_line), "{visible:?}");
}

#[test]
fn long_statement_does_not_hide_final_summary() {
    let statement = (0..60)
        .map(|i| format!("Statement line {i} with enough detail to fill rows"))
        .collect::<Vec<_>>()
        .join("\n");
    let view = CandidateView {
        phase: "Completed".to_string(),
        sync_phase: String::new(),
        question_title: Some("Big problem".to_string()),
        question_index: Some(0),
        statement: Some(statement),
        options: Vec::new(),
        selected: None,
        latest_test: None,
        chat_replies: Vec::new(),
        final_breakdown: Some(SnapshotFinal {
            rating: 8.25,
            per_question: vec![SnapshotFinalQuestion {
                slot_index: 0,
                grade: Some(8.0),
                rationale: Some("Solid".to_string()),
            }],
        }),
        recovery: None,
    };
    let lines = view.lines();
    let visible = visible_with_pinned(&lines, 12);
    assert!(visible.len() <= 12, "{visible:?}");
    assert!(
        visible.iter().any(|l| l.contains("final rating: 8.25")),
        "{visible:?}"
    );
    assert!(
        visible.iter().any(|l| l.contains("more lines")),
        "{visible:?}"
    );
    let content: Vec<String> = (0..60).map(|i| format!("line {i}")).collect();
    let pinned = vec!["final rating: 8.25".to_string()];
    let visible = visible_lines(&content, 6, &pinned);
    assert_eq!(visible.len(), 6);
    assert_eq!(
        visible.last().map(String::as_str),
        Some("final rating: 8.25")
    );
}

#[test]
fn terminal_resize_preserves_usable_layout() {
    let session = completed();
    let result =
        candidate_final(&session, &BTreeMap::new(), &[graded(0, 9.0)]).expect("candidate final");
    let full = candidate_final_lines(&result);
    let rating_line = full
        .iter()
        .find(|l| l.contains("final rating:"))
        .expect("rating line")
        .clone();
    for height in [50_usize, 24, 12, 6, 3, 2, 1, 0] {
        let visible = visible_with_pinned(&full, height);
        if height == 0 {
            assert!(visible.is_empty());
            continue;
        }
        assert!(visible.len() <= height, "h={height}: {visible:?}");
        assert!(visible.contains(&rating_line), "h={height}: {visible:?}");
    }
    let tall = visible_with_pinned(&full, 50);
    assert_eq!(tall, full);
    assert!(!tall.iter().any(|l| l.contains("more lines")), "{tall:?}");
    let shrunk = visible_with_pinned(&full, 6);
    assert!(
        shrunk.iter().any(|l| l.contains("more lines")),
        "{shrunk:?}"
    );
    let regrown = visible_with_pinned(&full, 50);
    assert_eq!(regrown, full);
    assert!(visible_lines(&[], 0, &[]).is_empty());
    assert_eq!(visible_lines(&[String::new()], 1, &[]), vec![String::new()]);
    let edge = visible_lines(&["x".repeat(500)], 2, &["final rating: 1.00".to_string()]);
    assert_eq!(edge.len(), 2);
    assert_eq!(edge[1], "final rating: 1.00");
}

#[test]
fn viewport_markers_and_pinned_split_are_exact() {
    assert_eq!(
        truncation_marker(3),
        "... 3 more lines \u{2014} scroll/resize for full text"
    );
    assert!(is_pinned_summary("  final rating: 8.25"));
    assert!(is_pinned_summary(" live rating: 8.25"));
    assert!(is_pinned_summary("   solved: 3"));
    assert!(!is_pinned_summary(
        "Given an array of integers, return the indices."
    ));
    assert!(!is_pinned_summary("  The final rating is pending."));
    let lines = vec![
        "Statement A".to_string(),
        "final rating: 8.25".to_string(),
        "Statement B".to_string(),
        "  solved: 3".to_string(),
        "live rating: 8.00".to_string(),
    ];
    let (content, pinned) = split_pinned(&lines);
    assert_eq!(
        content,
        vec!["Statement A".to_string(), "Statement B".to_string()]
    );
    assert_eq!(
        pinned,
        vec![
            "final rating: 8.25".to_string(),
            "  solved: 3".to_string(),
            "live rating: 8.00".to_string(),
        ]
    );
}

#[test]
fn centered_rect_is_centred_and_never_exceeds_the_area() {
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let rect = centered_rect(area, 76, 12);
    assert_eq!((rect.width, rect.height), (76, 12));
    assert_eq!(rect.x, (120 - 76) / 2);
    assert_eq!(rect.y, (40 - 12) / 2);
    let small = ratatui::layout::Rect::new(0, 0, 30, 8);
    let shrunk = centered_rect(small, 76, 12);
    assert_eq!((shrunk.width, shrunk.height), (30, 8));
    assert_eq!((shrunk.x, shrunk.y), (0, 0));
}

#[test]
fn panel_area_centres_only_when_asked() {
    let area = ratatui::layout::Rect::new(0, 0, 120, 40);
    let lines = vec!["Main Menu".to_string(), "Choose a role.".to_string()];
    assert_eq!(panel_area(area, &lines, false), area);
    let panel = panel_area(area, &lines, true);
    assert!(panel.width < area.width && panel.height < area.height);
    assert!(panel.x > 0 && panel.y > 0, "{panel:?}");
}

#[test]
fn main_menu_renders_as_a_centred_brand_panel() {
    use pairproof::tui::app::{screen_lines, ExpertApp};
    use pairproof::tui::render::draw_screen;
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let app = ExpertApp::new();
    let lines = screen_lines(&app);
    terminal
        .draw(|frame| draw_screen(frame, &lines, true))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains("PairProof"), "{rendered}");
    assert!(rendered.contains("Main Menu"), "{rendered}");
    // Purple is reserved for "Proof" and the expert key `e`;
    // "Pair", the "Main Menu" heading, and the candidate key `c`
    // are navy.
    let purple: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == PROOF_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(purple, "Proofee", "{purple}");
    let navy: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == PAIR_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert!(navy.contains("Pair"), "{navy}");
    assert!(navy.contains("Main Menu"), "{navy}");
    assert!(navy.contains('c'), "{navy}");
    let yellow: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == HELP_KEY_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(yellow, "?", "{yellow}");
    let red: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == ESC_KEY_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(red, "esc", "{red}");
    assert!(rendered.contains("Expert"), "{rendered}");
    assert!(rendered.contains("Candidate"), "{rendered}");
    // The panel must not fill the terminal: corners stay blank.
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(99, 29)].symbol(), " ");
    let left_border = (0..30)
        .find_map(|y| (0..100).find(|&x| buffer[(x, y)].symbol() == "\u{2502}"))
        .expect("left border");
    assert!(
        left_border > 10,
        "panel should be centred, border at {left_border}"
    );
}

#[test]
fn proposal_renders_as_a_centred_panel() {
    use pairproof::tui::app::{screen_lines, AppEvent, AppScreen, ExpertApp};
    use pairproof::tui::render::draw_screen;
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("preflight");
    app.set_disclosure_acknowledged(true);
    app.set_preflight_passed(true);
    app.set_api_key_ready(true);
    app.handle(AppEvent::PreflightCompleted).expect("setup");
    app.set_has_proposal(true);
    app.handle(AppEvent::ProposalReady).expect("proposal");
    assert_eq!(app.screen(), AppScreen::Proposal);
    app.set_proposal_lines(vec!["Language: Python".to_string()]);
    let lines = screen_lines(&app);
    terminal
        .draw(|frame| draw_screen(frame, &lines, true))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains("Proposed context"), "{rendered}");
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    let left_border = (0..30)
        .find_map(|y| (0..100).find(|&x| buffer[(x, y)].symbol() == "\u{2502}"))
        .expect("left border");
    assert!(
        left_border > 10,
        "panel should be centred, border at {left_border}"
    );
}

#[test]
fn loading_screen_renders_white_not_navy() {
    use pairproof::tui::render::{draw_loading_screen, loading_lines, HEADING_COLOR, TEXT_COLOR};
    let backend = ratatui::backend::TestBackend::new(80, 20);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let lines = loading_lines(2);
    terminal
        .draw(|frame| draw_loading_screen(frame, &lines))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let white: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == TEXT_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert!(
        white.contains("working on your request, loading..."),
        "{white}"
    );
    let navy: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == HEADING_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert!(!navy.contains("working"), "{navy}");
}

#[test]
fn copy_button_click_hit_and_miss() {
    use pairproof::tui::render::is_copy_button_at;
    use pairproof::tui::shared::base64_encode;
    assert_eq!(base64_encode("hello"), "aGVsbG8=");
    assert_eq!(base64_encode(""), "");
    // The string itself is never on screen; only the button is.
    let lines = vec![
        "Waiting for candidate".to_string(),
        "waiting for candidate to connect.".to_string(),
        "connection string: [COPY] (click to copy)".to_string(),
        "expires in 42s.".to_string(),
    ];
    let area = ratatui::layout::Rect::new(0, 0, 100, 30);
    let hit = (0..30)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .find(|(x, y)| is_copy_button_at(&lines, area, *x, *y))
        .expect("a clickable cell on the button");
    assert!(is_copy_button_at(&lines, area, hit.0, hit.1));
    assert!(!is_copy_button_at(&lines, area, 0, 0));
    assert!(!is_copy_button_at(&lines, area, hit.0, hit.1 + 1));
}

#[test]
fn loading_lines_cycle_one_to_three_dots() {
    use pairproof::tui::render::loading_lines;
    assert_eq!(loading_lines(0), vec!["working on your request, loading."]);
    assert_eq!(loading_lines(1), vec!["working on your request, loading.."]);
    assert_eq!(
        loading_lines(2),
        vec!["working on your request, loading..."]
    );
    assert_eq!(loading_lines(3), vec!["working on your request, loading."]);
}

#[test]
fn candidate_connection_renders_as_a_centred_panel() {
    use pairproof::tui::candidate::CandidateApp;
    use pairproof::tui::render::draw_screen;
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let lines = CandidateApp::new().screen_lines();
    terminal
        .draw(|frame| draw_screen(frame, &lines, true))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains("Candidate connection"), "{rendered}");
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    let left_border = (0..30)
        .find_map(|y| (0..100).find(|&x| buffer[(x, y)].symbol() == "\u{2502}"))
        .expect("left border");
    assert!(
        left_border > 10,
        "panel should be centred, border at {left_border}"
    );
}

#[test]
fn enter_is_green_and_b_and_m_are_navy_in_key_hints() {
    use pairproof::tui::render::{draw_screen, ENTER_KEY_COLOR, PAIR_COLOR};
    let backend = ratatui::backend::TestBackend::new(60, 10);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let lines = vec![
        "Keys".to_string(),
        "press enter to continue, b back, m change, esc quit, ? help".to_string(),
        "[PASS] Connection: direct connection ready".to_string(),
    ];
    terminal
        .draw(|frame| draw_screen(frame, &lines, false))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let green: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == ENTER_KEY_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert_eq!(green, "enter[PASS]", "{green}");
    let navy: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == PAIR_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert!(navy.contains('b'), "{navy}");
    assert!(navy.contains('m'), "{navy}");
}

#[test]
fn expert_setup_renders_as_a_centred_panel() {
    use pairproof::tui::app::{screen_lines, AppEvent, ExpertApp};
    use pairproof::tui::render::draw_screen;
    let backend = ratatui::backend::TestBackend::new(100, 34);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let mut app = ExpertApp::new();
    app.handle(AppEvent::SelectExpert).expect("preflight");
    app.set_preflight_lines(vec!["[PASS] Docker".to_string()]);
    app.set_api_key_status("Meta API key: configured (environment)");
    let lines = screen_lines(&app);
    terminal
        .draw(|frame| draw_screen(frame, &lines, true))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(rendered.contains("PairProof"), "{rendered}");
    assert!(rendered.contains("Expert Setup"), "{rendered}");
    assert!(rendered.contains("Meta disclosure"), "{rendered}");
    assert!(rendered.contains("[PASS] Docker"), "{rendered}");
    assert_eq!(buffer[(0, 0)].symbol(), " ");
    assert_eq!(buffer[(99, 33)].symbol(), " ");
}

#[test]
fn help_colours_tool_commands_and_role_keys() {
    use pairproof::tui::app::AppScreen;
    use pairproof::tui::help::expert_help_lines;
    use pairproof::tui::render::{draw_screen, PAIR_COLOR, PROOF_COLOR};
    let backend = ratatui::backend::TestBackend::new(100, 34);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let lines = expert_help_lines(AppScreen::RoleSelect);
    terminal
        .draw(|frame| draw_screen(frame, &lines, true))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let purple: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == PROOF_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert!(purple.contains("pairproof update"), "{purple}");
    assert!(
        purple.contains("pairproof uninstall --keep-data"),
        "{purple}"
    );
    assert!(purple.contains('e'), "{purple}");
    let navy: String = buffer
        .content
        .iter()
        .filter(|cell| cell.fg == PAIR_COLOR)
        .map(|cell| cell.symbol())
        .collect();
    assert!(navy.contains('c'), "{navy}");
}

#[test]
fn session_setup_boxes_show_five_and_two_lines() {
    use pairproof::tui::render::{draw_setup_screen, SetupPanel};
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let jd = (1..=9)
        .map(|i| format!("JD line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let notes = (1..=5)
        .map(|i| format!("Note line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    terminal
        .draw(|frame| {
            draw_setup_screen(
                frame,
                &SetupPanel {
                    job_description: &jd,
                    additional_notes: &notes,
                    job_focused: true,
                    job_scroll: 0,
                    notes_scroll: 0,
                    job_height: 5,
                    notes_height: 2,
                },
            )
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(
        rendered.contains("Job description (required)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Additional notes (optional)"),
        "{rendered}"
    );
    assert!(rendered.contains("JD line 5"), "{rendered}");
    assert!(!rendered.contains("JD line 6"), "{rendered}");
    assert!(rendered.contains("Note line 2"), "{rendered}");
    assert!(!rendered.contains("Note line 3"), "{rendered}");
    assert!(rendered.contains("paste the job description"), "{rendered}");
    assert!(
        !rendered.contains("Paste the job description"),
        "{rendered}"
    );
    // No phantom blank row: with no status, the footer sits
    // directly under the notes box.
    let rows: Vec<String> = (0..24)
        .map(|y| {
            (0..80)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        })
        .collect();
    let footer_y = rows
        .iter()
        .position(|row| row.contains("up/down scroll"))
        .expect("footer row");
    let (_, notes_box) =
        pairproof::tui::render::setup_box_areas(ratatui::layout::Rect::new(0, 0, 80, 24), 5, 2);
    assert_eq!(footer_y as u16, notes_box.y + notes_box.height);
    // Full-size screen: the outer frame fills the terminal.
    assert_eq!(buffer[(0, 0)].symbol(), "\u{250c}");
}

#[test]
fn session_setup_wraps_words_and_expands_with_height() {
    use pairproof::tui::render::{draw_setup_screen, setup_box_areas, SetupPanel};
    let backend = ratatui::backend::TestBackend::new(40, 24);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let jd = "alpha beta gamma delta epsilon zeta eta theta";
    terminal
        .draw(|frame| {
            draw_setup_screen(
                frame,
                &SetupPanel {
                    job_description: jd,
                    additional_notes: "note one\r\nnote two",
                    job_focused: true,
                    job_scroll: 0,
                    notes_scroll: 0,
                    job_height: 8,
                    notes_height: 2,
                },
            )
        })
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    // Word wrap: lines break between words, CRLF renders clean.
    assert!(rendered.contains("alpha beta"), "{rendered}");
    assert!(rendered.contains("note one"), "{rendered}");
    assert!(rendered.contains("note two"), "{rendered}");
    let (jd_box, notes_box) = setup_box_areas(ratatui::layout::Rect::new(0, 0, 40, 24), 8, 2);
    assert_eq!(jd_box.height, 10);
    assert_eq!(notes_box.height, 4);
    assert_eq!(notes_box.y, jd_box.y + jd_box.height);
}

#[test]
fn candidate_final_pinned_lines_are_the_completion_summary() {
    let view = FinalView {
        rating: 8.25,
        lines: vec!["detail".to_string()],
        per_question: vec!["Q1".to_string()],
        solved_count: 3,
        finished_count: 7,
        skipped_count: 2,
        paste_flag_count: 1,
        total_time_ms: 0,
        not_scored: Vec::new(),
        artefact_note: "note".to_string(),
    };
    assert_eq!(
        candidate_final_pinned_lines(&CandidateFinal::Breakdown(view)),
        vec![
            "final rating: 8.25".to_string(),
            "solved: 3".to_string(),
            "finished: 7".to_string(),
            "skipped: 2".to_string(),
            "paste flags: 1".to_string(),
        ]
    );
    assert_eq!(
        candidate_final_pinned_lines(&CandidateFinal::TechnicalFailure),
        Vec::<String>::new()
    );
}
