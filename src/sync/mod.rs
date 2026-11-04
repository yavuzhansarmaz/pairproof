//! Mutagen availability checks and session sync management.

pub mod command;
pub mod error;
pub mod failure;
pub mod lifecycle;
pub mod paste;

pub use command::{
    beta_url, describe_status, find_sync, mutagen_version, parse_mutagen_version, parse_sync_list,
    parse_sync_names, parse_sync_template, sync_create, sync_create_args, sync_flush, sync_list,
    sync_list_template, sync_resume, sync_session_name, sync_terminate, validate_alpha_dir,
    SyncSessionInfo, STATUS_TEMPLATE, SYNC_IGNORE_ARGS,
};
pub use error::SyncError;
pub use failure::{classify_ssh_failure, FailureClassification, SshFailureKind};
pub use lifecycle::{
    check_mutagen_available, create_sync, ensure_sync, flush_sync, resume_sync, sync_status,
    terminate_sync, EnsureOutcome, SYNC_TIMEOUT,
};
pub use paste::{
    detect_bulk_insert, PasteFlag, Snapshot, PASTE_BYTE_THRESHOLD, PASTE_LINE_THRESHOLD,
};
