use pairproof::core::{ApproachChoice, Language, QuestionRef, Session, TestSummary};
use pairproof::leetcode::Difficulty;
use pairproof::llm::grading::{
    build_question_grading_messages, grade_question, grade_questions, parse_question_grade,
    scorable_slots, QuestionScoringInput,
};
use pairproof::llm::{LlmError, MetaClient};
use pairproof::scoring::GradeOutcome;

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
        acceptance_pct: None,
    }
}

fn started_session() -> Session {
    let easy = (0..5)
        .map(|i| q(&format!("e{i}"), Difficulty::Easy))
        .collect();
    let mut session = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    session
}

fn bare_input(slot_index: usize) -> QuestionScoringInput {
    QuestionScoringInput {
        slot_index,
        question_statement: format!("question {slot_index}"),
        approach_text: "preset option 0".into(),
        solution_code: "pass".into(),
        test_summary: None,
        tests_requested: false,
        expected_tokens: Some(100),
        actual_tokens: 50,
        actions: vec!["question finished".into()],
    }
}

#[test]
fn parse_question_grade_accepts_integer_and_float() {
    let float = parse_question_grade(r#"{"grade":7.5,"rationale":"solid","correction":0.2}"#, 3)
        .expect("float grade");
    assert_eq!(
        (float.slot_index, float.grade, float.correction),
        (3, 7.5, 0.2)
    );
    assert_eq!(float.rationale, "solid");
    let int = parse_question_grade(r#"{"grade":8,"rationale":"good","correction":0}"#, 1)
        .expect("integer grade");
    assert_eq!((int.slot_index, int.grade, int.correction), (1, 8.0, 0.0));
    let fenced = parse_question_grade(
        "```json\n{\"grade\":0,\"rationale\":\"poor\",\"correction\":-1}\n```",
        0,
    )
    .expect("fenced grade");
    assert_eq!(fenced.grade, 0.0);
}

#[test]
fn parse_question_grade_rejects_out_of_range() {
    for bad in [
        r#"{"grade":10.5,"rationale":"r","correction":0.0}"#,
        r#"{"grade":-1,"rationale":"r","correction":0.0}"#,
        r#"{"grade":5,"rationale":"  ","correction":0.0}"#,
        r#"{"grade":5,"rationale":"r","correction":"high"}"#,
        "garbage",
        "",
    ] {
        assert!(parse_question_grade(bad, 0).is_err(), "input {bad:?}");
    }
}

#[test]
fn build_grading_messages_include_all_input_fields() {
    let input = QuestionScoringInput {
        slot_index: 4,
        question_statement: "STMT_SENTINEL find the median".into(),
        approach_text: "APPROACH_SENTINEL two heaps".into(),
        solution_code: "CODE_SENTINEL def solve(): pass".into(),
        test_summary: Some(TestSummary {
            passed: 7,
            failed: 2,
            at_ms: 99,
        }),
        tests_requested: true,
        expected_tokens: Some(424242),
        actual_tokens: 313131,
        actions: vec!["ACTION_SENTINEL redirected: use a heap".into()],
    };
    let messages = build_question_grading_messages(&input);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    let system = &messages[0].content;
    assert!(system.contains("\"grade\""), "{system}");
    assert!(system.contains("\"correction\""), "{system}");
    assert!(system.contains("-0.4..=0.4"), "{system}");
    assert!(system.contains("-0.8..=0.8"), "{system}");
    let user = &messages[1].content;
    for sentinel in [
        "STMT_SENTINEL",
        "APPROACH_SENTINEL",
        "CODE_SENTINEL",
        "424242",
        "313131",
        "ACTION_SENTINEL",
        "7 passed, 2 failed",
        "true",
    ] {
        assert!(user.contains(sentinel), "missing {sentinel} in {user}");
    }
}

#[test]
fn from_session_builds_input_and_none_for_missing_slot() {
    let mut session = started_session();
    session
        .select_approach(ApproachChoice::Preset(1), 100, 10)
        .expect("select");
    let before = QuestionScoringInput::from_session(&session, 0, "Q?", "code").expect("slot 0");
    assert_eq!(before.expected_tokens, Some(100));
    assert!(!before.tests_requested);
    assert_eq!(before.approach_text, "preset option 1");
    session.change_approach(11).expect("change");
    session
        .select_approach(ApproachChoice::Other("sliding window".into()), 200, 12)
        .expect("reselect");
    session.add_tokens(40).expect("tokens");
    session.request_tests(13).expect("request tests");
    session.record_test_result(3, 1, 14).expect("test run");
    let input = QuestionScoringInput::from_session(&session, 0, "Q?", "code").expect("slot 0");
    assert_eq!(input.slot_index, 0);
    assert_eq!(input.question_statement, "Q?");
    assert_eq!(input.solution_code, "code");
    assert_eq!(input.approach_text, "sliding window");
    assert_eq!(input.expected_tokens, Some(200), "last lock wins");
    assert_eq!(input.actual_tokens, 40);
    assert!(input.tests_requested);
    assert_eq!(
        input.test_summary,
        Some(TestSummary {
            passed: 3,
            failed: 1,
            at_ms: 14
        })
    );
    assert!(input.actions.iter().any(|a| a.contains("approach changed")));
    assert!(input.actions.iter().any(|a| a.contains("tests requested")));
    assert!(input
        .actions
        .iter()
        .any(|a| a.contains("3 passed, 1 failed")));
    let untouched = QuestionScoringInput::from_session(&session, 1, "Q?", "code").expect("slot 1");
    assert_eq!(untouched.approach_text, "none");
    assert_eq!(untouched.expected_tokens, None);
    assert!(!untouched.tests_requested);
    assert_eq!(
        QuestionScoringInput::from_session(&session, 99, "Q?", "code"),
        None
    );
}

#[test]
fn scorable_slots_only_finished() {
    let mut session = started_session();
    assert_eq!(scorable_slots(&session), Vec::<usize>::new());
    session.finish_question(10).expect("finish slot 0");
    session.skip_question(11).expect("skip slot 1");
    assert_eq!(scorable_slots(&session), vec![0]);
    session.finish_question(12).expect("finish slot 2");
    assert_eq!(scorable_slots(&session), vec![0, 2]);
}

#[tokio::test]
async fn grade_question_unreachable_api_is_typed_error() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let err = grade_question(&client, &bare_input(0)).await;
    assert!(matches!(err, Err(LlmError::Api(_))), "{err:?}");
}

#[tokio::test]
async fn grade_questions_continues_past_failures_as_not_scored() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let inputs = vec![bare_input(5), bare_input(0), bare_input(9)];
    let outcomes = grade_questions(&client, &inputs).await;
    assert_eq!(
        outcomes,
        vec![
            GradeOutcome::NotScored { slot_index: 5 },
            GradeOutcome::NotScored { slot_index: 0 },
            GradeOutcome::NotScored { slot_index: 9 },
        ]
    );
}

