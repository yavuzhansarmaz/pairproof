//! Self-update distribution: release lookup, verification, replacement.

pub mod error;
pub mod release;
pub mod startup;
pub mod uninstall;
pub mod updater;
pub mod version;

pub use error::DistributionError;
pub use release::{ReleaseClient, ReleaseInfo, ReqwestClient};
pub use startup::{
    ensure_latest_before_tui, reexec_args, should_run_startup_check,
    should_run_startup_check_from_env, startup_check, startup_gate, startup_gate_with,
    ExecReexecer, Gate, Reexecer, StartupGate, StartupOutcome, REEXEC_GUARD_ENV,
};
pub use uninstall::{build_plan, execute_plan, PlanEntry, PlanKind, RemovalPlan};
pub use updater::{UpdateOutcome, Updater};
pub use version::{parse_version, Version};

/// Runs `pairproof update [--check]` and prints the outcome.
pub fn run_cli(check_only: bool) -> anyhow::Result<()> {
    let outcome = updater::run_update(check_only)?;
    match outcome {
        UpdateOutcome::UpToDate => println!("PairProof is already up to date."),
        UpdateOutcome::Updated { from, to } => {
            println!("Updated PairProof from {from} to {to}.")
        }
        UpdateOutcome::CheckOnly {
            update_available: true,
            latest,
        } => println!(
            "A new version of PairProof is available: {latest}. Run `pairproof update` to install it."
        ),
        UpdateOutcome::CheckOnly {
            update_available: false,
            ..
        } => println!("PairProof is already up to date."),
    }
    Ok(())
}

/// Runs `pairproof uninstall [--yes] [--keep-data]`.
pub fn run_uninstall_cli(yes: bool, keep_data: bool) -> anyhow::Result<()> {
    use std::io::{BufRead, IsTerminal, Write};
    let is_tty = std::io::stdin().is_terminal();
    let confirm = || -> bool {
        print!("Type 'yes' to confirm: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(_) => line.trim_end_matches(['\r', '\n']) == "yes",
            Err(_) => false,
        }
    };
    let mut out = std::io::stdout().lock();
    uninstall::run_uninstall(yes, keep_data, is_tty, &confirm, &mut out)?;
    Ok(())
}
