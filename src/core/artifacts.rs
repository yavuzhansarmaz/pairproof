use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::session::Session;
use crate::core::types::{Language, QuestionRef};

/// Errors from session artifact persistence.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid session id: '{0}'")]
    InvalidSessionId(String),
    #[error("artifact serialize error: {0}")]
    Serialize(String),
}

/// Per-question token record stored in tokens.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRecord {
    pub index: usize,
    pub title_slug: String,
    pub locked_expected_tokens: Option<u64>,
    pub actual_tokens: u64,
}

impl TokenRecord {
    /// Builds one record per slot in slot order.
    pub fn from_session(session: &Session) -> Vec<TokenRecord> {
        session
            .slots()
            .iter()
            .map(|slot| TokenRecord {
                index: slot.index,
                title_slug: slot.question.title_slug.clone(),
                locked_expected_tokens: slot.locked_expected_tokens,
                actual_tokens: slot.actual_tokens,
            })
            .collect()
    }
}

/// Non-personal context summary stored in context.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSummary {
    pub session_id: String,
    pub topics: Vec<String>,
    pub level: String,
    pub language: Language,
    pub questions: Vec<QuestionRef>,
}

impl ContextSummary {
    /// Builds a summary from a session plus extracted topics and level.
    pub fn from_session(session: &Session, topics: Vec<String>, level: String) -> Self {
        Self {
            session_id: session.id().to_string(),
            topics,
            level,
            language: session.language(),
            questions: session
                .slots()
                .iter()
                .map(|slot| slot.question.clone())
                .collect(),
        }
    }
}

/// On-disk layout for one session's artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactLayout {
    session_dir: PathBuf,
    solutions_dir: PathBuf,
    raw_dir: PathBuf,
    events_path: PathBuf,
    report_path: PathBuf,
    tokens_path: PathBuf,
    context_path: PathBuf,
    shadow_path: PathBuf,
    language: Language,
}

impl ArtifactLayout {
    /// Creates the session artifact layout under `root`.
    pub fn create(
        root: impl AsRef<Path>,
        session_id: &str,
        language: Language,
    ) -> Result<Self, ArtifactError> {
        validate_session_id(session_id)?;
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let session_dir = root.join(session_id);
        std::fs::create_dir(&session_dir)?;
        let solutions_dir = session_dir.join("solutions");
        std::fs::create_dir(&solutions_dir)?;
        let raw_dir = session_dir.join("raw");
        std::fs::create_dir(&raw_dir)?;
        Ok(Self {
            events_path: session_dir.join("events.jsonl"),
            report_path: session_dir.join("report.md"),
            tokens_path: session_dir.join("tokens.json"),
            context_path: session_dir.join("context.json"),
            shadow_path: session_dir.join("shadow_evaluations.json"),
            session_dir,
            solutions_dir,
            raw_dir,
            language,
        })
    }

    /// Returns the session directory path.
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// Returns the events.jsonl path.
    pub fn events_path(&self) -> &Path {
        &self.events_path
    }

    /// Returns the report.md path.
    pub fn report_path(&self) -> &Path {
        &self.report_path
    }

    /// Returns the tokens.json path.
    pub fn tokens_path(&self) -> &Path {
        &self.tokens_path
    }

    /// Returns the context.json path.
    pub fn context_path(&self) -> &Path {
        &self.context_path
    }

    /// Returns the shadow_evaluations.json path.
    pub fn shadow_path(&self) -> &Path {
        &self.shadow_path
    }

    /// Writes shadow evaluation reports as pretty JSON. Shadow data
    /// is expert-side evidence only and is never candidate-facing.
    pub fn write_shadow_evaluations(
        &self,
        reports: &[crate::scoring::ShadowReport],
    ) -> Result<(), ArtifactError> {
        let json = serde_json::to_string_pretty(reports)
            .map_err(|e| ArtifactError::Serialize(e.to_string()))?;
        write_atomic(&self.shadow_path, json.as_bytes())
    }

    /// Returns the solutions directory path.
    pub fn solutions_dir(&self) -> &Path {
        &self.solutions_dir
    }

    /// Returns the raw directory path.
    pub fn raw_dir(&self) -> &Path {
        &self.raw_dir
    }

    /// Returns the solution file path for a slot index.
    pub fn solution_path(&self, slot_index: usize) -> PathBuf {
        self.solutions_dir.join(format!(
            "{:02}.{}",
            (slot_index as u128) + 1,
            self.language.file_extension()
        ))
    }

    /// Stores the given markdown verbatim as report.md.
    pub fn write_report(&self, markdown: &str) -> Result<(), ArtifactError> {
        write_atomic(&self.report_path, markdown.as_bytes())
    }

    /// Stores solution code verbatim for a slot index.
    pub fn write_solution(&self, slot_index: usize, code: &str) -> Result<(), ArtifactError> {
        write_atomic(&self.solution_path(slot_index), code.as_bytes())
    }

    /// Writes pretty JSON token records derived from the session.
    pub fn write_token_records(&self, session: &Session) -> Result<(), ArtifactError> {
        let records = TokenRecord::from_session(session);
        let json = serde_json::to_string_pretty(&records)
            .map_err(|e| ArtifactError::Serialize(e.to_string()))?;
        write_atomic(&self.tokens_path, json.as_bytes())
    }

    /// Writes the context summary as pretty JSON.
    pub fn write_context(&self, context: &ContextSummary) -> Result<(), ArtifactError> {
        let json = serde_json::to_string_pretty(context)
            .map_err(|e| ArtifactError::Serialize(e.to_string()))?;
        write_atomic(&self.context_path, json.as_bytes())
    }

    /// Removes the raw directory recursively if present.
    pub fn delete_raw_dir(&self) -> Result<(), ArtifactError> {
        if self.raw_dir.file_name() != Some(OsStr::new("raw"))
            || self.raw_dir.parent() != Some(self.session_dir.as_path())
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "raw dir is not inside the session dir",
            )
            .into());
        }
        let metadata = match std::fs::symlink_metadata(&self.raw_dir) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if metadata.file_type().is_symlink() {
            match std::fs::remove_file(&self.raw_dir) {
                Ok(()) => return Ok(()),
                Err(_) => std::fs::remove_dir(&self.raw_dir)?,
            }
            return Ok(());
        }
        match std::fs::remove_dir_all(&self.raw_dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Validates a session id with the canonical rule in
/// `crate::core::types::validate_session_id`, mapped to `ArtifactError`.
fn validate_session_id(session_id: &str) -> Result<(), ArtifactError> {
    if crate::core::types::validate_session_id(session_id) {
        Ok(())
    } else {
        Err(ArtifactError::InvalidSessionId(session_id.to_string()))
    }
}

/// Writes content via a sibling temp file then renames over target.
fn write_atomic(path: &Path, content: &[u8]) -> Result<(), ArtifactError> {
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp_path = PathBuf::from(tmp_os);
    if let Err(e) = std::fs::write(&tmp_path, content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }
    Ok(())
}
