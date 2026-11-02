//! JD and additional-notes context extraction on the strong model.

use serde::Deserialize;

use crate::core::Language;
use crate::llm::common::{decode, LlmError, LlmOutcome};
use crate::llm::{Message, MetaClient};

/// The only topic slugs selection may use; unknown slugs are dropped.
/// LeetCode silently ignores unknown slugs when filtering questions.
pub const KNOWN_TOPICS: &[&str] = &[
    "array",
    "string",
    "hash-table",
    "math",
    "dynamic-programming",
    "sorting",
    "greedy",
    "binary-search",
    "two-pointers",
    "matrix",
    "linked-list",
    "stack",
    "queue",
    "tree",
    "graph",
    "heap-priority-queue",
    "sliding-window",
    "depth-first-search",
    "breadth-first-search",
    "trie",
    "bit-manipulation",
    "recursion",
    "design",
    "simulation",
    "database",
];

/// Extracted interview context proposed from a JD and additional notes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionResult {
    pub topics: Vec<String>,
    pub level: String,
    pub proposed_language: Option<Language>,
    pub unsupported_language: Option<String>,
    pub language_reason: String,
}

#[derive(Debug, Deserialize)]
struct ExtractionWire {
    topics: Vec<String>,
    level: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    language_reason: String,
}

/// Builds system and user messages asking for JD/additional-notes extraction.
pub fn build_extraction_messages(jd_text: &str, additional_text: &str) -> Vec<Message> {
    let system = format!(
        concat!(
            "You extract interview context from a job description and additional expert notes. ",
            "Reply with JSON only, no prose outside the JSON, using exactly this schema: ",
            r#"{{"topics":["array"],"level":"mid","language":"Python","language_reason":"..."}}"#,
            ". Choose 3 to 6 topics ONLY from this list: {}. ",
            "Map the role's domain to the closest slugs in the list ",
            "(example: an AI/ML role maps to \"matrix\", \"math\", ",
            "\"dynamic-programming\", \"graph\", \"heap-priority-queue\"; ",
            "a data-heavy backend role maps to \"hash-table\", \"design\", ",
            "\"sorting\"; a SQL/data role maps to \"database\"). ",
            "Choose \"database\" only when the chosen language is SQL, and when ",
            "the chosen language is SQL choose ONLY \"database\" topics ",
            "(algorithm questions cannot be answered in SQL and database ",
            "questions cannot be answered in Python/JavaScript). ",
            "Never output a slug outside the list; LeetCode has ",
            "no domain topics such as machine-learning. ",
            "level is the seniority the role targets (for example \"junior\", \"mid\", ",
            "or \"senior\"). Choose the language from the job description and ",
            "the additional notes taken together: if either text names a ",
            "language, use the language they call for; use Python only when ",
            "neither text names one. If the two texts name different languages, ",
            "prefer the one stated as a requirement for the role. ",
            "The language field must name the actual language found (any language, ",
            "for example \"Java\"); never force-fit it to a supported list. ",
            "language_reason must say which source(s) decided (job description, ",
            "additional notes, both, or the Python default) and mention any conflict."
        ),
        KNOWN_TOPICS.join(", ")
    );
    let additional = if additional_text.trim().is_empty() {
        "(none provided)"
    } else {
        additional_text
    };
    let user = format!("Job description:\n{jd_text}\n\nAdditional notes:\n{additional}");
    vec![Message::new("system", &system), Message::new("user", &user)]
}

/// Normalizes one raw topic into a slug like `dynamic-programming`.
fn normalize_topic(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '_' { '-' } else { c })
        .collect()
}

/// Normalizes, dedupes, and allowlist-filters topics in first-occurrence order.
fn normalize_topics(raw: Vec<String>) -> Vec<String> {
    let mut topics: Vec<String> = Vec::new();
    for topic in raw {
        let normalized = normalize_topic(&topic);
        if !normalized.is_empty() && !topics.contains(&normalized) {
            topics.push(normalized);
        }
    }
    topics.retain(|topic| KNOWN_TOPICS.contains(&topic.as_str()));
    topics
}

/// Maps a model-proposed language name onto the supported languages.
fn map_language(raw: Option<&str>) -> (Option<Language>, Option<String>) {
    let trimmed = raw.unwrap_or_default().trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "" => (Some(Language::Python), None),
        "python" | "python3" => (Some(Language::Python), None),
        "javascript" | "js" | "node" | "node.js" => (Some(Language::JavaScript), None),
        "sql" | "mysql" | "postgresql" | "postgres" | "psql" => (Some(Language::Sql), None),
        _ => (None, Some(trimmed.to_string())),
    }
}

/// Parses a model reply into a normalized extraction result.
pub fn parse_extraction(content: &str) -> Result<ExtractionResult, LlmError> {
    let wire: ExtractionWire = decode(content)?;
    let level = wire.level.trim().to_string();
    if level.is_empty() {
        return Err(LlmError::Parse("level must not be empty".to_string()));
    }
    let (proposed_language, unsupported_language) = map_language(wire.language.as_deref());
    let mut topics = normalize_topics(wire.topics);
    if proposed_language == Some(Language::Sql) {
        topics.retain(|topic| topic == "database");
        if topics.is_empty() {
            topics = vec!["database".to_string()];
        }
    } else {
        topics.retain(|topic| topic != "database");
    }
    Ok(ExtractionResult {
        topics,
        level,
        proposed_language,
        unsupported_language,
        language_reason: wire.language_reason,
    })
}

/// Extracts topics, level, and language from JD and additional text.
pub async fn extract_context(
    client: &MetaClient,
    jd_text: &str,
    additional_text: &str,
) -> Result<LlmOutcome<ExtractionResult>, LlmError> {
    let model = client.strong_model.clone();
    let messages = build_extraction_messages(jd_text, additional_text);
    let outcome = client.chat(&model, messages).await?;
    let value = parse_extraction(&outcome.content)?;
    Ok(LlmOutcome {
        value,
        usage: outcome.usage,
    })
}
