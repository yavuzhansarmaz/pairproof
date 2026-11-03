//! Slice A: approaches stay visible above the statement, carry a
//! selection marker, and still reach the core token lock.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pairproof::connection::{ConnectionString, Endpoint, EphemeralUsername, Fingerprint, Secret};
use pairproof::core::{ApproachChoice, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::orchestration::control::{
    CandidateSnapshot, SnapshotOption, SnapshotQuestion, SnapshotSelection,
};
use pairproof::scoring::{ApproachGrade, ApproachOption, ApproachQuality, ApproachSet};
use pairproof::tui::candidate::{CandidateApp, CandidateConnectError, CandidateScreen};
use pairproof::tui::candidate_actions::OtherGrader;
use pairproof::tui::candidate_chat::{other_insert, ChatController, PasteBlocked};
use pairproof::tui::candidate_run::{
    handle_candidate_key, handle_candidate_paste, work_key_action, CandidateFocus,
    CandidateRunState, WorkKeyAction,
};
use pairproof::tui::candidate_runtime::CandidateView;
use pairproof::tui::candidate_work::{
    CandidateWork, CandidateWorkError, CodingRunner, RunnerError,
};
use pairproof::tui::render::draw_candidate_screen;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[rustfmt::skip]
fn view(options: Vec<(&str, &str)>, selected: Option<SnapshotSelection>, statement: &str) -> CandidateView {
    CandidateView {
        phase: "InProgress".to_string(),
        sync_phase: "Sync: active".to_string(),
        question_title: Some("Two Sum".to_string()),
        question_index: Some(0),
        statement: Some(statement.to_string()),
        options: options
            .into_iter()
            .map(|(title, summary)| SnapshotOption {
                title: title.to_string(),
                summary: summary.to_string(),
            })
            .collect(),
        selected,
        latest_test: None,
        chat_replies: Vec::new(),
        chat_partial: None,
        final_breakdown: None,
        recovery: None,
    }
}

fn four_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Alpha", "first plan"),
        ("Beta", "second plan"),
        ("Gamma", "third plan"),
        ("Delta", "fourth plan"),
    ]
}

fn q(slug: &str) -> QuestionRef {
    QuestionRef {
        title_slug: slug.into(),
        title: format!("Title {slug}"),
        difficulty: Difficulty::Easy,
        acceptance_pct: None,
    }
}

