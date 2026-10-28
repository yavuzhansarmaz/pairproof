use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use pairproof::core::{
    read_events, ApproachChoice, ArtifactError, ArtifactLayout, ContextSummary, EventLogWriter,
    Language, QuestionRef, Session, TokenRecord,
};
use pairproof::leetcode::Difficulty;

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pairproof-artifacts-{}-{name}", std::process::id()));
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

fn token_session() -> Session {
    let mut session = Session::new(
        "s1".to_string(),
        Language::Python,
        questions("e", 5, Difficulty::Easy),
        0,
    )
    .expect("session");
    session
        .set_expert_questions(
            questions("m", 3, Difficulty::Medium),
            questions("h", 2, Difficulty::Hard),
            1,
        )
        .expect("expert set");
    session.mark_waiting_for_candidate(2).expect("waiting");
    session.candidate_connected(3).expect("connected");
    session.start(4).expect("start");
    session
        .select_approach(ApproachChoice::Preset(0), 100, 5)
        .expect("select approach");
    session.add_tokens(42).expect("add tokens");
    session.finish_question(6).expect("finish");
    session.skip_question(7).expect("skip");
    session
}

fn collect_tmp_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_tmp_files(&path, out);
        } else if path.extension().map(|e| e == "tmp").unwrap_or(false)
            || path
                .file_name()
                .map(|n| n.to_string_lossy().ends_with(".tmp"))
                .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

#[test]
fn layout_creates_dirs_and_accessors_point_at_contracted_names() {
    let dir = TestDir::new("layout");
    let root = dir.path("nested/root");
    let layout = ArtifactLayout::create(&root, "s1", Language::Python).expect("layout");
    let session_dir = root.join("s1");
    assert_eq!(layout.session_dir(), session_dir.as_path());
    assert!(layout.session_dir().is_dir());
    assert_eq!(
        layout.solutions_dir(),
        session_dir.join("solutions").as_path()
    );
    assert!(layout.solutions_dir().is_dir());
    assert_eq!(layout.raw_dir(), session_dir.join("raw").as_path());
    assert!(layout.raw_dir().is_dir());
    assert_eq!(
        layout.events_path(),
        session_dir.join("events.jsonl").as_path()
    );
    assert_eq!(
        layout.report_path(),
        session_dir.join("report.md").as_path()
    );
    assert_eq!(
        layout.tokens_path(),
        session_dir.join("tokens.json").as_path()
    );
    assert_eq!(
        layout.context_path(),
        session_dir.join("context.json").as_path()
    );
    assert_eq!(
        layout.solution_path(0),
        session_dir.join("solutions").join("01.py")
    );
    assert_eq!(
        layout.solution_path(9),
        session_dir.join("solutions").join("10.py")
    );
}

#[test]
fn creating_same_session_twice_fails() {
    let dir = TestDir::new("twice");
    let root = dir.path("root");
    ArtifactLayout::create(&root, "s1", Language::Python).expect("first create");
    let err = ArtifactLayout::create(&root, "s1", Language::Python).expect_err("second fails");
    assert!(
        matches!(err, ArtifactError::Io(ref e) if e.kind() == ErrorKind::AlreadyExists),
        "got {err:?}"
    );
}

#[test]
fn invalid_session_ids_rejected_with_nothing_created() {
    let dir = TestDir::new("invalid-ids");
    let root = dir.path("root");
    for id in ["", "..", "../evil", "a/b", "a\\b", "with space", "."] {
        let err =
            ArtifactLayout::create(&root, id, Language::Python).expect_err("invalid id must fail");
        assert!(
            matches!(err, ArtifactError::InvalidSessionId(ref s) if s == id),
            "id {id:?} got {err:?}"
        );
    }
    assert!(!root.exists(), "nothing created for invalid ids");
    assert!(!dir.path("evil").exists());
    let entries: Vec<_> = fs::read_dir(&dir.0).expect("read dir").collect();
    assert!(entries.is_empty(), "no files outside root, got {entries:?}");
}

#[test]
fn token_records_capture_locked_and_actual_in_slot_order() {
    let dir = TestDir::new("tokens");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let session = token_session();
    let records = TokenRecord::from_session(&session);
    assert_eq!(records.len(), 10);
    assert_eq!(
        records[0],
        TokenRecord {
            index: 0,
            title_slug: "e0".to_string(),
            locked_expected_tokens: Some(100),
            actual_tokens: 42,
        }
    );
    assert_eq!(records[1].locked_expected_tokens, None);
    layout.write_token_records(&session).expect("write tokens");
    let text = fs::read_to_string(layout.tokens_path()).expect("read tokens");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse tokens");
    let array = value.as_array().expect("tokens is array");
    assert_eq!(array.len(), 10);
    assert_eq!(array[0]["index"], serde_json::json!(0));
    assert_eq!(array[0]["title_slug"], serde_json::json!("e0"));
    assert_eq!(array[0]["locked_expected_tokens"], serde_json::json!(100));
    assert_eq!(array[0]["actual_tokens"], serde_json::json!(42));
    assert_eq!(array[1]["index"], serde_json::json!(1));
    assert_eq!(array[1]["title_slug"], serde_json::json!("e1"));
    assert_eq!(array[1]["locked_expected_tokens"], serde_json::Value::Null);
    assert_eq!(array[1]["actual_tokens"], serde_json::json!(0));
    assert_eq!(array[9]["title_slug"], serde_json::json!("h1"));
    assert_eq!(array[9]["locked_expected_tokens"], serde_json::Value::Null);
    let parsed: Vec<TokenRecord> = serde_json::from_str(&text).expect("parse records");
    assert_eq!(parsed, records);
}

