//! Unit tests for `cli.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pretty_assertions::assert_eq;

use super::*;
use crate::theme::{self, ColorSupport};

#[test]
fn theme_flag_is_optional() {
    let cli = Cli::try_parse_from(["qry"]).unwrap();
    assert_eq!(cli.theme, None);
    let cli = Cli::try_parse_from(["qry", "--theme", "dark"]).unwrap();
    assert_eq!(cli.theme.as_deref(), Some("dark"));
}

#[test]
fn unknown_theme_is_an_error_listing_names() {
    let err = Cli::try_parse_from(["qry", "--theme", "solarized"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue);
    let message = err.to_string();
    assert!(message.contains("solarized"), "{message}");
    for name in Theme::NAMES {
        assert!(message.contains(name), "{name} missing from: {message}");
    }
}

#[test]
fn cli_theme_overrides_config() {
    let cli = Cli::try_parse_from(["qry", "--theme", "dark"]).unwrap();
    // The flag wins, so the config's unknown name is never looked at.
    let theme = theme::resolve(
        cli.theme.as_deref(),
        Some("no-such-theme"),
        ColorSupport::TrueColor,
    )
    .unwrap();
    assert_eq!(theme, Theme::DARK);
}

#[test]
fn version_shows_the_colour_support() {
    let version = version();
    let line = version
        .lines()
        .find(|l| l.starts_with("Colour support: "))
        .unwrap();
    assert_eq!(line, format!("Colour support: {}", ColorSupport::detect()));
}
