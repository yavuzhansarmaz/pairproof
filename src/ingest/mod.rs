//! Paste-only document intake, size bounds, and raw storage.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub mod pipeline;

pub use pipeline::{
    propose_context, propose_from_extraction, write_context_summary, LanguageProposal,
    ProposedContext,
};

use crate::core::ArtifactLayout;

/// Maximum characters kept per pasted document field.
pub const MAX_TEXT_CHARS: usize = 50_000;
/// Fixed raw filename for the job description paste.
pub const JD_FILENAME: &str = "jd.txt";
/// Fixed raw filename for the additional notes paste.
pub const ADDITIONAL_FILENAME: &str = "additional.txt";

/// Errors from document intake and raw storage.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("job description must not be empty")]
    EmptyJobDescription,
    #[error("ingest io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("raw directory is a symlink: {0}")]
    RawDirIsSymlink(PathBuf),
    #[error("raw file is a symlink: {0}")]
    RawFileIsSymlink(PathBuf),
    #[error("path is outside the sessions root: {0}")]
    OutsideRoot(PathBuf),
    #[error("path is not a raw directory: {0}")]
    NotARawDir(PathBuf),
    #[error("extraction returned no topics")]
    EmptyTopics,
    #[error("extraction returned an empty level")]
    EmptyLevel,
    #[error("unsupported language proposed: {0}")]
    UnsupportedLanguage(String),
    #[error("ingest llm error: {0}")]
    Llm(#[from] crate::llm::LlmError),
    #[error("ingest artifact error: {0}")]
    Artifact(#[from] crate::core::ArtifactError),
}

/// Text bounded to at most MAX_TEXT_CHARS characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedText {
    pub text: String,
    pub truncated: bool,
    pub original_chars: usize,
}

/// Bounds text to MAX_TEXT_CHARS chars on a char boundary.
pub fn bound_text(raw: &str) -> BoundedText {
    let original_chars = raw.chars().count();
    if original_chars <= MAX_TEXT_CHARS {
        return BoundedText {
            text: raw.to_string(),
            truncated: false,
            original_chars,
        };
    }
    BoundedText {
        text: raw.chars().take(MAX_TEXT_CHARS).collect(),
        truncated: true,
        original_chars,
    }
}

/// Prepared expert documents from the two pastes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpertDocuments {
    pub jd: BoundedText,
    pub additional: BoundedText,
}

impl ExpertDocuments {
    /// Returns the additional text when it is non-empty.
    pub fn additional_text(&self) -> Option<&str> {
        if self.additional.text.is_empty() {
            None
        } else {
            Some(&self.additional.text)
        }
    }
}

/// Trims and bounds both pastes, requiring a non-empty JD.
pub fn prepare_documents(
    jd_paste: &str,
    additional_paste: &str,
) -> Result<ExpertDocuments, IngestError> {
    let jd_trimmed = jd_paste.trim();
    if jd_trimmed.is_empty() {
        return Err(IngestError::EmptyJobDescription);
    }
    let additional_trimmed = additional_paste.trim();
    let additional = if additional_trimmed.is_empty() {
        BoundedText {
            text: String::new(),
            truncated: false,
            original_chars: 0,
        }
    } else {
        bound_text(additional_trimmed)
    };
    Ok(ExpertDocuments {
        jd: bound_text(jd_trimmed),
        additional,
    })
}

/// Rejects a symlinked raw dir at the given path.
fn reject_symlinked_raw_dir(raw_dir: &Path) -> Result<bool, IngestError> {
    match std::fs::symlink_metadata(raw_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(IngestError::RawDirIsSymlink(raw_dir.to_path_buf()))
        }
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Rejects a symlinked raw file at the given path.
fn reject_symlinked_raw_file(path: &Path) -> Result<(), IngestError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(IngestError::RawFileIsSymlink(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Stores prepared documents under fixed names in raw/.
pub fn store_raw_documents(
    layout: &ArtifactLayout,
    docs: &ExpertDocuments,
) -> Result<(), IngestError> {
    let raw_dir = layout.raw_dir();
    if !reject_symlinked_raw_dir(raw_dir)? {
        std::fs::create_dir_all(raw_dir)?;
    }
    reject_symlinked_raw_dir(raw_dir)?;
    let jd_path = raw_dir.join(JD_FILENAME);
    let additional_path = raw_dir.join(ADDITIONAL_FILENAME);
    let additional = docs.additional_text();
    reject_symlinked_raw_file(&jd_path)?;
    if additional.is_some() {
        reject_symlinked_raw_file(&additional_path)?;
    }
    std::fs::write(&jd_path, &docs.jd.text)?;
    if let Some(additional) = additional {
        reject_symlinked_raw_dir(raw_dir)?;
        std::fs::write(&additional_path, additional)?;
    }
    Ok(())
}

/// Lists leftover raw dirs holding stored documents.
pub fn find_leftover_raw_dirs(sessions_root: &Path) -> Result<Vec<PathBuf>, IngestError> {
    let root_metadata = match std::fs::metadata(sessions_root) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    if !root_metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "sessions root is not a directory: {}",
                sessions_root.display()
            ),
        )
        .into());
    }
    let mut leftovers = Vec::new();
    for entry in std::fs::read_dir(sessions_root)? {
        let entry = entry?;
        let session_dir = entry.path();
        let session_metadata = match std::fs::symlink_metadata(&session_dir) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if session_metadata.file_type().is_symlink() || !session_metadata.is_dir() {
            continue;
        }
        let raw_dir = session_dir.join("raw");
        let raw_metadata = match std::fs::symlink_metadata(&raw_dir) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if raw_metadata.file_type().is_symlink() || !raw_metadata.is_dir() {
            continue;
        }
        if raw_dir_has_document(&raw_dir)? {
            leftovers.push(raw_dir);
        }
    }
    leftovers.sort();
    Ok(leftovers)
}

/// Checks whether a raw dir holds a regular document file.
fn raw_dir_has_document(raw_dir: &Path) -> Result<bool, IngestError> {
    for filename in [JD_FILENAME, ADDITIONAL_FILENAME] {
        match std::fs::symlink_metadata(raw_dir.join(filename)) {
            Ok(metadata) if metadata.is_file() => return Ok(true),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}

/// Deletes one validated leftover raw dir inside the root.
pub fn delete_leftover_raw_dir(sessions_root: &Path, raw_dir: &Path) -> Result<(), IngestError> {
    if raw_dir.file_name() != Some(OsStr::new("raw")) {
        return Err(IngestError::NotARawDir(raw_dir.to_path_buf()));
    }
    let session_dir = raw_dir
        .parent()
        .ok_or_else(|| IngestError::OutsideRoot(raw_dir.to_path_buf()))?;
    let is_immediate_child = session_dir.parent() == Some(sessions_root);
    let has_normal_session = matches!(
        session_dir.components().next_back(),
        Some(Component::Normal(_))
    );
    if !is_immediate_child || !has_normal_session {
        return Err(IngestError::OutsideRoot(raw_dir.to_path_buf()));
    }
    let metadata = match std::fs::symlink_metadata(raw_dir) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    match std::fs::symlink_metadata(session_dir) {
        Ok(session_metadata) if session_metadata.file_type().is_symlink() => {
            return Err(IngestError::OutsideRoot(raw_dir.to_path_buf()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    }
    if metadata.file_type().is_symlink() {
        match std::fs::remove_file(raw_dir) {
            Ok(()) => return Ok(()),
            Err(_) => std::fs::remove_dir(raw_dir)?,
        }
        return Ok(());
    }
    match std::fs::remove_dir_all(raw_dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
