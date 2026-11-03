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
