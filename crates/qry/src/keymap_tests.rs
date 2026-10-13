//! Unit tests for `keymap.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn keymap(json: &str) -> Result<Keymap<ResultsCommand>, json5::Error> {
    json5::from_str(json)
}

#[test]
fn looks_up_keys_by_code_and_modifiers() {
    let map = keymap(r#"{ "<up>": "Up", "<i>": "Up", "<shift-g>": "LastRow" }"#).unwrap();
    assert_eq!(
        map.get(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(ResultsCommand::Up)
    );
    assert_eq!(
        map.get(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
        Some(ResultsCommand::Up)
    );
    assert_eq!(
        map.get(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)),
        Some(ResultsCommand::LastRow)
    );
    assert_eq!(
        map.get(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)),
        None
    );
}

#[test]
fn rejects_sequences_and_unknown_commands() {
    assert!(keymap(r#"{ "<g><g>": "FirstRow" }"#).is_err());
    assert!(keymap(r#"{ "<g>": "Teleport" }"#).is_err());
    assert!(keymap(r#"{ "<ctrl-notakey>": "Up" }"#).is_err());
}

#[test]
fn describe_groups_keys_in_command_order() {
    let map =
        keymap(r#"{ "<i>": "Up", "<up>": "Up", "<shift-g>": "LastRow", "<home>": "FirstColumn" }"#)
            .unwrap();
    assert_eq!(
        map.describe(),
        [
            ("↑ / i".to_string(), "Up a row"),
            ("Home".to_string(), "First column"),
            ("G".to_string(), "Last row"),
        ]
    );
}

#[test]
fn user_keys_win_over_defaults() {
    let mut user = keymap(r#"{ "<g>": "LastRow" }"#).unwrap();
    user.merge_defaults(&keymap(r#"{ "<g>": "FirstRow", "<up>": "Up" }"#).unwrap());
    assert_eq!(
        user.get(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)),
        Some(ResultsCommand::LastRow)
    );
    assert_eq!(
        user.get(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(ResultsCommand::Up)
    );
}

#[test]
fn keys_display_readably() {
    let show = |code, mods| display_key(&KeyEvent::new(code, mods));
    assert_eq!(show(KeyCode::Char('u'), KeyModifiers::CONTROL), "Ctrl-u");
    assert_eq!(show(KeyCode::BackTab, KeyModifiers::SHIFT), "Shift-Tab");
    assert_eq!(
        show(KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        "Ctrl-Shift-←"
    );
    assert_eq!(show(KeyCode::F(8), KeyModifiers::NONE), "F8");
    assert_eq!(show(KeyCode::Char(' '), KeyModifiers::NONE), "Space");
    assert_eq!(show(KeyCode::PageDown, KeyModifiers::NONE), "PageDown");
}
