use pairproof::ai::extraction::{build_extraction_messages, extract_context, parse_extraction};
use pairproof::ai::{AiError, MetaClient};
use pairproof::core::Language;

fn extraction_json(language: &str) -> String {
    format!(
        r#"{{"topics":["array"],"level":"mid","language":"{language}","language_reason":"from jd"}}"#
    )
}

#[test]
fn parse_extraction_normalizes_and_dedupes_topics() {
    let content = r#"{"topics":["Dynamic Programming","dynamic_programming"," Array "],"level":"mid","language":"Python","language_reason":"jd"}"#;
    let result = parse_extraction(content).expect("valid extraction");
    assert_eq!(result.topics, ["dynamic-programming", "array"]);
    assert_eq!(result.level, "mid");
    assert_eq!(result.proposed_language, Some(Language::Python));
    assert_eq!(result.unsupported_language, None);
    assert_eq!(result.language_reason, "jd");
    let fenced = format!("```json\n{content}\n```");
    assert_eq!(
        parse_extraction(&fenced).expect("fenced").topics,
        ["dynamic-programming", "array"]
    );
}

#[test]
fn parse_extraction_maps_supported_languages() {
    let python = parse_extraction(&extraction_json("Python3")).expect("python3");
    assert_eq!(python.proposed_language, Some(Language::Python));
    assert_eq!(python.unsupported_language, None);
    for alias in ["JS", "node.js", "javascript", "Node"] {
        let result = parse_extraction(&extraction_json(alias)).expect("js alias");
        assert_eq!(
            result.proposed_language,
            Some(Language::JavaScript),
            "alias {alias}"
        );
        assert_eq!(result.unsupported_language, None, "alias {alias}");
    }
}

#[test]
fn parse_extraction_surfaces_unsupported_language() {
    let result = parse_extraction(&extraction_json("Java")).expect("java");
    assert_eq!(result.proposed_language, None);
    assert_eq!(result.unsupported_language, Some("Java".to_string()));
    let padded = parse_extraction(&extraction_json("  Go  ")).expect("go");
    assert_eq!(padded.proposed_language, None);
    assert_eq!(padded.unsupported_language, Some("Go".to_string()));
}

#[test]
fn parse_extraction_empty_language_defaults_to_python() {
    let empty = parse_extraction(&extraction_json("")).expect("empty language");
    assert_eq!(empty.proposed_language, Some(Language::Python));
    assert_eq!(empty.unsupported_language, None);
    let missing = r#"{"topics":["graph"],"level":"senior"}"#;
    let result = parse_extraction(missing).expect("missing language");
    assert_eq!(result.proposed_language, Some(Language::Python));
    assert_eq!(result.unsupported_language, None);
    assert_eq!(result.language_reason, "");
}

#[test]
fn parse_extraction_rejects_blank_level_and_garbage() {
    let blank = r#"{"topics":["array"],"level":"  ","language":"Python","language_reason":"x"}"#;
    let err = parse_extraction(blank);
    assert!(matches!(err, Err(AiError::Parse(_))), "{err:?}");
    for bad in ["", "   ", "not json", "```json\n```", "{\"topics\":"] {
        assert!(parse_extraction(bad).is_err(), "input {bad:?}");
    }
}

#[test]
fn build_extraction_messages_contain_priority_and_inputs() {
    let messages = build_extraction_messages("JD_MARKER text", "RESUME_MARKER text");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    let system = &messages[0].content;
    assert!(system.contains("\"topics\""), "{system}");
    assert!(system.contains("\"language\""), "{system}");
    assert!(system.contains("\"language_reason\""), "{system}");
    assert!(system.contains("LeetCode topic slugs"), "{system}");
    let lower = system.to_lowercase();
    assert!(lower.contains("job description"), "{system}");
    assert!(lower.contains("strongest resume language"), "{system}");
    assert!(system.contains("Python"), "{system}");
    assert!(lower.contains("never force-fit"), "{system}");
    let user = &messages[1].content;
    assert!(user.contains("JD_MARKER text"), "{user}");
    assert!(user.contains("RESUME_MARKER text"), "{user}");
    let jd_pos = user.find("JD_MARKER").expect("jd present");
    let resume_pos = user.find("RESUME_MARKER").expect("resume present");
    assert!(jd_pos < resume_pos, "{user}");
}

#[tokio::test]
async fn extract_context_unreachable_api_is_typed_error() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let err = extract_context(&client, "jd text", "resume text").await;
    assert!(matches!(err, Err(AiError::Api(_))), "{err:?}");
}