fn started() -> Session {
    let easy: Vec<QuestionRef> = (0..5).map(|i| q(&format!("e{i}"))).collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn option(title: &str, summary: &str, tokens: u64) -> ApproachOption {
    ApproachOption {
        title: title.into(),
        summary: summary.into(),
        quality: ApproachQuality::Workable,
        expected_tokens: tokens,
    }
}

fn approach_set() -> ApproachSet {
    let mut best = option("Two pointers", "Move pointers from both ends", 3333);
    best.quality = ApproachQuality::Best;
    ApproachSet::new(vec![
        option("Hash table lookup", "Store seen values in a map", 1111),
        option("Sorting scan", "Sort then scan neighbours", 2222),
        best,
        option("Binary search", "Search halves of the range", 4444),
    ])
    .expect("approach set")
}

fn grade() -> ApproachGrade {
    ApproachGrade::new(ApproachQuality::Workable, 1234, "plausible plan".into()).expect("grade")
}

#[derive(Default)]
struct OkCodingRunner {
    calls: usize,
}

impl CodingRunner for OkCodingRunner {
    fn run_step(&mut self, _input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        self.calls += 1;
        Ok(CodingStepOutput {
            code: "print('fixed')".into(),
            ready: true,
            summary: "done".into(),
        })
    }
}

struct FakeGrader {
    calls: usize,
    last_other: String,
}

impl OtherGrader for FakeGrader {
    fn grade(&mut self, _statement: &str, other: &str) -> Result<ApproachGrade, RunnerError> {
        self.calls += 1;
        self.last_other = other.to_string();
        Ok(grade())
    }
}

struct OkVerifier;

impl pairproof::tui::candidate::ConnectionVerifier for OkVerifier {
    fn verify(&self, _c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        Ok(())
    }
}

fn encoded_connection() -> String {
    ConnectionString::new(
        Endpoint::new("secret-host.example.com", 2222).expect("endpoint"),
        EphemeralUsername::parse("pp-abcd1234").expect("username"),
        Secret::from_bytes([0xab; 16]),
        4_102_444_800_000,
        Fingerprint::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .expect("fingerprint"),
    )
    .encode()
}

fn solving_state(session: Session) -> CandidateRunState {
    let mut app = CandidateApp::with_verifier(OkVerifier);
    app.push_text(&encoded_connection());
    assert_eq!(app.submit_connection(1_000), Ok(CandidateScreen::Rules));
    app.attach_session(session);
    assert_eq!(app.accept_rules(), Ok(CandidateScreen::Solving));
    CandidateRunState::with_parts(app, CandidateWork::new(), ChatController::default())
}

#[test] #[rustfmt::skip]
fn candidate_question_displays_all_four_generated_approaches() { let value = view(four_options(), Some(SnapshotSelection::Preset(1)), "A long statement body."); let lines = value.question_lines(); assert_eq!(lines[0], "Q1: Two Sum"); assert_eq!(lines[1], "approaches: 1-4 choose, 5 other (your own)"); assert_eq!(lines[2], "1: Alpha - first plan"); assert_eq!(lines[3], "> 2: Beta - second plan"); assert_eq!(lines[4], "3: Gamma - third plan"); assert_eq!(lines[5], "4: Delta - fourth plan"); assert_eq!(lines[6], "5: other - describe your own approach"); assert_eq!(lines[7], "A long statement body."); assert!(lines.iter().any(|line| line == "selected approach: preset 1")); }

#[test] #[rustfmt::skip]
fn candidate_question_displays_other_option() { let plain = view(four_options(), None, "Statement.").question_lines(); assert!(plain.iter().any(|line| line == "5: other - describe your own approach")); let chosen = view(four_options(), Some(SnapshotSelection::Other("my own plan".to_string())), "Statement.").question_lines(); assert!(chosen.iter().any(|line| line == "> 5: other - describe your own approach")); assert!(chosen.iter().any(|line| line == "selected approach: my own plan")); }

#[test]
fn candidate_can_navigate_all_approach_options() {
    assert_eq!(work_key_action('1'), Some(WorkKeyAction::SelectPreset(0)));
    assert_eq!(work_key_action('2'), Some(WorkKeyAction::SelectPreset(1)));
    assert_eq!(work_key_action('3'), Some(WorkKeyAction::SelectPreset(2)));
    assert_eq!(work_key_action('4'), Some(WorkKeyAction::SelectPreset(3)));
    assert_eq!(work_key_action('5'), Some(WorkKeyAction::OpenOther));
    assert_eq!(work_key_action('o'), Some(WorkKeyAction::OpenOther));
}

#[test]
fn candidate_can_select_generated_approach() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_approaches(approach_set());
    let mut runner = OkCodingRunner::default();
    work.select_preset(&mut session, 1, &mut runner, 10)
        .expect("select preset");
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Preset(1))
    );
}

#[test]
fn candidate_can_select_other() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_other_text("  sliding window  ");
    let mut runner = OkCodingRunner::default();
    work.select_other(&mut session, grade(), &mut runner, 10)
        .expect("select other");
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Other("sliding window".into()))
    );
    assert_eq!(work.other_text(), "");
}

#[test]
fn candidate_other_selection_exposes_custom_input() {
    let mut state = solving_state(started());
    assert_eq!(state.app.screen(), CandidateScreen::Solving);
    assert_eq!(state.focus, CandidateFocus::Work);
    handle_candidate_key(&mut state, key(KeyCode::Char('5')));
    assert_eq!(state.focus, CandidateFocus::Other);
}

#[test]
fn candidate_other_custom_text_is_submitted() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_statement("e0", "Find two numbers that sum to target.");
    work.set_other_text("  my own plan  ");
    let mut grader = FakeGrader {
        calls: 0,
        last_other: String::new(),
    };
    let mut runner = OkCodingRunner::default();
    work.select_other_graded(&mut session, &mut grader, &mut runner, 10)
        .expect("graded other selection");
    assert_eq!(grader.calls, 1);
    assert_eq!(grader.last_other, "  my own plan  ");
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Other("my own plan".into()))
    );
}

