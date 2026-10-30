use clap::Parser;

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
}
