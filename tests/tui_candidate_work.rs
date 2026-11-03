use pairproof::core::{ApproachChoice, CoreError, EventKind, Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{CodingStepInput, CodingStepOutput};
use pairproof::scoring::{ApproachGrade, ApproachOption, ApproachQuality, ApproachSet};
use pairproof::tui::candidate_work::*;

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

fn new_session() -> Session {
    let easy = qs("e", 5, Difficulty::Easy);
    Session::new("s1".into(), Language::Python, easy, 0).expect("session")
}

fn started() -> Session {
    let mut session = new_session();
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn approach_set() -> ApproachSet {
    ApproachSet::new(vec![
        ApproachOption {
            title: "Hash table lookup".into(),
            summary: "Store seen values in a map".into(),
            quality: ApproachQuality::Workable,
            expected_tokens: 1111,
        },
        ApproachOption {
            title: "Sorting scan".into(),
            summary: "Sort then scan neighbours".into(),
            quality: ApproachQuality::Weak,
            expected_tokens: 2222,
        },
        ApproachOption {
            title: "Two pointers".into(),
            summary: "Move pointers from both ends".into(),
            quality: ApproachQuality::Best,
            expected_tokens: 3333,
        },
        ApproachOption {
            title: "Binary search".into(),
            summary: "Search halves of the range".into(),
            quality: ApproachQuality::Workable,
            expected_tokens: 4444,
        },
    ])
    .expect("approach set")
}

fn grade() -> ApproachGrade {
    ApproachGrade::new(ApproachQuality::Workable, 1234, "plausible plan".into()).expect("grade")
}

#[derive(Default)]
struct OkCodingRunner {
    calls: usize,
    last_input: Option<CodingStepInput>,
}

impl CodingRunner for OkCodingRunner {
    fn run_step(&mut self, input: &CodingStepInput) -> Result<CodingStepOutput, RunnerError> {
        self.calls += 1;
        self.last_input = Some(input.clone());
        Ok(CodingStepOutput {
            code: "print('fixed')".into(),
            ready: true,
            summary: "done".into(),
        })
    }
}

fn has_event(session: &Session, kind: EventKind) -> bool {
    session.events().iter().any(|event| event.kind == kind)
}

#[test]
fn candidate_renders_current_statement() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_statement("e0", "Given an array, find two numbers that add up.");
    let view = work_view(&session, &work);
    assert_eq!(view.question_label.as_deref(), Some("Q1: Title e0"));
    let lines = work_lines(&session, &work).join("\n");
    assert!(lines.contains("Q1: Title e0"), "{lines}");
    assert!(
        lines.contains("Given an array, find two numbers"),
        "{lines}"
    );
    work.set_statement("e0", "a".repeat(5000));
    let long_lines = work_lines(&session, &work).join("\n");
    assert!(long_lines.contains("[truncated]"), "{long_lines}");
    assert!(!long_lines.contains(&"a".repeat(5000)));
    session.finish_question(10).expect("finish");
}

#[test]
fn candidate_approach_list_preserves_four_plus_other() {
    let session = started();
    let mut work = CandidateWork::new();
    let missing = work_lines(&session, &work).join("\n");
    assert!(
        missing.contains("approaches are not available yet."),
        "{missing}"
    );
    work.set_approaches(approach_set());
    let view = work_view(&session, &work);
    assert_eq!(view.approaches.len(), 4);
    assert_eq!(view.approaches[0].title, "Hash table lookup");
    assert_eq!(view.approaches[3].title, "Binary search");
    let lines = work_lines(&session, &work).join("\n");
    for title in [
        "Hash table lookup",
        "Sorting scan",
        "Two pointers",
        "Binary search",
    ] {
        assert!(lines.contains(title), "{lines}");
    }
    assert!(lines.contains("other:"), "{lines}");
    for banned in [
        "Best", "Workable", "Weak", "score", "rating", "1111", "2222", "3333", "4444",
    ] {
        assert!(!lines.contains(banned), "{banned} in {lines}");
    }
}

#[test]
fn other_approach_accepts_blank_input() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_other_text("   ");
    let mut runner = OkCodingRunner::default();
    // A blank Other is a deliberate submission: it selects, records
    // the event, and runs a coding step like any other choice.
    work.select_other(&mut session, grade(), &mut runner, 10)
        .expect("blank other selects");
    assert_eq!(runner.calls, 1);
    assert_eq!(
        session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Other(String::new()))
    );
    assert!(has_event(
        &session,
        EventKind::ApproachSelected {
            index: 0,
            choice: ApproachChoice::Other(String::new())
        }
    ));
    assert_eq!(work.other_text(), "");
}

