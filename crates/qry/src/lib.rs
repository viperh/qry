//! qry as a library: the application, its components and its config.
//!
//! The binary in `main.rs` only parses the command line, installs the panic
//! and logging hooks and calls [`run`]. Everything else lives here so that
//! integration tests under `crates/qry/tests/` (and the test harness in
//! [`testing`]) can reach it. Domain logic that needs no terminal lives in
//! the `qry-core` crate.

pub mod action;
pub mod app;
pub mod cli;
pub mod components;
pub mod config;
pub mod connections;
pub mod db;
pub mod errors;
pub mod import;
pub mod keymap;
pub mod logging;
pub mod secrets;
pub mod tui;

/// Runs the TUI until the user quits.
pub async fn run(cli: cli::Cli) -> color_eyre::Result<()> {
    let mut app = app::App::new(cli.tick_rate, cli.frame_rate)?;
    app.run().await
}
