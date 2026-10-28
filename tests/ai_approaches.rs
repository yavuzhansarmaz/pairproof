use pairproof::ai::approaches::{
    build_approach_messages, build_other_grade_messages, generate_approaches, grade_other_approach,
    parse_approach_grade, parse_approach_set,
};
use pairproof::ai::coding::{
    apply_usage, build_coding_messages, parse_coding_step, run_coding_step, CodingStepInput,
};
use pairproof::ai::{AiError, ChatOptions, Message, MetaApiError, MetaClient, Usage};
use pairproof::core::{Language, QuestionRef, Session};
use pairproof::leetcode::Difficulty;
use pairproof::scoring::ApproachQuality;
use rand::rngs::StdRng;
use rand::SeedableRng;

fn approach_json(title: &str, quality: &str, tokens: &str) -> String {
    format!(
        r#"{{"title":"{title}","summary":"{title} summary","quality":"{quality}","expected_tokens":{tokens}}}"#
    )
}

fn set_json(items: &[String]) -> String {
    format!(r#"{{"approaches":[{}]}}"#, items.join(","))
}

fn valid_items() -> Vec<String> {
    vec![
        approach_json("Brute force", "weak", "100"),
        approach_json("Hash map", "best", "200"),
        approach_json("Sort first", "workable", "150"),
        approach_json("Two pointers", "workable", "180"),
    ]
}

fn rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

#[test]
fn parse_approach_set_accepts_fenced_json() {
    let fenced = format!("```json\n{}\n```", set_json(&valid_items()));
    let set = parse_approach_set(&fenced, &mut rng(7)).expect("valid set");
    assert_eq!(set.options.len(), 4);
    assert_eq!(set.options[set.best_index].title, "Hash map");
    let plain = parse_approach_set(&set_json(&valid_items()), &mut rng(7)).expect("plain set");
    assert_eq!(plain.options.len(), 4);
}

#[test]
fn parse_approach_set_shuffle_tracks_best_index() {
    let json = set_json(&valid_items());
    for seed in [1_u64, 42] {
        let set = parse_approach_set(&json, &mut rng(seed)).expect("valid set");
        assert_eq!(set.options.len(), 4);
        let best = &set.options[set.best_index];
        assert_eq!(best.title, "Hash map");
        assert_eq!(best.quality, ApproachQuality::Best);
        let mut titles: Vec<&str> = set.options.iter().map(|o| o.title.as_str()).collect();
        titles.sort_unstable();
        assert_eq!(
            titles,
            ["Brute force", "Hash map", "Sort first", "Two pointers"]
        );
        assert_eq!(
            set.quality_of(set.best_index as u8),
            Some(ApproachQuality::Best)
        );
    }
}

#[test]
fn parse_approach_set_rejects_wrong_count() {
    let mut three = valid_items();
    three.pop();
    assert!(parse_approach_set(&set_json(&three), &mut rng(1)).is_err());
    let mut five = valid_items();
    five.push(approach_json("Extra", "weak", "90"));
    assert!(parse_approach_set(&set_json(&five), &mut rng(1)).is_err());
    assert!(parse_approach_set(&set_json(&[]), &mut rng(1)).is_err());
}

#[test]
fn parse_approach_set_rejects_zero_and_two_bests() {
    let zero_best = vec![
        approach_json("A", "weak", "100"),
        approach_json("B", "workable", "200"),
        approach_json("C", "workable", "150"),
        approach_json("D", "weak", "180"),
    ];
    assert!(parse_approach_set(&set_json(&zero_best), &mut rng(1)).is_err());
    let two_best = vec![
        approach_json("A", "best", "100"),
        approach_json("B", "best", "200"),
        approach_json("C", "workable", "150"),
        approach_json("D", "weak", "180"),
    ];
    assert!(parse_approach_set(&set_json(&two_best), &mut rng(1)).is_err());
}

#[test]
fn parse_approach_set_rejects_bad_quality_and_bad_tokens() {
    let mut bad_quality = valid_items();
    bad_quality[0] = approach_json("Brute force", "excellent", "100");
    let err = parse_approach_set(&set_json(&bad_quality), &mut rng(1));
    assert!(matches!(err, Err(AiError::Parse(_))), "{err:?}");
    for tokens in ["0", "-5", "3.5", "\"many\""] {
        let mut items = valid_items();
        items[1] = approach_json("Hash map", "best", tokens);
        assert!(
            parse_approach_set(&set_json(&items), &mut rng(1)).is_err(),
            "tokens {tokens}"
        );
    }
    let missing = r#"{"approaches":[{"title":"A","summary":"s","quality":"best"}]}"#;
    assert!(parse_approach_set(missing, &mut rng(1)).is_err());
    let missing_top = r#"{"options":[]}"#;
    assert!(parse_approach_set(missing_top, &mut rng(1)).is_err());
}

#[test]
fn parse_approach_set_rejects_garbage_and_empty() {
    for bad in ["", "   ", "not json", "```json\n```", "{\"approaches\":"] {
        assert!(
            parse_approach_set(bad, &mut rng(1)).is_err(),
            "input {bad:?}"
        );
    }
}

#[test]
fn parse_approach_grade_ok_and_rejects_blank_rationale() {
    let ok = r#"{"quality":"Workable","expected_tokens":250,"rationale":"sound plan"}"#;
    let grade = parse_approach_grade(ok).expect("valid grade");
    assert_eq!(grade.quality, ApproachQuality::Workable);
    assert_eq!(grade.expected_tokens, 250);
    let fenced = format!("```json\n{ok}\n```");
    assert!(parse_approach_grade(&fenced).is_ok());
    let blank = r#"{"quality":"best","expected_tokens":10,"rationale":"  "}"#;
    assert!(parse_approach_grade(blank).is_err());
    let zero = r#"{"quality":"best","expected_tokens":0,"rationale":"r"}"#;
    assert!(parse_approach_grade(zero).is_err());
    assert!(parse_approach_grade("garbage").is_err());
}

#[test]
fn build_approach_messages_contain_question_schema_and_language() {
    let messages =
        build_approach_messages("Find two numbers that sum to target.", Language::Python);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    let system = &messages[0].content;
    assert!(system.contains("\"approaches\""), "{system}");
    assert!(system.contains("\"expected_tokens\""), "{system}");
    assert!(system.contains("best|workable|weak"), "{system}");
    assert!(system.contains("exactly 4"), "{system}");
    let user = &messages[1].content;
    assert!(user.contains("Find two numbers"), "{user}");
    assert!(user.contains("Python"), "{user}");
    let js = build_approach_messages("Q?", Language::JavaScript);
    assert!(js[1].content.contains("JavaScript"), "{}", js[1].content);
    let grade = build_other_grade_messages("Find two numbers.", "use a hash map");
    assert!(
        grade[0].content.contains("\"rationale\""),
        "{}",
        grade[0].content
    );
    assert!(
        grade[1].content.contains("use a hash map"),
        "{}",
        grade[1].content
    );
    assert!(
        grade[1].content.contains("Find two numbers."),
        "{}",
        grade[1].content
    );
}

fn coding_input(tests_requested: bool) -> CodingStepInput {
    CodingStepInput {
        question_statement: "Return the sum of two numbers.".into(),
        approach_text: "Add the inputs directly.".into(),
        current_code: String::new(),
        instruction: Some("Keep it simple.".into()),
        tests_requested,
        language: Language::Python,
    }
}

#[test]
fn parse_coding_step_ok_and_rejects_empty_code() {
    let ok = r#"{"code":"def add(a, b):\n    return a + b","ready":true,"summary":"done"}"#;
    let out = parse_coding_step(ok).expect("valid step");
    assert!(out.code.contains("return a + b"));
    assert!(out.ready);
    assert_eq!(out.summary, "done");
    let fenced = format!("```json\n{ok}\n```");
    assert!(parse_coding_step(&fenced).is_ok());
    for bad in [
        r#"{"code":"  ","ready":false,"summary":"empty"}"#,
        r#"{"code":"","ready":false,"summary":"empty"}"#,
        "garbage",
        "",
    ] {
        assert!(parse_coding_step(bad).is_err(), "input {bad:?}");
    }
}

#[test]
fn build_coding_messages_tests_flag_changes_prompt() {
    let with_tests = build_coding_messages(&coding_input(true));
    let lower = with_tests[0].content.to_lowercase();
    assert!(lower.contains("assert"), "{lower}");
    assert!(lower.contains("test"), "{lower}");
    assert!(!lower.contains("no test"), "{lower}");
    let without = build_coding_messages(&coding_input(false));
    let lower_no = without[0].content.to_lowercase();
    assert!(lower_no.contains("no test"), "{lower_no}");
    assert_ne!(with_tests[0].content, without[0].content);
    assert!(with_tests[0].content.contains("\"code\""));
    assert!(with_tests[0].content.contains("\"ready\""));
    let user = &with_tests[1].content;
    assert!(user.contains("Return the sum"), "{user}");
    assert!(user.contains("Add the inputs directly."), "{user}");
    assert!(user.contains("empty"), "{user}");
    assert!(user.contains("Keep it simple."), "{user}");
    let mut no_instruction = coding_input(false);
    no_instruction.instruction = None;
    no_instruction.current_code = "x = 1".into();
    let messages = build_coding_messages(&no_instruction);
    assert!(
        messages[1].content.contains("x = 1"),
        "{}",
        messages[1].content
    );
    assert!(!messages[1].content.contains("Keep it simple."));
}

fn q(slug: &str, difficulty: Difficulty) -> QuestionRef {
    QuestionRef {
        title_slug: slug.to_string(),
        title: slug.to_string(),
        difficulty,
    }
}

fn new_session() -> Session {
    let easy = (0..5)
        .map(|i| q(&format!("e{i}"), Difficulty::Easy))
        .collect();
    Session::new("s1".into(), Language::Python, easy, 0).expect("session")
}

#[test]
fn apply_usage_adds_tokens_and_errors_without_active_question() {
    let usage = Usage {
        prompt_tokens: 10,
        completion_tokens: 5,
        total_tokens: 15,
    };
    let mut idle = new_session();
    let err = apply_usage(&mut idle, &usage);
    assert!(matches!(err, Err(AiError::Core(_))), "{err:?}");
    let mut session = new_session();
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    apply_usage(&mut session, &usage).expect("usage applies");
    assert_eq!(session.current_slot().expect("slot").actual_tokens, 15);
    apply_usage(&mut session, &usage).expect("usage applies again");
    assert_eq!(session.current_slot().expect("slot").actual_tokens, 30);
}

fn unreachable_client() -> MetaClient {
    MetaClient::new("http://127.0.0.1:9/v1", "k")
}

#[tokio::test]
async fn generate_approaches_unreachable_api_is_typed_error() {
    let client = unreachable_client();
    let err = generate_approaches(
        &client,
        "Find two numbers that sum to target.",
        Language::Python,
        &mut rng(7),
    )
    .await;
    assert!(matches!(err, Err(AiError::Api(_))), "{err:?}");
}

#[tokio::test]
async fn grade_other_approach_unreachable_api_is_typed_error() {
    let client = unreachable_client();
    let err = grade_other_approach(
        &client,
        "Find two numbers that sum to target.",
        "use a hash map",
    )
    .await;
    assert!(matches!(err, Err(AiError::Api(_))), "{err:?}");
}

#[tokio::test]
async fn run_coding_step_unreachable_api_errors_and_books_no_tokens() {
    let client = unreachable_client();
    let mut session = new_session();
    session.mark_waiting_for_candidate(1).expect("waiting");
    session.candidate_connected(2).expect("connected");
    session.start(3).expect("start");
    let err = run_coding_step(&client, &mut session, &coding_input(false)).await;
    assert!(matches!(err, Err(AiError::Api(_))), "{err:?}");
    assert_eq!(session.current_slot().expect("slot").actual_tokens, 0);
}

#[tokio::test]
async fn chat_with_options_unreachable_api_is_typed_error() {
    let client = unreachable_client();
    let model = client.strong_model.clone();
    let err = client
        .chat_with_options(&model, vec![Message::new("user", "hi")], ChatOptions::low())
        .await;
    assert!(matches!(err, Err(MetaApiError::Http(_))), "{err:?}");
}

#[tokio::test]
async fn chat_stream_with_options_unreachable_api_is_typed_error() {
    let client = unreachable_client();
    let model = client.strong_model.clone();
    let err = client
        .chat_stream_with_options(&model, vec![Message::new("user", "hi")], ChatOptions::low())
        .await;
    assert!(matches!(err, Err(MetaApiError::Http(_))), "{err:?}");
}