#[test]
fn candidate_other_input_preserves_paste_restrictions() {
    let mut work = CandidateWork::new();
    work.set_other_text("typed");
    assert_eq!(
        other_insert(&mut work, "pasted text", true),
        Err(PasteBlocked::Blocked)
    );
    assert_eq!(work.other_text(), "typed");
    let mut state = solving_state(started());
    handle_candidate_paste(&mut state, "bulk pasted text");
    assert!(state.paste_notice);
}

#[test]
fn candidate_selected_approach_reaches_core_action() {
    let mut session = started();
    let mut work = CandidateWork::new();
    let set = approach_set();
    let expected = set.options[2].expected_tokens;
    work.set_approaches(set);
    let mut runner = OkCodingRunner::default();
    work.select_preset(&mut session, 2, &mut runner, 10)
        .expect("select preset");
    let slot = session.current_slot().expect("slot");
    assert_eq!(slot.approach, Some(ApproachChoice::Preset(2)));
    assert_eq!(slot.locked_expected_tokens, Some(expected));
    assert_eq!(expected, 3333);
}

#[test]
fn candidate_approach_selection_preserves_token_lock_semantics() {
    let mut session = started();
    let mut work = CandidateWork::new();
    let mut runner = OkCodingRunner::default();
    assert_eq!(
        work.select_preset(&mut session, 0, &mut runner, 10),
        Err(CandidateWorkError::NoApproaches)
    );
    work.set_approaches(approach_set());
    work.select_preset(&mut session, 0, &mut runner, 11)
        .expect("first select");
    assert_eq!(
        session.current_slot().expect("slot").locked_expected_tokens,
        Some(1111)
    );
    work.change_approach(&mut session, 12).expect("change");
    assert_eq!(session.current_slot().expect("slot").approach, None);
    assert_eq!(
        session.current_slot().expect("slot").locked_expected_tokens,
        None
    );
    work.select_preset(&mut session, 2, &mut runner, 13)
        .expect("reselect");
    let slot = session.current_slot().expect("slot");
    assert_eq!(slot.approach, Some(ApproachChoice::Preset(2)));
    assert_eq!(slot.locked_expected_tokens, Some(3333));
}