#[test]
fn context_summary_round_trips_through_disk() {
    let dir = TestDir::new("context");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let session = token_session();
    let context = ContextSummary::from_session(
        &session,
        vec!["array".to_string(), "graph".to_string()],
        "senior".to_string(),
    );
    assert_eq!(context.session_id, "s1");
    assert_eq!(
        context.topics,
        vec!["array".to_string(), "graph".to_string()]
    );
    assert_eq!(context.level, "senior");
    assert_eq!(context.language, Language::Python);
    let slugs: Vec<&str> = context
        .questions
        .iter()
        .map(|q| q.title_slug.as_str())
        .collect();
    assert_eq!(
        slugs,
        vec!["e0", "e1", "e2", "e3", "e4", "m0", "m1", "m2", "h0", "h1"]
    );
    layout.write_context(&context).expect("write context");
    let text = fs::read_to_string(layout.context_path()).expect("read context");
    let back: ContextSummary = serde_json::from_str(&text).expect("parse context");
    assert_eq!(back, context);
}

#[test]
fn solutions_use_two_digit_numbering_and_language_extension() {
    let dir = TestDir::new("solutions");
    let root = dir.path("root");
    let py = ArtifactLayout::create(&root, "py1", Language::Python).expect("python layout");
    py.write_solution(0, "print('hi')\n")
        .expect("write solution");
    let path = py.solutions_dir().join("01.py");
    assert_eq!(
        fs::read_to_string(&path).expect("read 01.py"),
        "print('hi')\n"
    );
    assert_eq!(py.solution_path(0), path);

    let js = ArtifactLayout::create(&root, "js1", Language::JavaScript).expect("js layout");
    js.write_solution(10, "console.log(1);\n")
        .expect("write 11th");
    let js_path = js.solutions_dir().join("11.js");
    assert_eq!(js.solution_path(10), js_path);
    assert_eq!(
        fs::read_to_string(&js_path).expect("read 11.js"),
        "console.log(1);\n"
    );
}

#[test]
fn report_stored_verbatim_and_no_tmp_files_remain() {
    let dir = TestDir::new("report");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let session = token_session();
    let context = ContextSummary::from_session(&session, vec!["array".into()], "mid".into());
    layout
        .write_report("# Report\n\nScore: 7.5\n")
        .expect("report");
    layout.write_solution(0, "x = 1\n").expect("solution");
    layout.write_token_records(&session).expect("tokens");
    layout.write_context(&context).expect("context");
    assert_eq!(
        fs::read_to_string(layout.report_path()).expect("read report"),
        "# Report\n\nScore: 7.5\n"
    );
    let mut tmp = Vec::new();
    collect_tmp_files(layout.session_dir(), &mut tmp);
    assert!(tmp.is_empty(), "tmp files remain: {tmp:?}");
}

#[test]
fn delete_raw_dir_removes_raw_only_and_second_call_is_ok() {
    let dir = TestDir::new("delete-raw");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    fs::write(layout.raw_dir().join("jd.txt"), "raw").expect("write raw file");
    fs::create_dir(layout.raw_dir().join("sub")).expect("raw subdir");
    fs::write(layout.raw_dir().join("sub/jd.txt"), "jd").expect("write nested raw");
    layout.write_report("# keep").expect("report");
    layout.write_solution(0, "keep").expect("solution");
    layout.delete_raw_dir().expect("delete raw");
    assert!(!layout.raw_dir().exists());
    assert_eq!(
        fs::read_to_string(layout.report_path()).expect("report intact"),
        "# keep"
    );
    assert_eq!(
        fs::read_to_string(layout.solution_path(0)).expect("solution intact"),
        "keep"
    );
    layout.delete_raw_dir().expect("second delete ok");
}

#[test]
fn event_log_composes_at_layout_events_path() {
    let dir = TestDir::new("event-log");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let session = token_session();
    let mut writer = EventLogWriter::create(layout.events_path()).expect("create writer");
    let written = writer.write_pending(&session).expect("write pending");
    assert_eq!(written, session.events().len());
    drop(writer);
    let back = read_events(layout.events_path()).expect("read events");
    assert_eq!(back.as_slice(), session.events());
}

#[test]
fn solution_path_extreme_index_does_not_panic() {
    let dir = TestDir::new("solution-extreme");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Python).expect("layout");
    let path = layout.solution_path(usize::MAX);
    assert_eq!(path.parent(), Some(layout.solutions_dir()));
    let file_name = path.file_name().expect("file name").to_string_lossy();
    assert!(file_name.ends_with(".py"), "got {file_name}");
}

#[test]
fn solution_path_uses_sql_extension() {
    let dir = TestDir::new("solution-sql");
    let layout = ArtifactLayout::create(dir.path("root"), "s1", Language::Sql).expect("layout");
    let path = layout.solutions_dir().join("01.sql");
    assert_eq!(layout.solution_path(0), path);
    assert!(path.to_string_lossy().ends_with("01.sql"));
    layout
        .write_solution(0, "SELECT 1;")
        .expect("write solution");
    assert_eq!(fs::read_to_string(&path).expect("read 01.sql"), "SELECT 1;");
}
