//! Expert session identity and sandbox port selection.

use std::ops::Range;

use crate::sandbox::preflight::{first_free_port, port_is_free};

/// Sandbox host-port scan range: 2222 inclusive to 2322 exclusive, as
/// historically scanned by `build_expert_runtime`. First free port wins.
pub const SANDBOX_PORT_RANGE: Range<u16> = 2222..2322;

/// Formats the expert session id as `e{pid}x{now}`.
pub fn format_session_id(pid: u32, now: u64) -> String {
    format!("e{pid}x{now}")
}

/// Selects the sandbox port the expert runtime will use: the first free
/// port in `SANDBOX_PORT_RANGE`. Preflight scans identically via this
/// helper so it checks the same selection the runtime will use.
pub(crate) fn select_sandbox_port() -> Option<u16> {
    first_free_port(SANDBOX_PORT_RANGE, port_is_free)
}
