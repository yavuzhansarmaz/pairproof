use crate::core::Language;

use super::error::SandboxError;

/// Root of the candidate workspace inside the container.
pub const WORKSPACE_ROOT: &str = "/workspace";

/// Maximum size of a single workspace file.
pub const MAX_FILE_BYTES: usize = 1024 * 1024;

/// Delegates to the canonical session-id rule in
/// `crate::core::types::validate_session_id`, mapped to `SandboxError`.
pub fn validate_session_id(raw: &str) -> Result<(), SandboxError> {
    if crate::core::types::validate_session_id(raw) {
        Ok(())
    } else {
        Err(SandboxError::InvalidSessionId(raw.to_string()))
    }
}

/// Derives the container name for a validated session id.
pub fn container_name(session_id: &str) -> Result<String, SandboxError> {
    validate_session_id(session_id)?;
    Ok(format!("pairproof-{session_id}"))
}

/// Derives the per-session network name for a validated session id.
pub fn network_name(session_id: &str) -> Result<String, SandboxError> {
    validate_session_id(session_id)?;
    Ok(format!("pairproof-net-{session_id}"))
}

/// Validates that `raw` is `pairproof-<session id>` and not a network name.
pub fn validate_container_name(raw: &str) -> Result<(), SandboxError> {
    if raw.starts_with("pairproof-net-") {
        return Err(SandboxError::InvalidCommand(raw.to_string()));
    }
    match raw.strip_prefix("pairproof-") {
        Some(suffix) => match validate_session_id(suffix) {
            Ok(()) => Ok(()),
            Err(_) => Err(SandboxError::InvalidCommand(raw.to_string())),
        },
        None => Err(SandboxError::InvalidCommand(raw.to_string())),
    }
}

/// A validated relative path inside the container workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePath {
    rel: String,
}

impl WorkspacePath {
    /// Validates a relative workspace path with forward slashes only.
    pub fn new(raw: &str) -> Result<Self, SandboxError> {
        let err = || SandboxError::InvalidWorkspacePath(raw.to_string());
        if raw.is_empty() || raw.len() > 512 {
            return Err(err());
        }
        if raw.starts_with('/') || raw.starts_with('\\') {
            return Err(err());
        }
        if raw.contains('\\') || raw.chars().any(|c| c.is_control()) {
            return Err(err());
        }
        for component in raw.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(err());
            }
            if component.len() > 255 {
                return Err(err());
            }
        }
        Ok(Self {
            rel: raw.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.rel
    }

    /// Returns the absolute path inside the container.
    pub fn container_path(&self) -> String {
        format!("{WORKSPACE_ROOT}/{}", self.rel)
    }

    /// Returns the container directory holding this path, if it has a parent.
    pub fn parent_container_dir(&self) -> Option<String> {
        let idx = self.rel.rfind('/')?;
        Some(format!("{WORKSPACE_ROOT}/{}", &self.rel[..idx]))
    }
}

/// Returns the question and solution paths for a zero-based question index.
pub fn question_layout(
    index: usize,
    language: Language,
) -> Result<(WorkspacePath, WorkspacePath), SandboxError> {
    if index > 98 {
        return Err(SandboxError::InvalidWorkspacePath(format!(
            "question index {index} exceeds 98"
        )));
    }
    let n = index + 1;
    let question = WorkspacePath::new(&format!("q{n}/question.md"))?;
    let solution = WorkspacePath::new(&format!("q{n}/solution.{}", language.file_extension()))?;
    Ok((question, solution))
}