#[test] #[rustfmt::skip]
fn candidate_question_layout_keeps_options_accessible_on_short_terminal() { let statement = "long statement wording ".repeat(30); let value = view(four_options(), None, statement.trim()); let mut left = value.question_lines(); left.push("focus: work/other/redirect/chat (tab switches)".to_string()); let right = vec!["coach: hello".to_string(), "message: ".to_string()]; let backend = ratatui::backend::TestBackend::new(64, 12); let mut terminal = ratatui::Terminal::new(backend).expect("terminal"); terminal.draw(|frame| draw_candidate_screen(frame, &left, &right, false, 60, 0, 0)).expect("draw"); let buffer = terminal.backend().buffer().clone(); let area = *buffer.area(); let text = (0..area.height).map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()).collect::<Vec<_>>().join("
"); for title in ["Alpha", "Beta", "Gamma", "Delta", "other"] { assert!(text.contains(title), "missing {title}: {text}"); } }

#[test] #[rustfmt::skip]
fn candidate_question_displays_only_existing_options_when_fewer_than_four() { let lines = view(vec![("Alpha", "first plan"), ("Beta", "second plan")], None, "Statement.").question_lines(); assert!(lines.iter().any(|line| line == "1: Alpha - first plan")); assert!(lines.iter().any(|line| line == "2: Beta - second plan")); assert!(lines.iter().any(|line| line == "3: other - describe your own approach")); assert!(!lines.iter().any(|line| line.contains("Gamma"))); assert!(!lines.iter().any(|line| line.starts_with("4:"))); assert!(!lines.iter().any(|line| line.starts_with("5:"))); }

#[test]
fn fewer_options_number_other_as_next_choice() {
    let lines = view(vec![("Alpha", "a"), ("Beta", "b")], None, "S.").question_lines();
    assert!(lines.iter().any(|l| l.contains("1-2 choose, 3 other")));
    assert!(lines
        .iter()
        .any(|l| l == "3: other - describe your own approach"));
}

#[test]
fn other_selection_marker_survives_preset_like_text() {
    let lines = view(
        four_options(),
        Some(SnapshotSelection::Other("preset 9".to_string())),
        "S.",
    )
    .question_lines();
    let other = lines
        .iter()
        .find(|l| l.contains("other - describe"))
        .expect("other");
    assert!(other.starts_with("> "), "{other}");
    assert!(lines[2..6].iter().all(|l| !l.starts_with("> ")));
}

#[test] #[rustfmt::skip]
fn other_text_starting_with_preset_1_marks_other_not_preset() { let lines = view(four_options(), Some(SnapshotSelection::Other("preset 1".to_string())), "S.").question_lines(); assert!(lines[2..6].iter().all(|l| !l.starts_with("> ")), "{lines:?}"); assert!(lines[6].starts_with("> "), "{lines:?}"); assert!(lines.iter().any(|l| l == "selected approach: preset 1"), "{lines:?}"); }

#[test] #[rustfmt::skip]
fn other_text_starting_with_preset_2_marks_other_not_preset() { let lines = view(four_options(), Some(SnapshotSelection::Other("preset 2".to_string())), "S.").question_lines(); assert!(lines[2..6].iter().all(|l| !l.starts_with("> ")), "{lines:?}"); assert!(lines[6].starts_with("> "), "{lines:?}"); assert!(lines.iter().any(|l| l == "selected approach: preset 2"), "{lines:?}"); }

#[test] #[rustfmt::skip]
fn other_text_with_any_in_range_preset_label_marks_other() { for n in 0..4 { let text = format!("preset {n}"); let lines = view(four_options(), Some(SnapshotSelection::Other(text.clone())), "S.").question_lines(); assert!(lines[2..6].iter().all(|l| !l.starts_with("> ")), "{text}: {lines:?}"); assert!(lines[6].starts_with("> "), "{text}: {lines:?}"); } }

#[test] #[rustfmt::skip]
fn actual_preset_selection_still_marks_the_preset_row() { let lines = view(four_options(), Some(SnapshotSelection::Preset(2)), "S.").question_lines(); assert!(lines[4].starts_with("> "), "{lines:?}"); assert!(!lines[6].starts_with("> "), "{lines:?}"); }

#[rustfmt::skip]
fn snap_with(selected: Option<SnapshotSelection>) -> CandidateSnapshot { CandidateSnapshot { v: 1, session_id: "s1".to_string(), phase: "InProgress".to_string(), sync_phase: "Sync: active".to_string(), current: Some(SnapshotQuestion { index: 0, title: "Two Sum".to_string(), slug: "two-sum".to_string(), statement: Some("S.".to_string()) }), options: Vec::new(), selected, latest_test: None, chat_replies: Vec::new(), final_breakdown: None, chat_partial: None, recovery: None } }

#[test] #[rustfmt::skip]
fn snapshot_selection_discriminator_survives_serialization() { let back: CandidateSnapshot = serde_json::from_str(&serde_json::to_string(&snap_with(Some(SnapshotSelection::Other("preset 2".to_string())))).expect("ser")).expect("de"); assert_eq!(back.selected, Some(SnapshotSelection::Other("preset 2".to_string()))); let back: CandidateSnapshot = serde_json::from_str(&serde_json::to_string(&snap_with(Some(SnapshotSelection::Preset(1)))).expect("ser")).expect("de"); assert_eq!(back.selected, Some(SnapshotSelection::Preset(1))); }

#[test]
fn candidate_question_shows_unavailable_when_no_approaches() {
    let lines = view(Vec::new(), None, "Statement.").question_lines();
    assert_eq!(lines[1], "approaches are not available yet.");
    assert!(!lines
        .iter()
        .any(|line| line == "approaches: 1-4 choose, 5 other (your own)"));
}
