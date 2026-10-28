mod ai;
mod cli;
mod connection;
mod core;
mod ingest;
mod leetcode;
mod sandbox;
mod scoring;
mod sync;
mod tui;

fn main() -> anyhow::Result<()> {
    cli::run()
}
