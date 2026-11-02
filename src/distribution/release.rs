//! Release metadata, platform assets, checksums, and the HTTP seam.

use std::time::Duration;

use sha2::{Digest, Sha256};

use super::error::DistributionError;
use super::version::{parse_version, Version};

/// GitHub API endpoint for the latest stable release.
pub const REPO_API_LATEST: &str =
    "https://api.github.com/repos/yavuzhansarmaz/pairproof/releases/latest";

/// Base URL under which release assets are downloaded.
pub const DOWNLOAD_BASE: &str = "https://github.com/yavuzhansarmaz/pairproof/releases/download";

const METADATA_LIMIT: u64 = 1024 * 1024;
const ARCHIVE_LIMIT: u64 = 512 * 1024 * 1024;

/// Maps an OS/architecture pair to its exact release asset name.
pub fn platform_asset(os: &str, arch: &str) -> Result<&'static str, DistributionError> {
    let normalized_arch = match arch {
        "arm64" => "aarch64",
        other => other,
    };
    match (os, normalized_arch) {
        ("linux", "x86_64") => Ok("pairproof-linux-x86_64-unknown-linux-musl.tar.gz"),
        ("linux", "aarch64") => Ok("pairproof-linux-aarch64-unknown-linux-musl.tar.gz"),
        ("darwin", "x86_64") => Ok("pairproof-darwin-x86_64-apple-darwin.tar.gz"),
        ("darwin", "aarch64") => Ok("pairproof-darwin-aarch64-apple-darwin.tar.gz"),
        _ => Err(DistributionError::UnsupportedPlatform {
            os: os.to_string(),
            arch: arch.to_string(),
        }),
    }
}

/// Release asset for the platform this binary was built for.
pub fn current_platform_asset() -> Result<&'static str, DistributionError> {
    platform_asset(std::env::consts::OS, std::env::consts::ARCH)
}

/// Download URL for one release asset.
pub fn asset_url(tag: &str, asset: &str) -> String {
    format!("{DOWNLOAD_BASE}/{tag}/{asset}")
}

/// Download URL for an asset's checksum file.
pub fn checksum_url(tag: &str, asset: &str) -> String {
    format!("{}.sha256", asset_url(tag, asset))
}

/// A validated latest-stable release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    /// Full tag, including the leading `v`.
    pub tag: String,
    /// Version parsed from the tag.
    pub version: Version,
}

/// Parses and validates `tag_name` from latest-release JSON.
///
/// Only stable `vMAJOR.MINOR.PATCH` tags are accepted; prerelease tags
/// are rejected because this endpoint must name the latest stable.
pub fn parse_release_tag(json: &str) -> Result<ReleaseInfo, DistributionError> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|err| DistributionError::InvalidRelease(err.to_string()))?;
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            DistributionError::InvalidRelease("missing string field 'tag_name'".to_string())
        })?;
    validate_stable_tag(tag)?;
    let version =
        parse_version(tag).map_err(|_| DistributionError::InvalidReleaseTag(tag.to_string()))?;
    Ok(ReleaseInfo {
        tag: tag.to_string(),
        version,
    })
}

/// Manual equivalent of `^v\d+\.\d+\.\d+(-prerelease)?$`, with
/// prerelease tags rejected as latest-stable targets.
fn validate_stable_tag(tag: &str) -> Result<(), DistributionError> {
    let invalid = || DistributionError::InvalidReleaseTag(tag.to_string());
    let rest = tag.strip_prefix('v').ok_or_else(invalid)?;
    let (core, prerelease) = match rest.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (rest, None),
    };
    let mut parts = core.split('.');
    for _ in 0..3 {
        let part = parts.next().ok_or_else(invalid)?;
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
    }
    if parts.next().is_some() {
        return Err(invalid());
    }
    if let Some(pre) = prerelease {
        if pre.is_empty()
            || !pre.split('.').all(|id| {
                !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
        {
            return Err(invalid());
        }
        return Err(DistributionError::PrereleaseTag(tag.to_string()));
    }
    Ok(())
}

/// Parses a strict checksum file: `<64hex><two spaces><filename>` on
/// exactly one line, naming exactly `expected_asset`.
pub fn parse_checksum(text: &str, expected_asset: &str) -> Result<String, DistributionError> {
    let invalid = |reason: &str| DistributionError::InvalidChecksum {
        asset: expected_asset.to_string(),
        reason: reason.to_string(),
    };
    let body = text.strip_suffix('\n').unwrap_or(text);
    let body = body.strip_suffix('\r').unwrap_or(body);
    if body.is_empty() || body.contains('\n') || body.contains('\r') {
        return Err(invalid("expected exactly one line"));
    }
    let bytes = body.as_bytes();
    if bytes.len() < 67 {
        return Err(invalid("expected '<64 hex><two spaces><filename>'"));
    }
    if !bytes[..64].iter().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("checksum is not 64 hexadecimal characters"));
    }
    if &bytes[64..66] != b"  " {
        return Err(invalid("expected two spaces after the checksum"));
    }
    let filename = std::str::from_utf8(&bytes[66..])
        .map_err(|_| invalid("checksum names a different file"))?;
    if filename != expected_asset {
        return Err(invalid("checksum names a different file"));
    }
    let hex = std::str::from_utf8(&bytes[..64])
        .map_err(|_| invalid("checksum is not 64 hexadecimal characters"))?;
    Ok(hex.to_ascii_lowercase())
}

