//! Unit tests for `textinput.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn typed(c: char, modifiers: KeyModifiers) -> Option<char> {
    typed_char(KeyEvent::new(KeyCode::Char(c), modifiers))
}

#[test]
fn altgr_characters_are_typed_but_shortcuts_are_not() {
    assert_eq!(typed('\\', KeyModifiers::NONE), Some('\\'));
    assert_eq!(typed('A', KeyModifiers::SHIFT), Some('A'));
    // AltGr, as Windows reports it.
    assert_eq!(
        typed('\\', KeyModifiers::CONTROL | KeyModifiers::ALT),
        Some('\\')
    );
    assert_eq!(typed('c', KeyModifiers::CONTROL), None);
    assert_eq!(typed('d', KeyModifiers::ALT), None);
}

#[test]
fn a_windows_path_types_in_full() {
    let mut input = TextInput::default();
    for c in r"C:\data\rows.csv".chars() {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL | KeyModifiers::ALT);
        if let Some(c) = typed_char(key) {
            input.insert(c);
        }
    }
    assert_eq!(input.value, r"C:\data\rows.csv");
}
