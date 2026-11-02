//! Secure API-key storage with explicit paths; never logs key material.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors from API-key storage; messages never contain the key.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// No usable key from environment or stored file.
    #[error("api key is missing; set MODEL_API_KEY or save a key")]
    MissingKey,
    /// A blank key was offered for saving.
    #[error("api key must not be empty")]
    EmptyKey,
    /// A path component is a symlink and was refused.
    #[error("refusing to use symlinked path: {0}")]
    Symlink(String),
    /// An underlying filesystem operation failed.
    #[error("api key storage io error: {0}")]
    Io(String),
}

fn io_err(error: std::io::Error) -> ConfigError {
    ConfigError::Io(error.to_string())
}

fn env_key() -> Option<String> {
    std::env::var(crate::llm::API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Returns true when a non-blank MODEL_API_KEY env var is set.
pub fn env_override_active() -> bool {
    env_key().is_some()
}

/// Resolves `$HOME/.config/pairproof/api_key` from the HOME variable.
pub fn default_api_key_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("pairproof")
            .join("api_key"),
    )
}

/// Masks a key, revealing only the last four characters when long enough.
pub fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() < 8 {
        return "********".to_string();
    }
    let last_four: String = chars[chars.len() - 4..].iter().collect();
    format!("\u{2022}\u{2022}\u{2022}\u{2022}{last_four}")
}

fn is_symlink(path: &Path) -> Result<bool, ConfigError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_err(error)),
    }
}

fn reject_symlink(path: &Path) -> Result<(), ConfigError> {
    if is_symlink(path)? {
        return Err(ConfigError::Symlink(path.display().to_string()));
    }
    Ok(())
}

fn reject_ancestor_symlinks(path: &Path) -> Result<(), ConfigError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    let mut chain: Vec<&Path> = parent.ancestors().collect();
    chain.reverse();
    for ancestor in chain {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        reject_symlink(ancestor)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(io_err)
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Result<u32, ConfigError> {
    use std::os::unix::fs::PermissionsExt;
    Ok(std::fs::metadata(path)
        .map_err(io_err)?
        .permissions()
        .mode()
        & 0o777)
}

fn ensure_parent_dir(path: &Path) -> Result<PathBuf, ConfigError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| ConfigError::Io("api key path has no parent directory".to_string()))?
        .to_path_buf();
    reject_ancestor_symlinks(path)?;
    match std::fs::symlink_metadata(&parent) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ConfigError::Symlink(parent.display().to_string()));
            }
            if !metadata.is_dir() {
                return Err(ConfigError::Io(format!(
                    "api key parent is not a directory: {}",
                    parent.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&parent).map_err(io_err)?;
            reject_ancestor_symlinks(path)?;
        }
        Err(error) => return Err(io_err(error)),
    }
    reject_ancestor_symlinks(path)?;
    set_permissions(&parent, 0o700)?;
    Ok(parent)
}

fn sibling_temp_path(parent: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".api_key.tmp-{}-{stamp}", std::process::id()))
}

/// Explicit-path API-key configuration; Debug shows only the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyConfig {
    path: PathBuf,
}

impl ApiKeyConfig {
    /// Creates a config bound to an explicit storage path.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Returns the storage path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolves the effective key for this config path.
    pub fn resolve(&self) -> Result<String, ConfigError> {
        resolve_api_key(&self.path)
    }

    /// Reads the stored key for this config path, if any.
    pub fn stored(&self) -> Result<Option<String>, ConfigError> {
        stored_api_key(&self.path)
    }

    /// Saves a key to this config path.
    pub fn save(&self, key: &str) -> Result<(), ConfigError> {
        save_api_key(&self.path, key)
    }

    /// Deletes the stored key for this config path.
    pub fn delete(&self) -> Result<(), ConfigError> {
        delete_api_key(&self.path)
    }
}

/// Reads the stored key, returning None when missing or blank.
pub fn stored_api_key(path: &Path) -> Result<Option<String>, ConfigError> {
    reject_ancestor_symlinks(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ConfigError::Symlink(path.display().to_string()));
            }
            if !metadata.is_file() {
                return Err(ConfigError::Io(format!(
                    "api key path is not a file: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_err(error)),
    }
    let raw = std::fs::read_to_string(path).map_err(io_err)?;
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

/// Resolves the effective key: non-blank env wins over stored.
pub fn resolve_api_key(path: &Path) -> Result<String, ConfigError> {
    if let Some(key) = env_key() {
        return Ok(key);
    }
    match stored_api_key(path)? {
        Some(key) => Ok(key),
        None => Err(ConfigError::MissingKey),
    }
}

/// Saves a trimmed key atomically with private permissions on Unix.
pub fn save_api_key(path: &Path, key: &str) -> Result<(), ConfigError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::EmptyKey);
    }
    let parent = ensure_parent_dir(path)?;
    reject_symlink(path)?;
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.is_dir() {
            return Err(ConfigError::Io(format!(
                "api key path is a directory: {}",
                path.display()
            )));
        }
    }
    let temp = sibling_temp_path(&parent);
    let write_result = write_temp_key(&temp, trimmed);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    // Re-check immediately before rename so a planted symlink is refused.
    if let Err(error) = reject_ancestor_symlinks(path).and_then(|()| reject_symlink(path)) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(io_err(error));
    }
    set_permissions(path, 0o600)?;
    #[cfg(unix)]
    if mode_of(path)? != 0o600 {
        return Err(ConfigError::Io(
            "api key file permissions are not 0600".to_string(),
        ));
    }
    Ok(())
}

fn write_temp_key(temp: &Path, trimmed: &str) -> Result<(), ConfigError> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(temp).map_err(io_err)?;
    file.write_all(trimmed.as_bytes()).map_err(io_err)?;
    file.flush().map_err(io_err)?;
    Ok(())
}

/// Deletes the stored key; missing files are already deleted.
pub fn delete_api_key(path: &Path) -> Result<(), ConfigError> {
    reject_ancestor_symlinks(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ConfigError::Symlink(path.display().to_string()));
            }
            if metadata.is_dir() {
                return Err(ConfigError::Io(format!(
                    "api key path is a directory: {}",
                    path.display()
                )));
            }
            std::fs::remove_file(path).map_err(io_err)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_err(error)),
    }
}

/// Parses raw terminal bytes into a trimmed key, taking the first line.
pub fn read_api_key_input(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let first_line = text.lines().next().unwrap_or("");
    first_line.trim().trim_matches('\u{0}').trim().to_string()
}
