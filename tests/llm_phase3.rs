//! Phase 3: JSON repair retry, usage fallback, and provenance.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pairproof::core::{ContextSummary, Language, ModelProvenance, Session, TokenRecord};
use pairproof::leetcode::Difficulty;
use pairproof::llm::coding::{run_coding_step, CodingStepInput};
use pairproof::llm::extraction::extract_context;
use pairproof::llm::{LlmError, Message, MetaClient, Usage};
use pairproof::orchestration::report::generate_report_with_provenance;
use pairproof::orchestration::Orchestrator;
use pairproof::scoring::FinalBreakdown;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::common_mod;
use common_mod::q;

#[derive(Clone, Copy, PartialEq)]
enum UsageMode {
    Zero,
    Real,
}

struct Stub {
    base: String,
    count: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
}

async fn stub(replies: Vec<String>, mode: UsageMode) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let count = Arc::new(AtomicUsize::new(0));
    let bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (c, b) = (count.clone(), bodies.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let (c, b, replies) = (c.clone(), b.clone(), replies.clone());
            tokio::spawn(async move {
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end = loop {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break p + 4;
                            }
                        }
                    }
                };
                let head = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
                let mut len = 0usize;
                for line in head.lines() {
                    if let Some(v) = line.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                while buf.len() < header_end + len {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                b.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[header_end..]).to_string());
                let n = c.fetch_add(1, Ordering::SeqCst);
                let content = replies
                    .get(n)
                    .or(replies.last())
                    .map(String::as_str)
                    .unwrap_or("");
                // Streaming requests get an SSE body with no usage.
                let is_stream =
                    String::from_utf8_lossy(&buf[header_end..]).contains("\"stream\":true");
                let (ctype, body) = if is_stream {
                    (
                        "text/event-stream",
                        format!(
                            "data: {}\n\ndata: [DONE]\n\n",
                            serde_json::json!({"choices": [{"delta": {"content": content}}]})
                        ),
                    )
                } else {
                    let mut resp = serde_json::json!({
                        "model": "stub-model",
                        "choices": [{"message": {"role": "assistant", "content": content}}],
                    });
                    match mode {
                        UsageMode::Zero => {
                            resp["usage"] = serde_json::json!({"prompt_tokens":0,"completion_tokens":0,"total_tokens":0})
                        }
                        UsageMode::Real => {
                            resp["usage"] = serde_json::json!({"prompt_tokens":10,"completion_tokens":5,"total_tokens":15})
                        }
                    }
                    ("application/json", resp.to_string())
                };
                let out = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(out.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });
    Stub {
        base: format!("http://{addr}/v1"),
        count,
        bodies,
    }
}

const GOOD_EXTRACTION: &str =
    r#"{"topics":["array"],"level":"mid","language":"Python","language_reason":"jd"}"#;

#[tokio::test]
async fn malformed_then_repaired_succeeds_with_two_requests() {
    let stub = stub(
        vec!["not json".into(), GOOD_EXTRACTION.into()],
        UsageMode::Real,
    )
    .await;
    let client = MetaClient::new(&stub.base, "k");
    let out = extract_context(&client, "jd text", "")
        .await
        .expect("repaired");
    assert_eq!(out.value.level, "mid");
    assert_eq!(
        out.usage.total_tokens, 30,
        "usage from both attempts summed"
    );
    assert_eq!(stub.count.load(Ordering::SeqCst), 2);
    let bodies = stub.bodies.lock().unwrap();
    assert!(
        bodies[1].contains("valid JSON only"),
        "repair prompt: {}",
        bodies[1]
    );
    assert!(bodies[1].contains("not json"), "truncated reply included");
}

#[tokio::test]
async fn malformed_then_malformed_returns_original_error_after_two_requests() {
    let stub = stub(
        vec!["first-bad".into(), "second-bad".into()],
        UsageMode::Real,
    )
    .await;
    let client = MetaClient::new(&stub.base, "k");
    let err = extract_context(&client, "jd", "")
        .await
        .expect_err("must fail");
    assert!(matches!(err, LlmError::Parse(_)), "{err:?}");
    assert_eq!(stub.count.load(Ordering::SeqCst), 2, "exactly one repair");
}

#[tokio::test]
async fn valid_first_reply_makes_no_repair_request() {
    let stub = stub(vec![GOOD_EXTRACTION.into()], UsageMode::Real).await;
    let client = MetaClient::new(&stub.base, "k");
    extract_context(&client, "jd", "").await.expect("ok");
    assert_eq!(stub.count.load(Ordering::SeqCst), 1);
}

fn started_session() -> Session {
    let easy = (0..5)
        .map(|i| q(&format!("e{i}"), Difficulty::Easy))
        .collect();
    let mut s = Session::new("s1".into(), Language::Python, easy, 0).expect("session");
    s.mark_waiting_for_candidate(1).expect("w");
    s.candidate_connected(2).expect("c");
    s.start(3).expect("s");
    s
}

