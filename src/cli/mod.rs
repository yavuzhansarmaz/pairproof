pub mod args;

use clap::Parser;

/// Parses arguments and hands control to role dispatch.
pub fn run() -> anyhow::Result<()> {
    let _args = args::Args::parse();
    println!("pairproof: role selection is not implemented yet");
    Ok(())
}
