use pairproof::core::Language;
use pairproof::llm::extraction::{
    build_extraction_messages, extract_context, parse_extraction, KNOWN_TOPICS,
};
use pairproof::llm::{LlmError, MetaClient};

fn extraction_json(language: &str) -> String {
    format!(
        r#"{{"topics":["array"],"level":"mid","language":"{language}","language_reason":"from jd"}}"#
    )
}

fn extraction_json_with_topics(language: &str, topics: &[&str]) -> String {
    let topics = topics
        .iter()
        .map(|topic| format!("\"{topic}\""))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"topics":[{topics}],"level":"mid","language":"{language}","language_reason":"from jd"}}"#
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
fn parse_extraction_filters_unknown_topics() {
    let content = r#"{"topics":["machine-learning","Array","pandas","matrix"],"level":"mid","language":"Python","language_reason":"jd"}"#;
    let result = parse_extraction(content).expect("valid extraction");
    assert_eq!(result.topics, ["array", "matrix"]);
}

#[test]
fn parse_extraction_all_unknown_topics_yield_empty() {
    let content = r#"{"topics":["machine-learning","deep-learning"],"level":"senior","language":"JavaScript","language_reason":"jd"}"#;
    let result = parse_extraction(content).expect("parse still succeeds");
    assert_eq!(result.topics, Vec::<String>::new());
    assert_eq!(result.level, "senior");
    assert_eq!(result.proposed_language, Some(Language::JavaScript));
    assert_eq!(result.unsupported_language, None);
}

#[test]
fn known_topics_are_normalized_slugs() {
    assert!(!KNOWN_TOPICS.is_empty());
    let mut seen: Vec<&str> = Vec::new();
    for topic in KNOWN_TOPICS {
        assert!(!topic.is_empty(), "empty topic");
        let normalized: String = topic
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c == ' ' || c == '_' { '-' } else { c })
            .collect();
        assert_eq!(*topic, normalized, "topic {topic} is not normalized");
        assert!(!topic.contains(' '), "topic {topic} contains a space");
        assert!(!topic.contains('_'), "topic {topic} contains underscore");
        assert_eq!(
            *topic,
            topic.to_ascii_lowercase(),
            "topic {topic} not lowercase"
        );
        assert!(!seen.contains(topic), "duplicate topic {topic}");
        seen.push(topic);
    }
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
fn parse_extraction_maps_sql_aliases() {
    for alias in [
        "sql",
        " SQL ",
        "MySQL",
        "mysql",
        "PostgreSQL",
        "postgres",
        "  PSQL  ",
        "psql",
    ] {
        let result = parse_extraction(&extraction_json(alias)).expect("sql alias");
        assert_eq!(
            result.proposed_language,
            Some(Language::Sql),
            "alias {alias}"
        );
        assert_eq!(result.unsupported_language, None, "alias {alias}");
    }
    let mssql = parse_extraction(&extraction_json("mssql")).expect("mssql");
    assert_eq!(mssql.proposed_language, None);
    assert_eq!(mssql.unsupported_language, Some("mssql".to_string()));
}

#[test]
fn parse_extraction_sql_keeps_only_database_topics() {
    let content = extraction_json_with_topics("SQL", &["array", "database", "graph"]);
    let result = parse_extraction(&content).expect("sql extraction");
    assert_eq!(result.proposed_language, Some(Language::Sql));
    assert_eq!(result.topics, ["database"]);
}

#[test]
fn parse_extraction_sql_without_database_gets_database() {
    let content = extraction_json_with_topics("SQL", &["array"]);
    let result = parse_extraction(&content).expect("sql extraction");
    assert_eq!(result.proposed_language, Some(Language::Sql));
    assert_eq!(result.topics, ["database"]);
}

#[test]
fn parse_extraction_non_sql_drops_database_topic() {
    let content = extraction_json_with_topics("Python", &["database", "array"]);
    let result = parse_extraction(&content).expect("python extraction");
    assert_eq!(result.proposed_language, Some(Language::Python));
    assert_eq!(result.topics, ["array"]);
    let content = extraction_json_with_topics("Java", &["database", "array"]);
    let result = parse_extraction(&content).expect("java extraction");
    assert_eq!(result.proposed_language, None);
    assert_eq!(result.unsupported_language, Some("Java".to_string()));
    assert_eq!(result.topics, ["array"]);
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
    assert!(matches!(err, Err(LlmError::Parse(_))), "{err:?}");
    for bad in ["", "   ", "not json", "```json\n```", "{\"topics\":"] {
        assert!(parse_extraction(bad).is_err(), "input {bad:?}");
    }
}

