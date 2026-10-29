//! Disposable Docker sandbox lifecycle and in-container test execution.

pub mod command;
pub mod error;
pub mod exec;
pub mod image;
pub mod lifecycle;
pub mod parse;
pub mod preflight;
pub mod process;
pub mod validate;
pub mod workspace;

pub use error::{bounded_stderr, SandboxError};
pub use exec::{
    check_runnability, run_in_container, run_tests, runnability_argv, test_argv, timeout_argv,
    RunnabilityReport, TestOutcome,
};
pub use image::{
    check_docker_available, effective_image_tag, embedded_build_context, ensure_image,
    image_present,
};
pub use lifecycle::{
    apply_network_lockdown, create_and_provision, create_network, destroy, destroy_by_session_id,
    provision_database_user, provision_user, read_fingerprint, rotate_secret, start_container,
    wait_for_host_key, SandboxHandle,
};
pub use preflight::{
    CheckKind, CheckStatus, IpClass, PreflightCheck, PreflightInputs, PreflightReport,
};
pub use process::{CommandOutcome, CommandSpec};
pub use validate::WorkspacePath;
pub use workspace::{read_file, write_file, write_question};
