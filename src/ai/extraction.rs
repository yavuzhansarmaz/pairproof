//! JD and resume context extraction on the strong model.

use serde::Deserialize;

use crate::ai::common::{strip_code_fences, AiError, AiOutcome};
use crate::ai::{Message, MetaClient};
use crate::core::Language;

/// Extracted interview context proposed from a JD and a resume.
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

/// Builds system and user messages asking for JD/resume extraction.
pub fn build_extraction_messages(jd_text: &str, resume_text: &str) -> Vec<Message> {
    let system = concat!(
        "You extract interview context from a job description and a candidate resume. ",
        "Reply with JSON only, no prose outside the JSON, using exactly this schema: ",
        r#"{"topics":["array"],"level":"mid","language":"Python","language_reason":"..."}"#,
        ". topics must be LeetCode topic slugs (for example \"array\", \"graph\", ",
        "\"dynamic-programming\") matching the skills the role and resume call for. ",
        "level is the seniority the role targets (for example \"junior\", \"mid\", ",
        "or \"senior\"). Choose language with this exact priority: ",
        "(1) the language required by the job description, else ",
        "(2) the candidate's strongest resume language, else (3) Python. ",
        "The language field must name the actual language found (any language, ",
        "for example \"Java\"); never force-fit it to a supported list. ",
        "language_reason must say which source decided the language: the job ",
        "description, the resume, or the Python default."
    );
    let user = format!("Job description:\n{jd_text}\n\nCandidate resume:\n{resume_text}");
    vec![Message::new("system", system), Message::new("user", &user)]
}

/// Normalizes one raw topic into a slug like `dynamic-programming`.
fn normalize_topic(raw: &str) -> String {
    raw.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '_' { '-' } else { c })
        .collect()
}

/// Normalizes topics and dedupes them, preserving first occurrence order.
fn normalize_topics(raw: Vec<String>) -> Vec<String> {
    let mut topics: Vec<String> = Vec::new();
    for topic in raw {
        let normalized = normalize_topic(&topic);
        if !normalized.is_empty() && !topics.contains(&normalized) {
            topics.push(normalized);
        }
    }
    topics
}

/// Maps a model-proposed language name onto the supported languages.
fn map_language(raw: Option<&str>) -> (Option<Language>, Option<String>) {
    let trimmed = raw.unwrap_or_default().trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "" => (Some(Language::Python), None),
        "python" | "python3" => (Some(Language::Python), None),
        "javascript" | "js" | "node" | "node.js" => (Some(Language::JavaScript), None),
        _ => (None, Some(trimmed.to_string())),
    }
}

/// Parses a model reply into a normalized extraction result.
pub fn parse_extraction(content: &str) -> Result<ExtractionResult, AiError> {
    let wire: ExtractionWire = serde_json::from_str(strip_code_fences(content))
        .map_err(|e| AiError::Parse(e.to_string()))?;
    let level = wire.level.trim().to_string();
    if level.is_empty() {
        return Err(AiError::Parse("level must not be empty".to_string()));
    }
    let (proposed_language, unsupported_language) = map_language(wire.language.as_deref());
    Ok(ExtractionResult {
        topics: normalize_topics(wire.topics),
        level,
        proposed_language,
        unsupported_language,
        language_reason: wire.language_reason,
    })
}

/// Extracts topics, level, and language from JD and resume text.
pub async fn extract_context(
    client: &MetaClient,
    jd_text: &str,
    resume_text: &str,
) -> Result<AiOutcome<ExtractionResult>, AiError> {
    let model = client.strong_model.clone();
    let messages = build_extraction_messages(jd_text, resume_text);
    let outcome = client.chat(&model, messages).await?;
    let value = parse_extraction(&outcome.content)?;
    Ok(AiOutcome {
        value,
        usage: outcome.usage,
    })
}