#[tokio::test]
async fn grade_questions_empty_inputs_empty_output() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let outcomes = grade_questions(&client, &[]).await;
    assert!(outcomes.is_empty());
}

#[tokio::test]
async fn grade_questions_retries_per_question_in_isolation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let fail_count = Arc::new(AtomicUsize::new(0));
    let success_count = Arc::new(AtomicUsize::new(0));
    let fail_c = fail_count.clone();
    let success_c = success_count.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let fail_c = fail_c.clone();
            let success_c = success_c.clone();
            tokio::spawn(async move {
                let mut buffer: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end = loop {
                    let n = match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buffer.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_header_end(&buffer) {
                        break pos;
                    }
                    if buffer.len() > 65536 {
                        return;
                    }
                };
                let header_text = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
                let mut content_length = 0usize;
                for line in header_text.lines() {
                    if let Some(v) = line.strip_prefix("content-length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                while buffer.len() < header_end + content_length {
                    let n = match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    buffer.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buffer[header_end..]).to_string();
                let is_fail = body.contains("Slot index: 0");
                let (status, status_text, resp_body) = if is_fail {
                    fail_c.fetch_add(1, Ordering::SeqCst);
                    (500, "Internal Server Error", "stub failure".to_string())
                } else {
                    success_c.fetch_add(1, Ordering::SeqCst);
                    let grade = r#"{"grade":8,"rationale":"solid","correction":0.1}"#;
                    let resp = serde_json::json!({
                        "model": "muse-wonder-1.0",
                        "choices": [{"message": {"role": "assistant", "content": grade}}],
                        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
                    });
                    (200, "OK", resp.to_string())
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });

    let client = MetaClient::new(&format!("http://{addr}/v1"), "k");
    let inputs = vec![bare_input(0), bare_input(1)];
    let outcomes = grade_questions(&client, &inputs).await;
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0], GradeOutcome::NotScored { slot_index: 0 });
    match &outcomes[1] {
        GradeOutcome::Graded(g) => {
            assert_eq!(g.slot_index, 1);
            assert_eq!(g.grade, 8.0);
            assert_eq!(g.rationale, "solid");
        }
        other => panic!("expected Graded for slot 1, got {other:?}"),
    }
    assert_eq!(
        fail_count.load(Ordering::SeqCst),
        3,
        "failing question should be requested exactly 3 times"
    );
    assert_eq!(
        success_count.load(Ordering::SeqCst),
        1,
        "succeeding question should be requested exactly once"
    );
    server.abort();
}
