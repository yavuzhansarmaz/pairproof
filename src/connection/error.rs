/// Errors returned by connection-string and secret-lifecycle operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionError {
    #[error("malformed connection string: {0}")]
    Malformed(String),
    #[error("unsupported connection string version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("invalid ephemeral username: {0}")]
    InvalidUsername(String),
    #[error("invalid secret")]
    InvalidSecret,
    #[error("invalid fingerprint: {0}")]
    InvalidFingerprint(String),
    #[error("invalid expiry: {0}")]
    InvalidExpiry(String),
    #[error("wrong secret")]
    WrongSecret,
    #[error("connection string expired")]
    Expired,
    #[error("connection string already locked")]
    AlreadyLocked,
    #[error("fingerprint mismatch")]
    FingerprintMismatch,
    #[error("endpoint mismatch")]
    EndpointMismatch,
    #[error("username mismatch")]
    UsernameMismatch,
    #[error("session ended")]
    SessionEnded,
}
