//! Shared error, outcome, and response-parsing helpers for LLM calls.

use crate::core::{CoreError, Language};
use crate::llm::{Message, MetaApiError, MetaClient, Usage};
use crate::scoring::{ApproachQuality, ScoringError};

/// Errors returned by LLM orchestration calls.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("meta api error: {0}")]
    Api(#[from] MetaApiError),
    #[error("llm response parse error: {0}")]
    Parse(String),
    #[error("scoring error: {0}")]
    Scoring(#[from] ScoringError),
    #[error("core error: {0}")]
    Core(#[from] CoreError),
}

/// A parsed LLM result together with the tokens spent producing it.
#[derive(Debug, Clone)]
pub struct LlmOutcome<T> {
    pub value: T,
    pub usage: Usage,
}

/// Returns the human-readable name of a solution language.
pub(crate) fn language_name(language: Language) -> &'static str {
    match language {
        Language::Python => "Python",
        Language::JavaScript => "JavaScript",
        Language::Sql => "SQL (MySQL)",
    }
}

/// Strips one surrounding markdown code fence, if present.
pub(crate) fn strip_code_fences(text: &str) -> &str {
    let mut stripped = text.trim();
    if stripped.starts_with("```") {
        stripped = match stripped.find('\n') {
            Some(index) => &stripped[index + 1..],
            None => "",
        };
    }
    stripped = stripped.trim_end();
    if let Some(without_tail) = stripped.strip_suffix("```") {
        stripped = without_tail;
    }
    stripped.trim()
}

/// Decodes a fenced JSON model reply, preserving `Parse` error wording.
pub(crate) fn decode<T: serde::de::DeserializeOwned>(content: &str) -> Result<T, LlmError> {
    serde_json::from_str(strip_code_fences(content)).map_err(|e| LlmError::Parse(e.to_string()))
}

const REPAIR_REPLY_CHARS: usize = 2000;

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Builds the single repair request: original messages (which carry the
/// schema), the truncated bad reply, and the parse error.
pub(crate) fn build_repair_messages(
    original: &[Message],
    bad_reply: &str,
    error: &LlmError,
) -> Vec<Message> {
    let truncated = truncate_chars(bad_reply, REPAIR_REPLY_CHARS);
    let mut messages = original.to_vec();
    messages.push(Message::new("assistant", &truncated));
    messages.push(Message::new(
        "user",
        &format!(
            "Your previous reply failed to parse or validate: {error}. \
             Return valid JSON only, matching the original schema exactly. \
             No prose and no markdown fences. Previous reply (truncated):\n{truncated}"
        ),
    ));
    messages
}

/// One non-streaming JSON call with exactly one repair retry on
/// parse/validation failure. `on_usage` runs after each attempt, before
/// parsing, so callers (coding) can book tokens from both attempts even
/// when parsing ultimately fails. On success the returned usage is the
/// sum of both attempts; if the repair reply also fails to parse, the
/// original error is returned and no further request is made.
pub(crate) async fn chat_json_with_repair<T, F>(
    client: &MetaClient,
    model: &str,
    messages: Vec<Message>,
    mut parse: F,
    on_usage: &mut (dyn FnMut(&Usage) -> Result<(), LlmError> + Send),
) -> Result<LlmOutcome<T>, LlmError>
where
    F: FnMut(&str) -> Result<T, LlmError>,
{
    let first = client.chat(model, messages.clone()).await?;
    on_usage(&first.usage)?;
    match parse(&first.content) {
        Ok(value) => Ok(LlmOutcome {
            value,
            usage: first.usage,
        }),
        Err(original_err) => {
            let repair_messages = build_repair_messages(&messages, &first.content, &original_err);
            let second = client.chat(model, repair_messages).await?;
            on_usage(&second.usage)?;
            let usage = first.usage.combined(&second.usage);
            match parse(&second.content) {
                Ok(value) => Ok(LlmOutcome { value, usage }),
                Err(_) => Err(original_err),
            }
        }
    }
}

/// Parses a quality word case-insensitively.
pub(crate) fn parse_quality(text: &str) -> Result<ApproachQuality, LlmError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "best" => Ok(ApproachQuality::Best),
        "workable" => Ok(ApproachQuality::Workable),
        "weak" => Ok(ApproachQuality::Weak),
        other => Err(LlmError::Parse(format!(
            "unknown approach quality: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_code_fences_handles_json_and_plain() {
        assert_eq!(strip_code_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fences("```\n{\"a\":1}\n```\n"), "{\"a\":1}");
        assert_eq!(strip_code_fences("  {\"a\":1}  "), "{\"a\":1}");
        assert_eq!(strip_code_fences("```json\n{\"a\":1}"), "{\"a\":1}");
        assert_eq!(strip_code_fences(""), "");
        assert_eq!(strip_code_fences("```"), "");
    }

    #[test]
    fn decode_parses_fenced_json_and_wraps_errors_as_parse() {
        #[derive(Debug, PartialEq, serde::Deserialize)]
        struct Wire {
            a: u32,
        }
        assert_eq!(
            decode::<Wire>("```json\n{\"a\":1}\n```").ok(),
            Some(Wire { a: 1 })
        );
        assert_eq!(decode::<Wire>("{\"a\":2}").ok(), Some(Wire { a: 2 }));
        let err = decode::<Wire>("not json").err();
        assert!(matches!(err, Some(LlmError::Parse(_))), "{err:?}");
    }

    #[test]
    fn parse_quality_is_case_insensitive_and_rejects_unknown() {
        assert_eq!(parse_quality("Best").ok(), Some(ApproachQuality::Best));
        assert_eq!(
            parse_quality(" WORKABLE ").ok(),
            Some(ApproachQuality::Workable)
        );
        assert_eq!(parse_quality("weak").ok(), Some(ApproachQuality::Weak));
        let err = parse_quality("excellent").err();
        assert!(matches!(err, Some(LlmError::Parse(_))), "{err:?}");
        assert!(parse_quality("").is_err());
    }
}
