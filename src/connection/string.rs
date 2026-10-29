use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use rand::rngs::OsRng;
use rand::Rng;

use super::error::ConnectionError;
use super::secret::Secret;

/// A validated host and port. Direct mode targets public IPv4 plus a forwarded port;
/// IPv6 hosts are rejected because `:` is the field separator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    /// Validates an IPv4 address or DNS hostname and a non-zero port (syntactic only).
    /// Use [`Endpoint::public`] for the issuance-grade public-reachability check.
    pub fn new(host: &str, port: u16) -> Result<Self, ConnectionError> {
        if port == 0 {
            return Err(invalid_endpoint("port must not be zero"));
        }
        Ok(Self {
            host: validate_host(host)?,
            port,
        })
    }

    /// Issuance-grade constructor: IPv4 literals must be globally routable.
    /// DNS names are accepted without resolution, except `localhost` and numeric disguises.
    pub fn public(host: &str, port: u16) -> Result<Self, ConnectionError> {
        let endpoint = Self::new(host, port)?;
        let validated = endpoint.host();
        if let Ok(addr) = validated.parse::<Ipv4Addr>() {
            if let Some(reason) = non_public_reason(addr) {
                return Err(invalid_endpoint(format!(
                    "endpoint host is not public: {addr} is {reason}"
                )));
            }
            return Ok(endpoint);
        }
        if validated == "localhost" {
            return Err(invalid_endpoint(
                "endpoint host is not public: localhost is not publicly reachable",
            ));
        }
        if is_numeric_ip_disguise(validated) {
            return Err(invalid_endpoint(format!(
                "endpoint host is not public: {validated} is a numeric IP disguise"
            )));
        }
        Ok(endpoint)
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

fn invalid_endpoint(msg: impl Into<String>) -> ConnectionError {
    ConnectionError::InvalidEndpoint(msg.into())
}

fn validate_host(host: &str) -> Result<String, ConnectionError> {
    if host.is_empty() {
        return Err(invalid_endpoint("host must not be empty"));
    }
    if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid_endpoint("host must not contain whitespace"));
    }
    if host.contains(':') {
        return Err(invalid_endpoint("IPv6 hosts are not supported"));
    }
    let lowered = host.to_ascii_lowercase();
    if lowered.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        let addr: Ipv4Addr = lowered
            .parse()
            .map_err(|_| invalid_endpoint("host is not a valid IPv4 address"))?;
        if addr.is_unspecified() {
            return Err(invalid_endpoint("host must not be 0.0.0.0"));
        }
        return Ok(addr.to_string());
    }
    validate_hostname(&lowered)?;
    Ok(lowered)
}

fn validate_hostname(host: &str) -> Result<(), ConnectionError> {
    if host.len() > 253 {
        return Err(invalid_endpoint("hostname exceeds 253 characters"));
    }
    if host.ends_with('.') {
        return Err(invalid_endpoint("hostname must not end with a dot"));
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(invalid_endpoint("hostname label must be 1-63 characters"));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(invalid_endpoint(
                "hostname label must not start or end with '-'",
            ));
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(invalid_endpoint(
                "hostname labels must contain only [a-z0-9-]",
            ));
        }
    }
    Ok(())
}

/// Returns the non-public reason for `addr`, or `None` if globally routable.
/// `Ipv4Addr::is_global` is unstable on this toolchain; replicated from stable predicates + explicit ranges.
fn non_public_reason(addr: Ipv4Addr) -> Option<&'static str> {
    let octets = addr.octets();
    if addr.is_unspecified() {
        Some("an unspecified address")
    } else if addr.is_loopback() {
        Some("a loopback address")
    } else if addr.is_private() {
        Some("a private address")
    } else if addr.is_link_local() {
        Some("a link-local address")
    } else if addr.is_multicast() {
        Some("a multicast address")
    } else if addr.is_documentation() {
        Some("a documentation address")
    } else if addr.is_broadcast() {
        Some("the broadcast address")
    } else if octets[0] == 0 {
        Some("a \"this host\" (0.0.0.0/8) address")
    } else if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        Some("a shared/CGNAT (100.64.0.0/10) address")
    } else if octets[0] == 198 && (18..=19).contains(&octets[1]) {
        Some("a benchmarking (198.18.0.0/15) address")
    } else if matches!(octets, [192, 0, 0, 9] | [192, 0, 0, 10]) {
        None
    } else if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        Some("a special-purpose (192.0.0.0/24) address")
    } else if octets[0] == 192 && octets[1] == 88 && octets[2] == 99 {
        Some("a deprecated 6to4 relay (192.88.99.0/24) address")
    } else if octets[0] >= 240 {
        Some("a reserved (240.0.0.0/4) address")
    } else {
        None
    }
}

/// True when every label reads as a number: decimal digits or a `0x` hex form.
fn is_numeric_ip_disguise(host: &str) -> bool {
    host.split('.').all(|label| {
        if let Some(hex) = label.strip_prefix("0x") {
            !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
        } else {
            !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit())
        }
    })
}

