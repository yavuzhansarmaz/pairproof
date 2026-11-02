use clap::{Parser, Subcommand};

use crate::tui::shared::Role;

fn parse_cli_role(value: &str) -> Result<Role, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "expert" => Ok(Role::Expert),
        "candidate" => Ok(Role::Candidate),
        _ => Err(format!(
            "invalid role '{value}': expected expert or candidate"
        )),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "pairproof",
    version,
    about = "AI-native technical interviewing"
)]
pub struct Args {
    /// Role to run as: expert or candidate.
    #[arg(long = "role", value_name = "ROLE", value_parser = parse_cli_role)]
    pub role: Option<Role>,

    /// Subcommand to run instead of the interactive TUI.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Non-interactive subcommands. When a subcommand is present it is
/// handled by `tui::run` and the TUI is never entered, even if `--role`
/// was also supplied.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Update pairproof to the latest stable release.
    Update(UpdateArgs),
    /// Remove the pairproof binary and its saved data.
    Uninstall(UninstallArgs),
}

#[derive(Debug, clap::Args)]
pub struct UpdateArgs {
    /// Only check whether an update is available; download nothing.
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, clap::Args)]
pub struct UninstallArgs {
    /// Skip the confirmation prompt.
    #[arg(long)]
    pub yes: bool,
    /// Remove only the binary; keep the saved API key and config.
    #[arg(long = "keep-data")]
    pub keep_data: bool,
}

/// Dispatches a non-interactive subcommand; the TUI is never entered.
pub fn run_command(command: &Command) -> anyhow::Result<()> {
    match command {
        Command::Update(update) => crate::distribution::run_cli(update.check),
        Command::Uninstall(uninstall) => {
            crate::distribution::run_uninstall_cli(uninstall.yes, uninstall.keep_data)
        }
    }
}
