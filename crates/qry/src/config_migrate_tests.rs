//! Unit tests for `config_migrate.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pretty_assertions::assert_eq;

use crate::{
    action::Action,
    config::Config,
    keymap::{KeyContext, Lookup, parse_key_sequence},
    testing::fixture,
};

/// What `chord` runs in `context` alone.
fn bound(config: &Config, context: KeyContext, chord: &str) -> Option<Action> {
    match config
        .keymap
        .resolve(&[context], &parse_key_sequence(chord).unwrap())
    {
        Lookup::Match(action) => Some(action),
        Lookup::Prefix | Lookup::None => config
            .keymap
            .exact(&[context], &parse_key_sequence(chord).unwrap()),
    }
}

/// The warnings about the old format.
fn migration_warnings(config: &Config) -> Vec<String> {
    config
        .warnings()
        .into_iter()
        .filter(|w| w.contains("old keybinding format"))
        .collect()
}

#[test]
fn old_format_is_converted() {
    let path = fixture("config/legacy-0.1.json");
    let json = std::fs::read_to_string(&path).unwrap();
    let name = path.display().to_string();
    let config = Config::with_user_file(&name, &json).unwrap();

    assert_eq!(
        bound(&config, KeyContext::Global, "ctrl-n"),
        Some(Action::NewConnection)
    );
    assert_eq!(
        bound(&config, KeyContext::Editor, "F8"),
        Some(Action::RunAll)
    );
    // A modal's own section and the forms' shared one both become Dialog.
    assert_eq!(
        bound(&config, KeyContext::Dialog, "Esc"),
        Some(Action::Cancel)
    );
    assert_eq!(
        bound(&config, KeyContext::Dialog, "shift-Tab"),
        Some(Action::PrevField)
    );
    assert_eq!(
        bound(&config, KeyContext::Tree, "Enter"),
        Some(Action::Open)
    );
    assert_eq!(
        bound(&config, KeyContext::Global, "ctrl-e"),
        Some(Action::ExportResults)
    );
    assert_eq!(
        bound(&config, KeyContext::Help, "Esc"),
        Some(Action::Cancel)
    );
    // The file's keys win over the new defaults: it bound `k` to Down.
    assert_eq!(bound(&config, KeyContext::Results, "k"), Some(Action::Down));

    let warnings = migration_warnings(&config);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains(&name), "{}", warnings[0]);
    assert!(
        warnings[0].contains("run qry --print-config to see the new one"),
        "{}",
        warnings[0]
    );
    assert!(!warnings[0].contains("skipped"), "{}", warnings[0]);
}

#[test]
fn unmappable_old_keys_are_listed() {
    let config = Config::with_user_file(
        "old.json",
        r#"{
            "keybindings": {
                "Home": {
                    "<ctrl-q>": "Quit",
                    "<ctrl-x>": { "Execute": "select 1" },
                    "<f2>": { "ChangeMode": "PasswordModal" }
                }
            },
            "panes": {
                "Editor": { "<f9>": "Teleport", "<f8>": "RunQuery" },
                "Sidebar": { "<x>": "Up" }
            }
        }"#,
    )
    .unwrap();

    assert_eq!(
        bound(&config, KeyContext::Global, "ctrl-q"),
        Some(Action::Quit)
    );
    assert_eq!(
        bound(&config, KeyContext::Editor, "F8"),
        Some(Action::RunAll)
    );
    assert_eq!(bound(&config, KeyContext::Editor, "F9"), None);
    assert_eq!(bound(&config, KeyContext::Global, "ctrl-x"), None);

    let warnings = migration_warnings(&config);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for skipped in [
        r#"Home <ctrl-x>: {"Execute":"select 1"}"#,
        r#"Home <f2>: {"ChangeMode":"PasswordModal"}"#,
        r#"panes.Editor <f9>: "Teleport""#,
        "panes.Sidebar",
    ] {
        assert!(
            warnings[0].contains(skipped),
            "{skipped} in {}",
            warnings[0]
        );
    }
    assert!(!warnings[0].contains("RunQuery"), "{}", warnings[0]);
}

#[test]
fn a_new_format_file_gives_no_migration_warning() {
    let config = Config::with_user_file(
        "new.json",
        r#"{"keybindings": {"Global": {"<f2>": "Help"}}}"#,
    )
    .unwrap();
    assert!(config.warnings().is_empty(), "{:?}", config.warnings());
    assert_eq!(bound(&config, KeyContext::Global, "F2"), Some(Action::Help));
}
