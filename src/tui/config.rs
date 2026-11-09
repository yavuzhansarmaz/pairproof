//! Secure API-key storage with explicit paths; never logs key material.

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors from API-key storage; messages never contain the key.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// No usable key from environment or stored file.
    #[error("provider api key is missing; set PAIRPROOF_API_KEY or the provider key env (e.g. MODEL_API_KEY) or save a key")]
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

fn clean_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Env key for the selected provider: `PAIRPROOF_API_KEY` wins, then the
/// provider-specific env (for meta that is `MODEL_API_KEY`). The stored
/// key file holds the current provider's key and is reused as fallback.
fn env_key() -> Option<String> {
    if let Some(key) = clean_env(crate::llm::PROVIDER_API_KEY_ENV) {
        return Some(key);
    }
    let preset = crate::llm::ProviderConfig::preset_from_env().ok()?;
    preset.api_key_env().and_then(clean_env)
}

/// Provider-aware variant of [`env_key`] for an explicit provider id.
pub fn env_key_for_provider(provider: &str) -> Option<String> {
    if let Some(key) = clean_env(crate::llm::PROVIDER_API_KEY_ENV) {
        return Some(key);
    }
    crate::llm::provider_preset(provider)
        .and_then(|p| p.api_key_env)
        .and_then(clean_env)
}

/// Returns true when a non-blank provider key env var is set.
pub fn env_override_active() -> bool {
    env_key().is_some()
}

/// Returns true when an env key is set for an explicit provider id.
pub fn env_override_active_for_provider(provider: &str) -> bool {
    env_key_for_provider(provider).is_some()
}

fn normalize_provider(provider: &str) -> String {
    let normalized = provider.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        crate::llm::META_PROVIDER.to_string()
    } else {
        normalized
    }
}

/// Legacy (untagged) stored keys were only ever written for Meta.
const LEGACY_PROVIDER: &str = crate::llm::META_PROVIDER;

/// Parses stored content into `(provider, key)`. Tagged form is
/// `provider:<id>\n<key>`; anything else is legacy and maps to Meta.
fn parse_stored(raw: &str) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("provider:") {
        let mut lines = rest.lines();
        if let (Some(provider), Some(key)) = (lines.next(), lines.next()) {
            let key = key.trim();
            if !provider.trim().is_empty() && !key.is_empty() {
                return Some((normalize_provider(provider), key.to_string()));
            }
        }
    }
    Some((LEGACY_PROVIDER.to_string(), trimmed.to_string()))
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

    /// Reads the stored key only when it was saved for `provider`.
    pub fn stored_for_provider(&self, provider: &str) -> Result<Option<String>, ConfigError> {
        stored_api_key_for_provider(&self.path, provider)
    }

    /// Resolves the effective key for an explicit provider id.
    pub fn resolve_for_provider(&self, provider: &str) -> Result<String, ConfigError> {
        resolve_api_key_for_provider(&self.path, provider)
    }

    /// Saves a key tagged with the provider it belongs to.
    pub fn save_for_provider(&self, provider: &str, key: &str) -> Result<(), ConfigError> {
        save_api_key_for_provider(&self.path, provider, key)
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

fn read_stored(path: &Path) -> Result<Option<(String, String)>, ConfigError> {
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
    Ok(parse_stored(&raw))
}

/// Reads the stored key, returning None when missing or blank.
pub fn stored_api_key(path: &Path) -> Result<Option<String>, ConfigError> {
    Ok(read_stored(path)?.map(|(_, key)| key))
}

/// Provider recorded with the stored key (legacy files report `meta`).
pub fn stored_api_key_provider(path: &Path) -> Result<Option<String>, ConfigError> {
    Ok(read_stored(path)?.map(|(provider, _)| provider))
}

/// Reads the stored key only when its recorded provider matches
/// `provider`; a mismatch is treated as no stored key, never reused.
pub fn stored_api_key_for_provider(
    path: &Path,
    provider: &str,
) -> Result<Option<String>, ConfigError> {
    let wanted = normalize_provider(provider);
    Ok(read_stored(path)?.and_then(|(recorded, key)| (recorded == wanted).then_some(key)))
}

/// Resolves the effective key for an explicit provider: a non-blank
/// env key for that provider wins, then a stored key recorded for
/// that same provider; otherwise [`ConfigError::MissingKey`].
pub fn resolve_api_key_for_provider(path: &Path, provider: &str) -> Result<String, ConfigError> {
    if let Some(key) = env_key_for_provider(provider) {
        return Ok(key);
    }
    match stored_api_key_for_provider(path, provider)? {
        Some(key) => Ok(key),
        None => Err(ConfigError::MissingKey),
    }
}

/// Resolves the effective key for the provider selected in the
/// environment: env wins over a provider-matching stored key. An
/// invalid selected provider never falls back to a stored key.
pub fn resolve_api_key(path: &Path) -> Result<String, ConfigError> {
    match crate::llm::ProviderConfig::preset_from_env() {
        Ok(preset) => resolve_api_key_for_provider(path, &preset.provider),
        Err(_) => Err(ConfigError::MissingKey),
    }
}

/// Saves a trimmed key atomically with private permissions on Unix.
pub fn save_api_key(path: &Path, key: &str) -> Result<(), ConfigError> {
    save_api_key_contents(path, key)
}

/// Saves a key tagged with `provider` so it is only reused for it.
pub fn save_api_key_for_provider(
    path: &Path,
    provider: &str,
    key: &str,
) -> Result<(), ConfigError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::EmptyKey);
    }
    let contents = format!("provider:{}\n{trimmed}", normalize_provider(provider));
    save_api_key_contents(path, &contents)
}

fn save_api_key_contents(path: &Path, contents: &str) -> Result<(), ConfigError> {
    let trimmed = contents.trim();
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
