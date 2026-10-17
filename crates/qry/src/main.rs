//! Entry point.
//!
//! The binary owns nothing but startup: everything else is in the `qry`
//! library, and domain logic lives in the `qry-core` crate so it stays
//! testable without a TTY.

use clap::Parser;
use qry::cli::{Cli, Command};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let cli = Cli::parse();
    // `qry generate …` writes a release asset to stdout. It runs before the
    // panic hooks, the log file and the terminal are set up, so it works in
    // a build sandbox with no TTY and no writable home.
    if let Some(Command::Generate { asset }) = cli.command {
        qry::cli::generate(asset, &mut std::io::stdout().lock())?;
        return Ok(());
    }
    qry::errors::init()?;
    qry::logging::init()?;
    qry::run(cli).await
}
