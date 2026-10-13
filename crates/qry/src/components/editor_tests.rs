//! Unit tests for `editor.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};

use super::*;

fn editor() -> Editor {
    let mut editor = Editor::default();
    editor.register_config_handler(Config::embedded()).unwrap();
    editor
}

fn press(editor: &mut Editor, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    editor
        .handle_key_event(KeyEvent::new(code, modifiers))
        .unwrap()
}

fn type_str(editor: &mut Editor, text: &str) {
    for c in text.chars() {
        press(editor, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

#[test]
fn typing_and_the_run_key() {
    let mut editor = editor();
    type_str(&mut editor, "select 1");
    press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
    type_str(&mut editor, "q");
    assert_eq!(
        press(&mut editor, KeyCode::F(8), KeyModifiers::NONE),
        Some(Action::Execute("select 1\nq".into()))
    );
}

#[test]
fn unbound_shortcuts_type_nothing() {
    let mut editor = editor();
    type_str(&mut editor, "ab");
    press(&mut editor, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(editor.query(), "ab");
}

#[test]
fn undo_redo_and_delete_word() {
    let mut editor = editor();
    type_str(&mut editor, "select name");
    press(&mut editor, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(editor.query(), "select ");
    press(&mut editor, KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(editor.query(), "select name");
    press(&mut editor, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(editor.query(), "select ");
}

#[test]
fn shift_selects_and_cut_paste_moves_text() {
    let mut editor = editor();
    type_str(&mut editor, "abcd");
    press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert_eq!(editor.query(), "ab");

    press(&mut editor, KeyCode::Home, KeyModifiers::NONE);
    press(&mut editor, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert_eq!(editor.query(), "cdab");
}

#[test]
fn plain_movement_drops_the_selection() {
    let mut editor = editor();
    type_str(&mut editor, "abcd");
    press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
    // Nothing is selected any more, so typing inserts instead of replacing.
    type_str(&mut editor, "X");
    assert_eq!(editor.query(), "abXcd");
}