#[tokio::test]
async fn coding_repair_books_usage_from_both_attempts() {
    let good = r#"{"code":"x = 1","ready":true,"summary":"done"}"#;
    let stub = stub(vec!["bad".into(), good.into()], UsageMode::Real).await;
    let client = MetaClient::new(&stub.base, "k");
    let mut session = started_session();
    let input = CodingStepInput {
        question_statement: "q".into(),
        approach_text: "a".into(),
        current_code: String::new(),
        instruction: None,
        tests_requested: false,
        language: Language::Python,
    };
    let out = run_coding_step(&client, &mut session, &input)
        .await
        .expect("repaired coding");
    assert_eq!(out.value.code, "x = 1");
    assert_eq!(
        session.current_slot().unwrap().actual_tokens,
        30,
        "10+5 booked twice"
    );
    assert_eq!(stub.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn coding_failed_repair_still_books_both_attempts() {
    let stub = stub(vec!["bad".into(), "still bad".into()], UsageMode::Real).await;
    let client = MetaClient::new(&stub.base, "k");
    let mut session = started_session();
    let input = CodingStepInput {
        question_statement: "q".into(),
        approach_text: "a".into(),
        current_code: String::new(),
        instruction: None,
        tests_requested: false,
        language: Language::Python,
    };
    assert!(run_coding_step(&client, &mut session, &input)
        .await
        .is_err());
    assert_eq!(session.current_slot().unwrap().actual_tokens, 30);
}

#[tokio::test]
async fn zero_usage_is_estimated_nonstream_and_stream() {
    let stub = stub(vec!["hello world".into()], UsageMode::Zero).await;
    let client = MetaClient::new(&stub.base, "k");
    let msgs = vec![Message::new("user", "12345678")];
    let out = client.chat("m", msgs.clone()).await.expect("chat");
    assert!(out.usage.estimated);
    assert_eq!(
        (out.usage.prompt_tokens, out.usage.completion_tokens),
        (2, 3)
    );
    let streamed = client.chat_stream("m", msgs).await.expect("stream");
    assert_eq!(streamed.content, "hello world");
    assert!(
        streamed.usage.estimated,
        "stream zero usage estimated: {:?}",
        streamed.usage
    );
    assert!(streamed.usage.total_tokens > 0);
}

#[tokio::test]
async fn real_usage_is_never_marked_estimated() {
    let stub = stub(vec!["hello".into()], UsageMode::Real).await;
    let client = MetaClient::new(&stub.base, "k");
    let out = client
        .chat("m", vec![Message::new("user", "hi")])
        .await
        .expect("chat");
    assert!(!out.usage.estimated);
    assert_eq!(out.usage.total_tokens, 15);
    // Old payloads without the flag deserialize as not estimated.
    let old: Usage =
        serde_json::from_str(r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#)
            .unwrap();
    assert!(!old.estimated);
}

#[test]
fn token_records_mark_estimated_usage_and_real_usage_stays_false() {
    // Real usage: session flag never marked.
    let real = started_session();
    let real_records = TokenRecord::from_session(&real);
    assert!(!real_records.is_empty());
    assert!(
        real_records.iter().all(|r| !r.token_usage_estimated),
        "real usage must stay false: {real_records:?}"
    );

    // Estimated usage: session-level flag flows to every token record.
    let mut estimated = started_session();
    estimated.mark_token_usage_estimated();
    let records = TokenRecord::from_session(&estimated);
    assert!(
        records.iter().all(|r| r.token_usage_estimated),
        "estimated usage must be marked: {records:?}"
    );
    let json = serde_json::to_string(&records).unwrap();
    assert!(
        json.contains("\"token_usage_estimated\":true"),
        "tokens.json must carry the flag: {json}"
    );
    let parsed: Vec<TokenRecord> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, records);
    // Old tokens.json without the flag parses as not estimated.
    let old: Vec<TokenRecord> = serde_json::from_str(
        r#"[{"index":0,"title_slug":"e0","locked_expected_tokens":null,"actual_tokens":42}]"#,
    )
    .unwrap();
    assert!(!old[0].token_usage_estimated);
}

#[test]
fn provenance_appears_in_context_json_and_report() {
    let easy = (0..5)
        .map(|i| q(&format!("e{i}"), Difficulty::Easy))
        .collect::<Vec<_>>();
    let orch = Orchestrator::create("s1".into(), Language::Python, easy, 0).expect("orch");
    let provenance = ModelProvenance {
        token_usage_estimated: true,
        ..ModelProvenance::new("ollama", "strong-x", "light-y")
    };
    let context = ContextSummary::from_session(orch.session(), vec!["array".into()], "mid".into())
        .with_provenance(&provenance);
    assert_eq!(context.provider, "ollama");
    assert_eq!(context.strong_model, "strong-x");
    assert_eq!(context.light_model, "light-y");
    assert!(context.token_usage_estimated);
    let json = serde_json::to_string(&context).unwrap();
    for needle in [
        "\"provider\":\"ollama\"",
        "\"strong_model\":\"strong-x\"",
        "\"light_model\":\"light-y\"",
        "\"token_usage_estimated\":true",
    ] {
        assert!(json.contains(needle), "missing {needle} in {json}");
    }
    // Old context.json without provenance fields still parses, as unknown.
    let old: ContextSummary = serde_json::from_str(
        r#"{"session_id":"s1","topics":[],"level":"mid","language":"Python","questions":[]}"#,
    )
    .unwrap();
    assert_eq!(old.provider, "unknown");
    assert!(!old.token_usage_estimated);

    let breakdown = FinalBreakdown {
        rating: 6.0,
        unclamped: 6.0,
        ai_correction_total: 0.0,
        solved_count: 0,
        finished_count: 0,
        skipped_count: 0,
        paste_flag_count: 0,
        total_time_ms: 0,
        lines: Vec::new(),
        per_question: Vec::new(),
        not_scored: Vec::new(),
    };
    let report = generate_report_with_provenance(orch.session(), &breakdown, &[], &[], &provenance);
    assert!(report.contains("## Provenance"), "{report}");
    assert!(report.contains("Provider: ollama"), "{report}");
    assert!(report.contains("Strong model: strong-x"), "{report}");
    assert!(report.contains("Light model: light-y"), "{report}");
    assert!(report.contains("Token usage estimated: true"), "{report}");
}