#[test]
fn candidate_select_approach_uses_core_contract() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_statement("e0", "Statement for the first question.");
    work.set_approaches(approach_set());
    let mut runner = OkCodingRunner::default();
    assert_eq!(
        work.select_preset(&mut session, 4, &mut runner, 9),
        Err(CandidateWorkError::InvalidPreset(4))
    );
    work.select_preset(&mut session, 1, &mut runner, 10)
        .expect("select");
    let slot = session.current_slot().expect("slot");
    assert_eq!(slot.approach, Some(ApproachChoice::Preset(1)));
    assert_eq!(slot.locked_expected_tokens, Some(2222));
    assert!(has_event(
        &session,
        EventKind::ApproachSelected {
            index: 0,
            choice: ApproachChoice::Preset(1)
        }
    ));
    assert!(has_event(
        &session,
        EventKind::TokensLocked {
            index: 0,
            expected: 2222
        }
    ));
    assert_eq!(work.current_code(), "print('fixed')");
    assert_eq!(work.phase(), WorkPhase::CodingReady);
    let input = runner.last_input.expect("coding input");
    assert_eq!(input.approach_text, "Sorting scan");
    assert_eq!(
        input.question_statement,
        "Statement for the first question."
    );
    assert_eq!(input.language, Language::Python);
    let lines = work_lines(&session, &work).join("\n");
    assert!(lines.contains("selected approach: preset 1"), "{lines}");
    assert!(!lines.contains("2222"), "{lines}");
    let mut unavailable_session = started();
    let mut unavailable_work = CandidateWork::new();
    unavailable_work.set_approaches(approach_set());
    let mut unavailable = UnavailableCodingRunner;
    unavailable_work
        .select_preset(&mut unavailable_session, 0, &mut unavailable, 10)
        .expect("core selection stands");
    assert_eq!(unavailable_work.phase(), WorkPhase::RunnerUnavailable);
    assert_eq!(unavailable_work.current_code(), "");
    assert_eq!(
        unavailable_session.current_slot().expect("slot").approach,
        Some(ApproachChoice::Preset(0))
    );
}

#[test]
fn redirect_dispatches_real_core_action() {
    let mut session = started();
    let mut work = CandidateWork::new();
    work.set_redirect_text("  use a heap  ");
    let mut runner = OkCodingRunner::default();
    work.redirect(&mut session, &mut runner, 10)
        .expect("redirect");
    assert!(has_event(
        &session,
        EventKind::Redirected {
            index: 0,
            instruction: "use a heap".into()
        }
    ));
    assert_eq!(work.redirect_text(), "");
    assert_eq!(
        runner
            .last_input
            .expect("coding input")
            .instruction
            .as_deref(),
        Some("use a heap")
    );
    assert_eq!(work.current_code(), "print('fixed')");
}

