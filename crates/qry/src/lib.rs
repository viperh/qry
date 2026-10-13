//! qry as a library: the application, its components and its config.
//!
//! The binary in `main.rs` only parses the command line, installs the panic
//! and logging hooks and calls [`run`]. Everything else lives here so that
//! integration tests under `crates/qry/tests/` (and the test harness in
//! [`testing`]) can reach it. Domain logic that needs no terminal lives in
//! the `qry-core` crate.

// The library exists for the binary and its tests, not as a public API, so
// its docs are built with `--document-private-items` (the CI `docs` job) and
// may link to crate-private items.
#![allow(
    rustdoc::private_intra_doc_links,
    reason = "docs are built with --document-private-items; see above"
)]

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

// Drives the app without a terminal, for tests. Compiled for this crate's
// own tests and, through the `test-util` feature, for other crates'
// dev-dependencies; a release build never contains it. (The module's own
// `//!` docs are its rustdoc: an outer `///` here would make rustdoc resolve
// their intra-doc links from this file's scope, where they break.)
#[cfg(any(test, feature = "test-util"))]
pub mod testing;

/// Runs the TUI until the user quits.
pub async fn run(cli: cli::Cli) -> color_eyre::Result<()> {
    let mut app = app::App::new(cli.tick_rate, cli.frame_rate)?;
    app.run().await
}
