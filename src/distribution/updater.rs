//! Check-only and self-replacing update flows.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::error::DistributionError;
use super::release::{
    asset_url, checksum_url, current_platform_asset, latest_release, verify_checksum,
    ReleaseClient, ReqwestClient,
};
use super::version::{parse_version, Version};

/// Result of a check or an update attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// The installed version is at least as new as the latest stable.
    UpToDate,
    /// The executable was replaced.
    Updated {
        /// Version that was replaced.
        from: Version,
        /// Version now installed.
        to: Version,
    },
    /// No download or replacement was performed.
    CheckOnly {
        /// Whether a newer stable release exists.
        update_available: bool,
        /// Latest stable version seen.
        latest: Version,
    },
}

/// Testable updater core over a [`ReleaseClient`] seam.
pub struct Updater<C: ReleaseClient> {
    client: C,
    exe_path: PathBuf,
}

impl<C: ReleaseClient> Updater<C> {
    /// Creates an updater replacing the binary at `exe_path`.
    pub fn new(client: C, exe_path: PathBuf) -> Self {
        Self { client, exe_path }
    }

    /// Path of the executable this updater would replace.
    pub fn exe_path(&self) -> &Path {
        &self.exe_path
    }

    /// Checks for a newer stable release. Only the metadata endpoint
    /// is requested: never an archive or checksum URL.
    pub fn check(&self, current: &Version) -> Result<UpdateOutcome, DistributionError> {
        let latest = latest_release(&self.client)?;
        Ok(UpdateOutcome::CheckOnly {
            update_available: latest.version > *current,
            latest: latest.version,
        })
    }

    /// Downloads, verifies, and atomically installs a newer release.
    ///
    /// On any error before the final rename the staged temporary file
    /// is removed and the existing executable is left untouched.
    pub fn update(&self, current: &Version) -> Result<UpdateOutcome, DistributionError> {
        let latest = latest_release(&self.client)?;
        if latest.version <= *current {
            return Ok(UpdateOutcome::UpToDate);
        }
        let asset = current_platform_asset()?;
        let archive = self.client.get_bytes(&asset_url(&latest.tag, asset))?;
        let checksum_text = self.client.get_text(&checksum_url(&latest.tag, asset))?;
        verify_checksum(&archive, &checksum_text, asset)?;
        let binary = extract_pairproof(&archive)?;
        self.install(&binary, &latest.version)?;
        Ok(UpdateOutcome::Updated {
            from: current.clone(),
            to: latest.version,
        })
    }

    /// Stages, verifies, and renames the new binary over `exe_path`.
    fn install(&self, binary: &[u8], expected: &Version) -> Result<(), DistributionError> {
        let parent = self.exe_path.parent().ok_or_else(|| {
            DistributionError::CurrentExe(format!(
                "executable path {} has no parent directory",
                self.exe_path.display()
            ))
        })?;
        let temp_path = parent.join(temp_file_name());
        let staged = (|| -> Result<(), DistributionError> {
            std::fs::write(&temp_path, binary)
                .map_err(|err| DistributionError::from_io(&temp_path, err))?;
            set_executable(&temp_path)?;
            verify_new_binary(&temp_path, expected)
        })();
        if let Err(err) = staged {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err);
        }
        if let Err(err) = std::fs::rename(&temp_path, &self.exe_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(DistributionError::from_replace(&self.exe_path, err));
        }
        Ok(())
    }
}

/// Unique staging name in the destination directory, so the final
/// rename stays on one filesystem.
fn temp_file_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(".pairproof-update-{}-{nanos}", std::process::id())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), DistributionError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|err| DistributionError::from_io(path, err))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), DistributionError> {
    Ok(())
}

/// Runs the staged binary with `--version` and requires its output to
/// name the expected version, as clap prints `pairproof X.Y.Z`.
fn verify_new_binary(path: &Path, expected: &Version) -> Result<(), DistributionError> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .map_err(|err| {
            DistributionError::VerificationFailed(format!(
                "could not run {} --version: {err}",
                path.display()
            ))
        })?;
    if !output.status.success() {
        return Err(DistributionError::VerificationFailed(format!(
            "{} --version exited with {}",
            path.display(),
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let wanted = format!("pairproof {expected}");
    if !stdout.contains(&wanted) {
        return Err(DistributionError::VerificationFailed(format!(
            "expected --version output to contain '{wanted}', got '{}'",
            stdout.trim()
        )));
    }
    Ok(())
}

/// Extracts the single regular `pairproof` file from a gzip tarball.
///
/// Directories, extra entries, nested or parent-traversing paths, and
/// links are all rejected.
pub fn extract_pairproof(archive_bytes: &[u8]) -> Result<Vec<u8>, DistributionError> {
    let decoder = flate2::read::GzDecoder::new(archive_bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| DistributionError::InvalidArchive(err.to_string()))?;
    let mut extracted: Option<Vec<u8>> = None;
    for entry in entries {
        let mut entry = entry.map_err(|err| DistributionError::InvalidArchive(err.to_string()))?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() {
            return Err(DistributionError::InvalidArchive(format!(
                "entry is not a regular file: {entry_type:?}"
            )));
        }
        let path = entry
            .path()
            .map_err(|err| DistributionError::InvalidArchive(err.to_string()))?;
        if path != Path::new("pairproof") {
            return Err(DistributionError::InvalidArchive(format!(
                "expected exactly one entry named 'pairproof', found '{}'",
                path.display()
            )));
        }
        if extracted.is_some() {
            return Err(DistributionError::InvalidArchive(
                "archive contains more than one 'pairproof' entry".to_string(),
            ));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(512 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|err| DistributionError::InvalidArchive(err.to_string()))?;
        extracted = Some(bytes);
    }
    extracted.ok_or_else(|| {
        DistributionError::InvalidArchive("archive contains no 'pairproof' file".to_string())
    })
}

/// Production entry: updates (or checks) the running executable.
pub fn run_update(check_only: bool) -> Result<UpdateOutcome, DistributionError> {
    let exe_path =
        std::env::current_exe().map_err(|err| DistributionError::CurrentExe(err.to_string()))?;
    let current = parse_version(env!("CARGO_PKG_VERSION"))?;
    let updater = Updater::new(ReqwestClient, exe_path);
    if check_only {
        updater.check(&current)
    } else {
        updater.update(&current)
    }
}
