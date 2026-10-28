//! Shared error, outcome, and response-parsing helpers for AI calls.

use crate::ai::{MetaApiError, Usage};
use crate::core::{CoreError, Language};
use crate::scoring::{ApproachQuality, ScoringError};

/// Errors returned by AI orchestration calls.
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("meta api error: {0}")]
    Api(#[from] MetaApiError),
    #[error("ai response parse error: {0}")]
    Parse(String),
    #[error("scoring error: {0}")]
    Scoring(#[from] ScoringError),
    #[error("core error: {0}")]
    Core(#[from] CoreError),
}

/// A parsed AI result together with the tokens spent producing it.
#[derive(Debug, Clone)]
pub struct AiOutcome<T> {
    pub value: T,
    pub usage: Usage,
}

/// Returns the human-readable name of a solution language.
pub(crate) fn language_name(language: Language) -> &'static str {
    match language {
        Language::Python => "Python",
        Language::JavaScript => "JavaScript",
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

/// Parses a quality word case-insensitively.
pub(crate) fn parse_quality(text: &str) -> Result<ApproachQuality, AiError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "best" => Ok(ApproachQuality::Best),
        "workable" => Ok(ApproachQuality::Workable),
        "weak" => Ok(ApproachQuality::Weak),
        other => Err(AiError::Parse(format!("unknown approach quality: {other}"))),
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
    fn parse_quality_is_case_insensitive_and_rejects_unknown() {
        assert_eq!(parse_quality("Best").ok(), Some(ApproachQuality::Best));
        assert_eq!(
            parse_quality(" WORKABLE ").ok(),
            Some(ApproachQuality::Workable)
        );
        assert_eq!(parse_quality("weak").ok(), Some(ApproachQuality::Weak));
        let err = parse_quality("excellent").err();
        assert!(matches!(err, Some(AiError::Parse(_))), "{err:?}");
        assert!(parse_quality("").is_err());
    }
}