#[test]
fn build_extraction_messages_contain_priority_and_inputs() {
    let messages = build_extraction_messages("JD_MARKER text", "ADDITIONAL_MARKER text");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
    assert_eq!(messages[1].role, "user");
    let system = &messages[0].content;
    assert!(system.contains("\"topics\""), "{system}");
    assert!(system.contains("\"level\""), "{system}");
    assert!(system.contains("\"language\""), "{system}");
    assert!(system.contains("\"language_reason\""), "{system}");
    assert!(system.contains("Choose 3 to 6 topics ONLY"), "{system}");
    let joined = KNOWN_TOPICS.join(", ");
    assert!(system.contains(&joined), "{system}");
    assert!(system.contains("matrix"), "{system}");
    assert!(system.contains("heap-priority-queue"), "{system}");
    assert!(system.contains("database"), "{system}");
    assert!(
        system.contains("a SQL/data role maps to \"database\""),
        "{system}"
    );
    assert!(
        system.contains("Choose \"database\" only when the chosen language is SQL"),
        "{system}"
    );
    assert!(
        system.contains("the chosen language is SQL choose ONLY \"database\""),
        "{system}"
    );
    assert!(
        system.contains("algorithm questions cannot be answered in SQL"),
        "{system}"
    );
    assert!(system.contains("an AI/ML role maps to"), "{system}");
    assert!(system.contains("\"dynamic-programming\""), "{system}");
    assert!(
        system.contains("a data-heavy backend role maps to"),
        "{system}"
    );
    assert!(
        system.contains("Never output a slug outside the list"),
        "{system}"
    );
    assert!(system.contains("machine-learning"), "{system}");
    let lower = system.to_lowercase();
    assert!(lower.contains("job description"), "{system}");
    assert!(lower.contains("additional notes"), "{system}");
    assert!(system.contains("Python"), "{system}");
    assert!(lower.contains("never force-fit"), "{system}");
    assert!(system.contains("taken together"), "{system}");
    assert!(system.contains("use Python only when neither"), "{system}");
    assert!(lower.contains("neither"), "{system}");
    assert!(system.contains("Python default"), "{system}");
    assert!(!system.contains("(1)"), "{system}");
    assert!(!system.contains("(2)"), "{system}");
    assert!(!system.contains("(3)"), "{system}");
    let user = &messages[1].content;
    assert!(user.contains("JD_MARKER text"), "{user}");
    assert!(user.contains("ADDITIONAL_MARKER text"), "{user}");
    let jd_pos = user.find("JD_MARKER").expect("jd present");
    let additional_pos = user.find("ADDITIONAL_MARKER").expect("additional present");
    assert!(jd_pos < additional_pos, "{user}");
    let combined = format!("{system}\n{user}").to_lowercase();
    assert!(!combined.contains("resume"), "{combined}");
}

#[test]
fn build_extraction_messages_empty_additional_shows_none_provided() {
    for additional in ["   ", ""] {
        let messages = build_extraction_messages("jd text", additional);
        let user = &messages[1].content;
        assert!(
            user.contains("Additional notes:\n(none provided)"),
            "{user}"
        );
        assert!(user.contains("jd text"), "{user}");
    }
    let messages = build_extraction_messages("jd text", "real notes about Go");
    let user = &messages[1].content;
    assert!(!user.contains("(none provided)"), "{user}");
    assert!(user.contains("real notes about Go"), "{user}");
    assert!(user.contains("jd text"), "{user}");
}

#[tokio::test]
async fn extract_context_unreachable_api_is_typed_error() {
    let client = MetaClient::new("http://127.0.0.1:9/v1", "k");
    let err = extract_context(&client, "jd text", "additional text").await;
    assert!(matches!(err, Err(LlmError::Api(_))), "{err:?}");
}