/// A session-scoped username of the form `pp-` plus 8 chars from `[a-z0-9]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EphemeralUsername {
    value: String,
}

impl EphemeralUsername {
    /// Parses exactly `pp-` followed by 8 characters from `[a-z0-9]`.
    pub fn parse(raw: &str) -> Result<Self, ConnectionError> {
        let ok = raw.len() == 11
            && raw.starts_with("pp-")
            && raw[3..]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if !ok {
            return Err(ConnectionError::InvalidUsername(raw.to_string()));
        }
        Ok(Self {
            value: raw.to_string(),
        })
    }

    /// Generates a random username from the operating system RNG.
    pub fn generate() -> Self {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = OsRng;
        let suffix: String = (0..8)
            .map(|_| char::from(CHARS[rng.gen_range(0..CHARS.len())]))
            .collect();
        Self {
            value: format!("pp-{suffix}"),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// A lowercase hex machine fingerprint of even length, 32 to 128 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    value: String,
}

impl Fingerprint {
    /// Parses lowercase hex of even length between 32 and 128 characters.
    pub fn parse(raw: &str) -> Result<Self, ConnectionError> {
        let ok = (32..=128).contains(&raw.len())
            && raw.len().is_multiple_of(2)
            && raw
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !ok {
            return Err(ConnectionError::InvalidFingerprint(raw.to_string()));
        }
        Ok(Self {
            value: raw.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }
}

/// A versioned, single-line connection string with pre-validated components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionString {
    endpoint: Endpoint,
    username: EphemeralUsername,
    secret: Secret,
    expires_at_ms: u64,
    fingerprint: Fingerprint,
}

impl ConnectionString {
    /// Builds a connection string from already-validated components.
    pub fn new(
        endpoint: Endpoint,
        username: EphemeralUsername,
        secret: Secret,
        expires_at_ms: u64,
        fingerprint: Fingerprint,
    ) -> Self {
        Self {
            endpoint,
            username,
            secret,
            expires_at_ms,
            fingerprint,
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn username(&self) -> &EphemeralUsername {
        &self.username
    }

    pub fn secret(&self) -> &Secret {
        &self.secret
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// Encodes the canonical `pairproof:v1:...` form.
    pub fn encode(&self) -> String {
        format!(
            "pairproof:v1:{}:{}:{}:{}:{}:{}",
            self.endpoint.host(),
            self.endpoint.port(),
            self.username.as_str(),
            self.secret.expose_hex(),
            self.expires_at_ms,
            self.fingerprint.as_str()
        )
    }

    /// Parses the canonical form, rejecting any deviation with a typed error.
    pub fn parse(raw: &str) -> Result<Self, ConnectionError> {
        if raw.is_empty() {
            return Err(ConnectionError::Malformed("input is empty".to_string()));
        }
        if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ConnectionError::Malformed(
                "input must not contain whitespace or control characters".to_string(),
            ));
        }
        let fields: Vec<&str> = raw.split(':').collect();
        if fields.len() != 8 {
            return Err(ConnectionError::Malformed(format!(
                "expected 8 fields, got {}",
                fields.len()
            )));
        }
        if fields[0] != "pairproof" {
            return Err(ConnectionError::Malformed(format!(
                "wrong prefix: {}",
                fields[0]
            )));
        }
        if fields[1] != "v1" {
            return Err(ConnectionError::UnsupportedVersion(fields[1].to_string()));
        }
        let port = parse_port(fields[3])?;
        let endpoint = Endpoint::new(fields[2], port)?;
        let username = EphemeralUsername::parse(fields[4])?;
        let secret = Secret::from_hex(fields[5])?;
        let expires_at_ms = parse_expiry(fields[6])?;
        let fingerprint = Fingerprint::parse(fields[7])?;
        Ok(Self::new(
            endpoint,
            username,
            secret,
            expires_at_ms,
            fingerprint,
        ))
    }
}

fn is_canonical_decimal(raw: &str) -> bool {
    !raw.is_empty()
        && raw.bytes().all(|b| b.is_ascii_digit())
        && (raw.len() == 1 || !raw.starts_with('0'))
}

fn parse_port(raw: &str) -> Result<u16, ConnectionError> {
    if !is_canonical_decimal(raw) {
        return Err(invalid_endpoint(format!("non-canonical port: {raw}")));
    }
    let value: u32 = raw
        .parse()
        .map_err(|_| invalid_endpoint(format!("port out of range: {raw}")))?;
    if value == 0 || value > u32::from(u16::MAX) {
        return Err(invalid_endpoint(format!("port out of range: {raw}")));
    }
    Ok(value as u16)
}

fn parse_expiry(raw: &str) -> Result<u64, ConnectionError> {
    if !is_canonical_decimal(raw) {
        return Err(ConnectionError::InvalidExpiry(raw.to_string()));
    }
    raw.parse()
        .map_err(|_| ConnectionError::InvalidExpiry(raw.to_string()))
}

impl fmt::Display for ConnectionString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.encode())
    }
}

impl FromStr for ConnectionString {
    type Err = ConnectionError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}
