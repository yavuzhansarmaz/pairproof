//! Context pipeline: extracted proposal, then summary write.

use crate::core::{ArtifactLayout, ContextSummary, Language, Session};
use crate::ingest::{ExpertDocuments, IngestError};
use crate::llm::extraction::{extract_context, ExtractionResult};
use crate::llm::MetaClient;

/// Language proposed by extraction, supported or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageProposal {
    Proposed(Language),
    Unsupported(String),
}

impl LanguageProposal {
    /// Returns the supported language, only for `Proposed`.
    pub fn proposed(&self) -> Option<Language> {
        match self {
            Self::Proposed(language) => Some(*language),
            Self::Unsupported(_) => None,
        }
    }

    /// Returns the unsupported name, only for `Unsupported`.
    pub fn unsupported(&self) -> Option<&str> {
        match self {
            Self::Proposed(_) => None,
            Self::Unsupported(name) => Some(name.as_str()),
        }
    }
}

/// Extracted context awaiting expert language confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedContext {
    pub topics: Vec<String>,
    pub level: String,
    pub language: LanguageProposal,
    pub language_reason: String,
    pub jd_truncated: bool,
    pub additional_truncated: bool,
}

/// Maps an extraction result into a proposed context.
pub fn propose_from_extraction(
    docs: &ExpertDocuments,
    result: ExtractionResult,
) -> Result<ProposedContext, IngestError> {
    if result.topics.iter().all(|t| t.trim().is_empty()) {
        return Err(IngestError::EmptyTopics);
    }
    let level = result.level.trim().to_string();
    if level.is_empty() {
        return Err(IngestError::EmptyLevel);
    }
    let language = match &result.unsupported_language {
        Some(name) if !name.trim().is_empty() => {
            LanguageProposal::Unsupported(name.trim().to_string())
        }
        _ => match result.proposed_language {
            Some(language) => LanguageProposal::Proposed(language),
            None => LanguageProposal::Proposed(Language::Python),
        },
    };
    Ok(ProposedContext {
        topics: result.topics,
        level,
        language,
        language_reason: result.language_reason,
        jd_truncated: docs.jd.truncated,
        additional_truncated: docs.additional.truncated,
    })
}

/// Extracts a proposed context from the bounded documents.
pub async fn propose_context(
    client: &MetaClient,
    docs: &ExpertDocuments,
) -> Result<ProposedContext, IngestError> {
    if docs.jd.text.trim().is_empty() {
        return Err(IngestError::EmptyJobDescription);
    }
    let outcome =
        extract_context(client, &docs.jd.text, docs.additional_text().unwrap_or("")).await?;
    propose_from_extraction(docs, outcome.value)
}

/// Writes the context summary for an existing session.
pub fn write_context_summary(
    layout: &ArtifactLayout,
    session: &Session,
    proposed: &ProposedContext,
) -> Result<ContextSummary, IngestError> {
    if let LanguageProposal::Unsupported(name) = &proposed.language {
        return Err(IngestError::UnsupportedLanguage(name.clone()));
    }
    let summary =
        ContextSummary::from_session(session, proposed.topics.clone(), proposed.level.clone());
    layout.write_context(&summary)?;
    Ok(summary)
}
