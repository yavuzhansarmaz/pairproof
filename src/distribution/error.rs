//! Typed errors for the self-update distribution path.

use std::io;

/// Errors returned by version parsing, release lookup, and updating.
#[derive(Debug, thiserror::Error)]
pub enum DistributionError {
    /// A version string did not follow semantic versioning.
    #[error("invalid version '{0}': expected MAJOR.MINOR.PATCH with optional prerelease")]
    InvalidVersion(String),

    /// A release tag did not match `vMAJOR.MINOR.PATCH`.
    #[error("invalid release tag '{0}': expected vMAJOR.MINOR.PATCH")]
    InvalidReleaseTag(String),

    /// The latest-stable endpoint returned a prerelease tag.
    #[error("release tag '{0}' is a prerelease, not a stable release")]
    PrereleaseTag(String),

    /// Release metadata could not be parsed.
    #[error("invalid release metadata: {0}")]
    InvalidRelease(String),

    /// A checksum file did not follow `<64hex><two spaces><filename>`.
    #[error("invalid checksum file for '{asset}': {reason}")]
    InvalidChecksum {
        /// Asset the checksum file was expected to name.
        asset: String,
        /// Why the checksum file was rejected.
        reason: String,
    },

    /// The downloaded archive bytes did not match the recorded checksum.
    #[error("checksum mismatch for '{asset}': recorded {expected}, computed {actual}")]
    ChecksumMismatch {
        /// Asset whose checksum mismatched.
        asset: String,
        /// Hex digest recorded in the checksum file.
        expected: String,
        /// Hex digest computed over the downloaded bytes.
        actual: String,
    },

    /// The current operating system or architecture has no release asset.
    #[error("unsupported platform '{os}/{arch}': supported platforms are linux and darwin on x86_64 or aarch64/arm64")]
    UnsupportedPlatform {
        /// Operating system name that was not supported.
        os: String,
        /// Architecture name that was not supported.
        arch: String,
    },

    /// An HTTP request failed before a response body was available.
    #[error("failed to fetch {url}: {message}")]
    Http {
        /// URL that was being fetched.
        url: String,
        /// Transport-level failure description.
        message: String,
    },

    /// An HTTP request returned a non-success status.
    #[error("failed to fetch {url}: HTTP status {status}")]
    HttpStatus {
        /// URL that was being fetched.
        url: String,
        /// HTTP status code returned by the server.
        status: u16,
    },

    /// A response exceeded its size limit.
    #[error("response from {url} exceeded the {limit}-byte limit")]
    TooLarge {
        /// URL whose response was too large.
        url: String,
        /// Maximum accepted size in bytes.
        limit: u64,
    },

    /// The downloaded archive was not the expected single-file tarball.
    #[error("invalid release archive: {0}")]
    InvalidArchive(String),

    /// The extracted replacement binary failed verification.
    #[error("new binary verification failed: {0}")]
    VerificationFailed(String),

    /// The atomic replacement of the current executable failed.
    #[error("failed to replace executable at {path}: {source}")]
    ReplaceFailed {
        /// Destination path that could not be replaced.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// A filesystem operation was denied.
    #[error("permission denied for {path}: {source}. Check that the path is writable by the current user and re-run `pairproof update`")]
    PermissionDenied {
        /// Path that could not be accessed.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// Any other filesystem failure on the update path.
    #[error("filesystem error for {path}: {source}")]
    Io {
        /// Path involved in the failed operation.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The path of the running executable could not be determined.
    #[error("could not determine the current executable path: {0}")]
    CurrentExe(String),

    /// A planned uninstall path failed safety validation.
    #[error("refusing to remove unsafe path '{path}': {reason}")]
    UnsafePath {
        /// Path that was rejected.
        path: String,
        /// Why the path was rejected.
        reason: String,
    },

    /// Uninstall was not confirmed.
    #[error("uninstall aborted: confirmation was not given")]
    UninstallAborted,

    /// Uninstall needs `--yes` outside an interactive terminal.
    #[error("refusing to uninstall without confirmation in non-interactive mode; pass --yes")]
    NonInteractiveConfirmation,

    /// No home directory was available for uninstall.
    #[error("could not determine the home directory for uninstall: HOME is not set")]
    MissingHome,
}

impl DistributionError {
    /// Maps an I/O error, giving permission failures an actionable message.
    pub(crate) fn from_io(path: &std::path::Path, source: io::Error) -> Self {
        let path = path.display().to_string();
        if source.kind() == io::ErrorKind::PermissionDenied {
            Self::PermissionDenied { path, source }
        } else {
            Self::Io { path, source }
        }
    }

    /// Maps an I/O error from the final rename step.
    pub(crate) fn from_replace(path: &std::path::Path, source: io::Error) -> Self {
        let path = path.display().to_string();
        if source.kind() == io::ErrorKind::PermissionDenied {
            Self::PermissionDenied { path, source }
        } else {
            Self::ReplaceFailed { path, source }
        }
    }
}
