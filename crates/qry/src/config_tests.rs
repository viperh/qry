//! Unit tests for `config.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pretty_assertions::assert_eq;

use super::*;
use crate::keymap::Lookup;

/// What `chord` runs in `context` alone.
fn bound(config: &Config, context: KeyContext, chord: &str) -> Option<Action> {
    match config
        .keymap
        .resolve(&[context], &parse_key_sequence(chord).unwrap())
    {
        Lookup::Match(action) => Some(action),
        Lookup::Prefix | Lookup::None => None,
    }
}

#[test]
fn the_embedded_config_names_the_dark_theme() {
    let c = Config::embedded();
    assert_eq!(c.theme.as_deref(), Some("dark"));
    assert!(c.warnings().is_empty(), "{:?}", c.warnings());
}

#[test]
fn theme_key_is_read() {
    let theme = |json| Config::with_user_file("c.json", json).unwrap().theme;
    assert_eq!(theme(r#"{"theme": "neon"}"#).as_deref(), Some("neon"));
    // Without one, the built-in config's stays.
    assert_eq!(theme("{}").as_deref(), Some("dark"));
}

#[test]
fn unknown_theme_is_an_error_listing_names() {
    let c = Config::with_user_file("c.json", r#"{"theme": "neon"}"#).unwrap();
    let err = crate::theme::resolve(
        None,
        c.theme.as_deref(),
        crate::theme::ColorSupport::TrueColor,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("`neon`"), "{err}");
    for name in crate::theme::Theme::NAMES {
        assert!(err.contains(name), "{name} missing from: {err}");
    }
}

#[test]
fn legacy_styles_section_warns_and_loads() {
    let c = Config::with_user_file(
        "c.json",
        r#"{
            "keybindings": {"Global": {"<f2>": "Quit"}},
            "styles": {"Home": {"title": "bold red on blue"}}
        }"#,
    )
    .unwrap();
    assert_eq!(bound(&c, KeyContext::Global, "F2"), Some(Action::Quit));
    let warnings = c.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("\"styles\""), "{}", warnings[0]);
    assert!(!warnings[0].contains('\n'), "one line: {}", warnings[0]);
}

#[test]
fn unknown_context_is_a_load_error_listing_contexts() {
    let err = Config::with_user_file("/home/u/c.json", r#"{"keybindings": {"Sidebar": {}}}"#)
        .unwrap_err()
        .to_string();
    assert!(err.contains("/home/u/c.json"), "{err}");
    assert!(err.contains("Sidebar"), "{err}");
    assert!(err.contains("Global, Tree, Editor"), "{err}");
}

#[test]
fn malformed_key_is_a_load_error_with_location() {
    let err = Config::with_user_file(
        "/home/u/c.json",
        r#"{"keybindings": {"Tree": {"<ctrl-notakey>": "Up"}}}"#,
    )
    .unwrap_err()
    .to_string();
    for part in ["/home/u/c.json", "Tree", "<ctrl-notakey>"] {
        assert!(err.contains(part), "{part} missing from: {err}");
    }
}

/// A user file as `Config::new` reads one.
fn read(json: &str) -> RawFile {
    config::Config::builder()
        .add_source(config::File::from_str(json, config::FileFormat::Json))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap()
}

#[test]
fn a_later_file_wins_and_files_layer_per_chord() {
    let c = Config::layered(vec![
        (
            "a.json".into(),
            read(r#"{"keybindings": {"Tree": {"<x>": "Refresh", "<z>": "Open"}}}"#),
        ),
        (
            "b.json".into(),
            read(r#"{"keybindings": {"Tree": {"<x>": "Toggle"}}}"#),
        ),
    ])
    .unwrap();
    assert_eq!(bound(&c, KeyContext::Tree, "x"), Some(Action::Toggle));
    assert_eq!(bound(&c, KeyContext::Tree, "z"), Some(Action::Open));
    assert_eq!(bound(&c, KeyContext::Tree, "j"), Some(Action::Down));
}

#[test]
fn print_config_default() {
    insta::assert_snapshot!(Config::embedded().effective_json());
}

#[test]
fn printed_config_loads_back_to_the_same_keymap() {
    let embedded = Config::embedded();
    // Read on its own, not over the defaults, so nothing can be missing.
    let mut again = Config::default();
    again
        .apply("printed.json", read(&embedded.effective_json()))
        .unwrap();
    assert_eq!(again.keymap, embedded.keymap);
    assert_eq!(again.theme, embedded.theme);
    assert!(again.warnings().is_empty(), "{:?}", again.warnings());
}
