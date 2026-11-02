//! Semantic version parsing and precedence, without a semver crate.

use std::cmp::Ordering;

use super::error::DistributionError;

/// One prerelease identifier: numeric identifiers compare below
/// alphanumeric ones and numerically among themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleaseId {
    Numeric(u64),
    Alphanumeric(String),
}

impl Ord for PrereleaseId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::Numeric(_), Self::Alphanumeric(_)) => Ordering::Less,
            (Self::Alphanumeric(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Alphanumeric(a), Self::Alphanumeric(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PrereleaseId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A parsed semantic version. Build metadata is accepted and ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Vec<PrereleaseId>,
}

impl Version {
    /// Major version component.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// Minor version component.
    pub fn minor(&self) -> u64 {
        self.minor
    }

    /// Patch version component.
    pub fn patch(&self) -> u64 {
        self.patch
    }

    /// Whether this version carries a prerelease suffix.
    pub fn is_prerelease(&self) -> bool {
        !self.prerelease.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.prerelease.is_empty() {
            let parts: Vec<String> = self
                .prerelease
                .iter()
                .map(|id| match id {
                    PrereleaseId::Numeric(n) => n.to_string(),
                    PrereleaseId::Alphanumeric(s) => s.clone(),
                })
                .collect();
            write!(f, "-{}", parts.join("."))?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| compare_prerelease(&self.prerelease, &other.prerelease))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A release outranks each of its prereleases; when one prerelease list
/// is a prefix of the other, the shorter list has lower precedence.
fn compare_prerelease(a: &[PrereleaseId], b: &[PrereleaseId]) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (left, right) in a.iter().zip(b.iter()) {
                match left.cmp(right) {
                    Ordering::Equal => continue,
                    order => return order,
                }
            }
            a.len().cmp(&b.len())
        }
    }
}

/// Parses a semantic version, accepting an optional leading `v` and
/// ignoring build metadata after `+`.
pub fn parse_version(input: &str) -> Result<Version, DistributionError> {
    let invalid = || DistributionError::InvalidVersion(input.to_string());
    let trimmed = input.trim();
    let without_v = trimmed.strip_prefix('v').unwrap_or(trimmed);
    if without_v.is_empty() {
        return Err(invalid());
    }
    let (core_and_pre, _build) = match without_v.split_once('+') {
        Some((head, build)) => {
            if build.is_empty()
                || !build.split('.').all(|id| {
                    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                })
            {
                return Err(invalid());
            }
            (head, build)
        }
        None => (without_v, ""),
    };
    let (core, prerelease_text) = match core_and_pre.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (core_and_pre, None),
    };
    let mut core_parts = core.split('.');
    let major = parse_core_number(core_parts.next(), input)?;
    let minor = parse_core_number(core_parts.next(), input)?;
    let patch = parse_core_number(core_parts.next(), input)?;
    if core_parts.next().is_some() {
        return Err(invalid());
    }
    let mut prerelease = Vec::new();
    if let Some(text) = prerelease_text {
        if text.is_empty() {
            return Err(invalid());
        }
        for identifier in text.split('.') {
            if identifier.is_empty()
                || !identifier
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return Err(invalid());
            }
            if identifier.chars().all(|c| c.is_ascii_digit()) {
                if identifier.len() > 1 && identifier.starts_with('0') {
                    return Err(invalid());
                }
                let value = identifier.parse::<u64>().map_err(|_| invalid())?;
                prerelease.push(PrereleaseId::Numeric(value));
            } else {
                prerelease.push(PrereleaseId::Alphanumeric(identifier.to_string()));
            }
        }
    }
    Ok(Version {
        major,
        minor,
        patch,
        prerelease,
    })
}

/// Parses one core component, rejecting leading zeroes and overflow.
fn parse_core_number(part: Option<&str>, input: &str) -> Result<u64, DistributionError> {
    let invalid = || DistributionError::InvalidVersion(input.to_string());
    let text = part.ok_or_else(invalid)?;
    if text.is_empty()
        || !text.chars().all(|c| c.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return Err(invalid());
    }
    text.parse::<u64>().map_err(|_| invalid())
}