#[test]
fn candidate_illegal_actions_return_typed_error() {
    let mut session = Session::new(
        "s1".into(),
        Language::Python,
        qs("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    let mut work = CandidateWork::new();
    let mut runner = OkCodingRunner::default();
    assert_eq!(
        work.select_preset(&mut session, 0, &mut runner, 10),
        Err(CandidateWorkError::NoApproaches)
    );
    work.set_approaches(approach_set());
    assert_eq!(
        work.select_preset(&mut session, 0, &mut runner, 10),
        Err(CandidateWorkError::Core(CoreError::NoActiveQuestion))
    );
    assert_eq!(
        work.continue_work(&mut session, &mut runner, 10),
        Err(CandidateWorkError::Core(CoreError::NoActiveQuestion))
    );
    assert_eq!(
        work.finish_question(&mut session, 10),
        Err(CandidateWorkError::Core(CoreError::NoActiveQuestion))
    );
    assert_eq!(runner.calls, 0);
    let mut started_session = started();
    let mut started_work = CandidateWork::new();
    started_work.set_redirect_text("   ");
    assert_eq!(
        started_work.redirect(&mut started_session, &mut runner, 10),
        Err(CandidateWorkError::Core(CoreError::EmptyInstruction))
    );
    let raw = "ssh root@10.0.0.1 /home/expert";
    for error in [
        CandidateWorkError::RunnerUnavailable(raw.into()),
        CandidateWorkError::RunnerFailed(raw.into()),
    ] {
        let shown = error.to_string();
        assert!(!shown.contains("ssh"), "{shown}");
        assert!(!shown.contains("10.0.0.1"), "{shown}");
        assert!(!shown.contains("/home/expert"), "{shown}");
    }
    assert_eq!(
        CandidateWorkError::RunnerFailed(raw.into()).to_string(),
        "The coding step failed; ask the expert if it persists."
    );
    let no_active_lines = work_lines(&session, &work).join("\n");
    assert!(
        no_active_lines.contains("no active question."),
        "{no_active_lines}"
    );
}

#[test]
fn approach_options_returns_candidate_visible_views_only() {
    let mut work = CandidateWork::new();
    assert!(work.approach_options().is_empty());
    work.set_approaches(
        ApproachSet::new(vec![
            ApproachOption {
                title: "Alpha plan".into(),
                summary: "Alpha summary".into(),
                quality: ApproachQuality::Workable,
                expected_tokens: 424242,
            },
            ApproachOption {
                title: "Beta plan".into(),
                summary: "Beta summary".into(),
                quality: ApproachQuality::Weak,
                expected_tokens: 424243,
            },
            ApproachOption {
                title: "Gamma plan".into(),
                summary: "Gamma summary".into(),
                quality: ApproachQuality::Best,
                expected_tokens: 424244,
            },
            ApproachOption {
                title: "Delta plan".into(),
                summary: "Delta summary".into(),
                quality: ApproachQuality::Workable,
                expected_tokens: 424245,
            },
        ])
        .expect("approach set"),
    );
    let views = work.approach_options();
    assert_eq!(views.len(), 4);
    assert_eq!(views[0].title, "Alpha plan");
    assert_eq!(views[0].summary, "Alpha summary");
    assert_eq!(views[1].title, "Beta plan");
    assert_eq!(views[1].summary, "Beta summary");
    assert_eq!(views[2].title, "Gamma plan");
    assert_eq!(views[2].summary, "Gamma summary");
    assert_eq!(views[3].title, "Delta plan");
    assert_eq!(views[3].summary, "Delta summary");
}

#[test]
fn other_and_redirect_text_accept_typed_bulk_and_single_chars() {
    let mut work = CandidateWork::new();
    work.push_other_text("heap ");
    work.push_other_char('x');
    assert_eq!(work.other_text(), "heap x");
    work.push_other_char('\u{7}');
    assert_eq!(work.other_text(), "heap x");
    work.push_redirect_char('a');
    work.push_redirect_char('b');
    assert_eq!(work.redirect_text(), "ab");
}

#[test]
fn approach_options_are_numbered_from_one_matching_keys() {
    let session = started();
    let mut work = CandidateWork::new();
    work.set_approaches(approach_set());
    let lines = work_lines(&session, &work).join("\n");
    assert!(lines.contains("1: Hash table lookup"), "{lines}");
    assert!(lines.contains("4: Binary search"), "{lines}");
    assert!(!lines.contains("0: Hash table lookup"), "{lines}");
}
