//! Entry point.
//!
//! The binary owns nothing but startup: everything else is in the `qry`
//! library, and domain logic lives in the `qry-core` crate so it stays
//! testable without a TTY.

use clap::Parser;
use qry::cli::Cli;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    qry::errors::init()?;
    qry::logging::init()?;
    qry::run(Cli::parse()).await
}