/// Computes the SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Verifies `bytes` against a strict checksum file for `asset`.
pub fn verify_checksum(
    bytes: &[u8],
    checksum_text: &str,
    asset: &str,
) -> Result<(), DistributionError> {
    let expected = parse_checksum(checksum_text, asset)?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(DistributionError::ChecksumMismatch {
            asset: asset.to_string(),
            expected,
            actual,
        });
    }
    Ok(())
}

/// Synchronous HTTP seam so tests can substitute a recording fake.
pub trait ReleaseClient {
    /// Fetches a small text resource such as release metadata.
    fn get_text(&self, url: &str) -> Result<String, DistributionError>;
    /// Fetches a binary resource such as a release archive.
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, DistributionError>;
}

/// Fetches and validates the latest stable release via `client`.
pub fn latest_release(client: &dyn ReleaseClient) -> Result<ReleaseInfo, DistributionError> {
    let json = client.get_text(REPO_API_LATEST)?;
    parse_release_tag(&json)
}

/// Production [`ReleaseClient`] backed by reqwest.
///
/// Each call runs the async client on a small current-thread runtime,
/// keeping the trait synchronous for the sync CLI entry point. No
/// credentials are ever sent: only a product `User-Agent`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReqwestClient;

impl ReqwestClient {
    fn fetch(&self, url: &str, limit: u64) -> Result<Vec<u8>, DistributionError> {
        let url_owned = url.to_string();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| DistributionError::Http {
                url: url_owned.clone(),
                message: err.to_string(),
            })?;
        runtime.block_on(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .user_agent(concat!("pairproof/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|err| DistributionError::Http {
                    url: url_owned.clone(),
                    message: err.to_string(),
                })?;
            let mut request = client.get(&url_owned);
            if url_owned.contains("api.github.com") {
                request = request.header("Accept", "application/vnd.github+json");
            }
            let response = request
                .send()
                .await
                .map_err(|err| DistributionError::Http {
                    url: url_owned.clone(),
                    message: err.to_string(),
                })?;
            let status = response.status();
            if !status.is_success() {
                return Err(DistributionError::HttpStatus {
                    url: url_owned.clone(),
                    status: status.as_u16(),
                });
            }
            if let Some(length) = response.content_length() {
                if length > limit {
                    return Err(DistributionError::TooLarge {
                        url: url_owned.clone(),
                        limit,
                    });
                }
            }
            let bytes = response
                .bytes()
                .await
                .map_err(|err| DistributionError::Http {
                    url: url_owned.clone(),
                    message: err.to_string(),
                })?;
            if bytes.len() as u64 > limit {
                return Err(DistributionError::TooLarge {
                    url: url_owned.clone(),
                    limit,
                });
            }
            Ok(bytes.to_vec())
        })
    }
}

impl ReleaseClient for ReqwestClient {
    fn get_text(&self, url: &str) -> Result<String, DistributionError> {
        let bytes = self.fetch(url, METADATA_LIMIT)?;
        String::from_utf8(bytes).map_err(|err| DistributionError::Http {
            url: url.to_string(),
            message: format!("response was not valid UTF-8: {err}"),
        })
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, DistributionError> {
        self.fetch(url, ARCHIVE_LIMIT)
    }
}
