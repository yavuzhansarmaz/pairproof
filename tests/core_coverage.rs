use std::fs;
use std::path::PathBuf;

use pairproof::core::{
    ArtifactLayout, CoreError, Language, QuestionRef, QuestionSlot, QuestionSource, QuestionStatus,
    Session, SessionStatus,
};
use pairproof::leetcode::Difficulty;

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pairproof-coverage-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        Self(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn questions(prefix: &str, n: usize, difficulty: Difficulty) -> Vec<QuestionRef> {
    (0..n)
        .map(|i| QuestionRef {
            title_slug: format!("{prefix}{i}"),
            title: format!("{prefix}{i}"),
            difficulty,
            acceptance_pct: None,
        })
        .collect()
}

fn new_session(id: &str, language: Language) -> Session {
    Session::new(
        id.to_string(),
        language,
        questions("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session")
}

#[test]
fn session_accessors_return_constructor_values() {
    let session = new_session("sess-42", Language::JavaScript);
    assert_eq!(session.id(), "sess-42");
    assert_eq!(session.language(), Language::JavaScript);
}

#[test]
fn session_status_is_terminal_table() {
    assert!(!SessionStatus::Setup.is_terminal());
    assert!(!SessionStatus::WaitingForCandidate.is_terminal());
    assert!(!SessionStatus::InProgress.is_terminal());
    assert!(!SessionStatus::WaitingForExpert.is_terminal());
    assert!(!SessionStatus::Scoring.is_terminal());
    assert!(SessionStatus::Completed.is_terminal());
    assert!(SessionStatus::EndedByExpert.is_terminal());
    assert!(SessionStatus::TechnicalFailure.is_terminal());
}

#[test]
fn question_slot_new_starts_pending_and_empty() {
    let q = QuestionRef {
        title_slug: "two-sum".to_string(),
        title: "Two Sum".to_string(),
        difficulty: Difficulty::Easy,
        acceptance_pct: None,
    };
    let slot = QuestionSlot::new(3, QuestionSource::Expert, q);
    assert_eq!(slot.index, 3);
    assert_eq!(slot.source, QuestionSource::Expert);
    assert_eq!(slot.status, QuestionStatus::Pending);
    assert_eq!(slot.approach, None);
    assert_eq!(slot.locked_expected_tokens, None);
    assert_eq!(slot.actual_tokens, 0);
    assert!(!slot.re_question_used);
    assert!(!slot.re_question_pending);
    assert_eq!(slot.started_at_ms, None);
    assert_eq!(slot.finished_at_ms, None);
    assert_eq!(slot.latest_test, None);
    assert_eq!(slot.paste_flags, 0);
}

#[test]
fn slot_lookup_out_of_bounds_is_none() {
    let session = new_session("s1", Language::Python);
    assert!(session.slot(0).is_some());
    assert!(session.slot(999).is_none());
    assert!(session.current_slot().is_none());
    assert_eq!(session.current_index(), None);
}

#[test]
fn blank_question_fields_are_rejected() {
    let mut blank_title = questions("e", 5, Difficulty::Easy);
    blank_title[0].title = String::new();
    let err = Session::new("s1".to_string(), Language::Python, blank_title, 0)
        .expect_err("empty title must fail");
    assert!(
        matches!(err, CoreError::InvalidQuestion(_)),
        "empty title got {err:?}"
    );

    let mut blank_slug = questions("e", 5, Difficulty::Easy);
    blank_slug[0].title_slug = String::new();
    let err = Session::new("s1".to_string(), Language::Python, blank_slug, 0)
        .expect_err("empty title_slug must fail");
    assert!(
        matches!(err, CoreError::InvalidQuestion(_)),
        "empty title_slug got {err:?}"
    );
}

#[cfg(unix)]
#[test]
fn delete_raw_dir_does_not_follow_symlink() {
    let dir = TestDir::new("delete-raw-symlink");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let outside = dir.path("outside.txt");
    fs::write(&outside, "keep me").expect("write outside file");
    fs::remove_dir(layout.raw_dir()).expect("remove real raw dir");
    std::os::unix::fs::symlink(&outside, layout.raw_dir()).expect("symlink raw to outside file");
    layout.delete_raw_dir().expect("delete raw dir");
    assert_eq!(
        fs::read_to_string(&outside).expect("outside file intact"),
        "keep me"
    );
    assert!(
        fs::symlink_metadata(layout.raw_dir()).is_err(),
        "symlink itself must be gone"
    );
}

#[test]
fn artifact_writers_overwrite_on_second_write() {
    let dir = TestDir::new("overwrite");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    layout.write_report("# first\n").expect("first report");
    layout.write_report("# second\n").expect("second report");
    assert_eq!(
        fs::read_to_string(layout.report_path()).expect("read report"),
        "# second\n"
    );
    layout.write_solution(0, "x = 1\n").expect("first solution");
    layout
        .write_solution(0, "x = 2\n")
        .expect("second solution");
    assert_eq!(
        fs::read_to_string(layout.solution_path(0)).expect("read solution"),
        "x = 2\n"
    );
}
