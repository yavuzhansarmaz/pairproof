use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use pairproof::core::{
    read_events, ApproachChoice, EventKind, EventLogError, EventLogWriter, Language, QuestionRef,
    Session, SessionEvent,
};
use pairproof::leetcode::Difficulty;

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pairproof-event-log-{}-{name}", std::process::id()));
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

fn real_session() -> Session {
    let mut session = Session::new(
        "s1".to_string(),
        Language::Python,
        questions("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
        .select_approach(ApproachChoice::Other("sliding window".to_string()), 100, 4)
        .expect("select approach");
    session.redirect("use a heap", 5).expect("redirect");
    session.finish_question(6).expect("finish");
    session
}

fn event(seq: u64, kind: EventKind) -> SessionEvent {
    SessionEvent {
        seq,
        at_ms: 10 + seq,
        kind,
    }
}

fn write_text(path: &Path, content: &str) {
    fs::write(path, content).expect("write hand-made log");
}

const LINE0: &str = r#"{"seq":0,"at_ms":0,"kind":"SessionCreated"}"#;
const LINE1: &str = r#"{"seq":1,"at_ms":1,"kind":"CandidateConnected"}"#;

#[test]
fn round_trip_real_session_events() {
    let dir = TestDir::new("round-trip");
    let path = dir.path("events.jsonl");
    let session = real_session();
    let mut writer = EventLogWriter::create(&path).expect("create writer");
    assert_eq!(writer.path(), path.as_path());
    let written = writer.write_pending(&session).expect("write pending");
    assert_eq!(written, session.events().len());
    assert_eq!(writer.written_count(), session.events().len());
    drop(writer);
    let back = read_events(&path).expect("read events");
    assert_eq!(back.as_slice(), session.events());
}

#[test]
fn write_pending_is_incremental() {
    let dir = TestDir::new("incremental");
    let path = dir.path("events.jsonl");
    let mut session = Session::new(
        "s1".to_string(),
        Language::Python,
        questions("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    let mut writer = EventLogWriter::create(&path).expect("create writer");
    assert_eq!(writer.write_pending(&session).expect("first write"), 1);
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    assert_eq!(writer.write_pending(&session).expect("second write"), 3);
    assert_eq!(writer.write_pending(&session).expect("third write"), 0);
    assert_eq!(writer.written_count(), session.events().len());
    drop(writer);
    assert_eq!(
        read_events(&path).expect("read").as_slice(),
        session.events()
    );
}

#[test]
fn create_fails_if_file_exists() {
    let dir = TestDir::new("exists");
    let path = dir.path("events.jsonl");
    drop(EventLogWriter::create(&path).expect("first create"));
    let err = EventLogWriter::create(&path).expect_err("second create must fail");
    assert!(
        matches!(err, EventLogError::Io(ref e) if e.kind() == ErrorKind::AlreadyExists),
        "got {err:?}"
    );
}

#[test]
fn create_fails_with_missing_parent() {
    let dir = TestDir::new("missing-parent");
    let path = dir.path("no/such/events.jsonl");
    let err = EventLogWriter::create(&path).expect_err("missing parent must fail");
    assert!(
        matches!(err, EventLogError::Io(ref e) if e.kind() == ErrorKind::NotFound),
        "got {err:?}"
    );
    assert!(!dir.path("no").exists());
}

#[test]
fn append_rejects_wrong_first_seq_and_gap() {
    let dir = TestDir::new("append-seq");
    let path = dir.path("events.jsonl");
    let mut writer = EventLogWriter::create(&path).expect("create writer");
    let err = writer
        .append(&event(5, EventKind::SessionCreated))
        .expect_err("wrong first seq");
    assert!(
        matches!(
            err,
            EventLogError::Sequence {
                expected: 0,
                got: 5
            }
        ),
        "got {err:?}"
    );
    assert_eq!(writer.written_count(), 0);
    writer
        .append(&event(0, EventKind::SessionCreated))
        .expect("seq 0");
    let err = writer
        .append(&event(2, EventKind::CandidateConnected))
        .expect_err("gap seq");
    assert!(
        matches!(
            err,
            EventLogError::Sequence {
                expected: 1,
                got: 2
            }
        ),
        "got {err:?}"
    );
    writer
        .append(&event(1, EventKind::CandidateConnected))
        .expect("seq 1 after rejected gap");
    drop(writer);
    assert_eq!(read_events(&path).expect("read").len(), 2);
}

#[test]
fn read_events_reports_parse_line_for_malformed_json() {
    let dir = TestDir::new("malformed");
    let path = dir.path("events.jsonl");
    write_text(&path, &format!("{LINE0}\nnot json\n{LINE1}\n"));
    let err = read_events(&path).expect_err("malformed json must fail");
    match err {
        EventLogError::Parse { line, message } => {
            assert_eq!(line, 2);
            assert!(!message.is_empty());
        }
        other => panic!("expected Parse, got {other:?}"),
    }
}

#[test]
fn read_events_rejects_blank_line_in_middle() {
    let dir = TestDir::new("blank-middle");
    let path = dir.path("events.jsonl");
    write_text(&path, &format!("{LINE0}\n\n{LINE1}\n"));
    let err = read_events(&path).expect_err("blank middle line must fail");
    assert!(
        matches!(err, EventLogError::Parse { line: 2, .. }),
        "got {err:?}"
    );
}

#[test]
fn read_events_reports_sequence_gap_on_disk() {
    let dir = TestDir::new("seq-gap");
    let path = dir.path("events.jsonl");
    write_text(
        &path,
        &format!("{LINE0}\n{{\"seq\":2,\"at_ms\":2,\"kind\":\"CandidateConnected\"}}\n"),
    );
    let err = read_events(&path).expect_err("seq gap must fail");
    assert!(
        matches!(
            err,
            EventLogError::Sequence {
                expected: 1,
                got: 2
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn read_events_empty_file_is_empty() {
    let dir = TestDir::new("empty");
    let path = dir.path("events.jsonl");
    write_text(&path, "");
    assert_eq!(read_events(&path).expect("read empty"), Vec::new());
}

#[test]
fn read_events_ignores_single_trailing_empty_line() {
    let dir = TestDir::new("trailing");
    let path = dir.path("events.jsonl");
    write_text(&path, &format!("{LINE0}\n{LINE1}\n\n"));
    let events = read_events(&path).expect("read with trailing empty line");
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].seq, 1);
}

#[test]
fn read_events_missing_file_is_io() {
    let dir = TestDir::new("missing-file");
    let err = read_events(dir.path("absent.jsonl")).expect_err("missing file must fail");
    assert!(
        matches!(err, EventLogError::Io(ref e) if e.kind() == ErrorKind::NotFound),
        "got {err:?}"
    );
}

#[test]
fn writer_emits_exact_golden_line() {
    let dir = TestDir::new("golden");
    let path = dir.path("events.jsonl");
    let known = SessionEvent {
        seq: 0,
        at_ms: 7,
        kind: EventKind::ApproachSelected {
            index: 3,
            choice: ApproachChoice::Other("sliding window".to_string()),
        },
    };
    let mut writer = EventLogWriter::create(&path).expect("create writer");
    writer.append(&known).expect("append golden");
    let content = fs::read_to_string(&path).expect("flushed line readable before drop");
    drop(writer);
    assert_eq!(
        content,
        "{\"seq\":0,\"at_ms\":7,\"kind\":{\"ApproachSelected\":{\"index\":3,\"choice\":{\"Other\":\"sliding window\"}}}}\n"
    );
    assert_eq!(read_events(&path).expect("read golden"), vec![known]);
}
