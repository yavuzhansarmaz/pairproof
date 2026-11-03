//! Candidate connection verification seam.

use crate::connection::ConnectionString;
use crate::tui::candidate::CandidateConnectError;

/// Verifies a parsed connection string without leaking diagnostics.
pub trait ConnectionVerifier {
    /// Returns Ok only when the connection may proceed.
    fn verify(&self, c: &ConnectionString, now_ms: u64) -> Result<(), CandidateConnectError>;
}

impl<F> ConnectionVerifier for F
where
    F: Fn(&ConnectionString, u64) -> Result<(), CandidateConnectError>,
{
    fn verify(&self, c: &ConnectionString, now_ms: u64) -> Result<(), CandidateConnectError> {
        self(c, now_ms)
    }
}

/// Legacy Phase-9 seam: always `Unavailable`, never fakes verification.
/// Live Phase-10 production replaces it via `fresh_verifier`/
/// `SshAuthVerifier` in `candidate_runtime`; crate-internal only, tests
/// exercise it through `CandidateApp::new_unavailable`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UnavailableVerifier;

impl ConnectionVerifier for UnavailableVerifier {
    fn verify(&self, _c: &ConnectionString, _now_ms: u64) -> Result<(), CandidateConnectError> {
        Err(CandidateConnectError::Unavailable)
    }
}
