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

#[test]
fn print_config_is_a_flag() {
    assert!(!Cli::try_parse_from(["qry"]).unwrap().print_config);
    assert!(
        Cli::try_parse_from(["qry", "--print-config"])
            .unwrap()
            .print_config
    );
}

#[test]
fn help_output_snapshot() {
    // A fixed width: with `wrap_help`, clap otherwise wraps to the
    // terminal running the tests.
    let help = Cli::command().term_width(100).render_long_help();
    insta::assert_snapshot!("cli_help", help.to_string());
}

#[test]
fn generate_parses_man_and_completions() {
    let cli = Cli::try_parse_from(["qry", "generate", "man"]).unwrap();
    assert_eq!(cli.command, Some(Command::Generate { asset: Asset::Man }));
    let cli = Cli::try_parse_from(["qry", "generate", "completions", "zsh"]).unwrap();
    assert_eq!(
        cli.command,
        Some(Command::Generate {
            asset: Asset::Completions { shell: Shell::Zsh }
        })
    );
    assert!(Cli::try_parse_from(["qry", "generate", "completions", "tcsh"]).is_err());
    assert_eq!(Cli::try_parse_from(["qry"]).unwrap().command, None);
}

#[test]
fn generate_is_hidden_from_the_help() {
    let help = Cli::command().render_long_help().to_string();
    assert!(!help.contains("generate"), "{help}");
}

fn generated(asset: Asset) -> String {
    let mut out = Vec::new();
    generate(asset, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn generate_man_contains_the_environment_section() {
    let man = generated(Asset::Man);
    assert!(man.contains(".TH qry 1"), "{man}");
    assert!(man.contains(".SH ENVIRONMENT"), "{man}");
    for var in [
        "QRY_CONFIG",
        "QRY_DATA",
        "QRY_LOG_LEVEL",
        "QRY_KEYRING",
        "NO_COLOR",
        "COLORTERM",
    ] {
        assert!(man.contains(&format!("\\fB{var}\\fR")), "{var} missing");
    }
    // ENVIRONMENT sits between OPTIONS and VERSION.
    let options = man.find(".SH OPTIONS").unwrap();
    let environment = man.find(".SH ENVIRONMENT").unwrap();
    let version = man.find(".SH VERSION").unwrap();
    assert!(options < environment && environment < version);
}

#[test]
fn generate_man_is_the_same_on_every_machine() {
    let man = generated(Asset::Man);
    // The plain version, not `--version`'s text with this machine's
    // directories in it, and no subcommand in the synopsis.
    assert!(
        man.contains(concat!("v", env!("CARGO_PKG_VERSION"))),
        "{man}"
    );
    assert!(!man.contains("Config directory"), "{man}");
    assert!(!man.contains("subcommands"), "{man}");
    assert!(!man.contains("generate"), "{man}");
}

#[test]
fn generate_completions_for_every_shell() {
    for shell in [
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::PowerShell,
        Shell::Elvish,
    ] {
        let script = generated(Asset::Completions { shell });
        assert!(!script.is_empty(), "{shell}: empty");
        assert!(script.contains("qry"), "{shell}: {script}");
        assert!(script.contains("print-config"), "{shell}: {script}");
        assert!(!script.contains("generate"), "{shell}: {script}");
    }
}
