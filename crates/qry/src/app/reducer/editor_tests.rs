//! Unit tests for `editor.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::{
    app::{App, SessionId, UiEvent, state::Focus},
    db::DbCommand,
};

/// An app with the editor focused.
fn editor() -> App {
    let mut app = App::embedded();
    app.state.focus = Focus::Editor;
    app
}

fn press(editor: &mut App, code: KeyCode, modifiers: KeyModifiers) -> Vec<Effect> {
    editor.handle(UiEvent::Key(KeyEvent::new(code, modifiers)))
}

/// What has been typed.
fn query(editor: &App) -> String {
    editor.state().editor.query()
}

fn type_str(editor: &mut App, text: &str) {
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
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 1\nq".into())
        )]
    );
}

#[test]
fn the_run_key_sends_nothing_while_the_editor_is_empty() {
    let mut editor = editor();
    type_str(&mut editor, "  ");
    assert!(press(&mut editor, KeyCode::F(8), KeyModifiers::NONE).is_empty());
}

#[test]
fn unbound_shortcuts_type_nothing() {
    let mut editor = editor();
    type_str(&mut editor, "ab");
    press(&mut editor, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(query(&editor), "ab");
}

#[test]
fn f5_runs_the_statement_and_ctrl_e_the_selection() {
    let mut editor = editor();
    type_str(&mut editor, "select 1; select 2");
    assert_eq!(
        press(&mut editor, KeyCode::F(5), KeyModifiers::NONE),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 1; select 2".into())
        )]
    );
    for _ in 0.."select 2".len() {
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    }
    assert_eq!(
        press(&mut editor, KeyCode::Char('e'), KeyModifiers::CONTROL),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 2".into())
        )]
    );
}

#[test]
fn a_selection_over_lines_runs_as_written() {
    let mut editor = editor();
    type_str(&mut editor, "xx select");
    press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
    type_str(&mut editor, "1 yy");
    for _ in 0..3 {
        press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
    }
    for _ in 0.."select\n1".len() {
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    }
    assert_eq!(
        press(&mut editor, KeyCode::Char('e'), KeyModifiers::CONTROL),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select\n1".into())
        )]
    );
}

#[test]
fn undo_redo_and_delete_word() {
    let mut editor = editor();
    type_str(&mut editor, "select name");
    press(&mut editor, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert_eq!(query(&editor), "select ");
    press(&mut editor, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(query(&editor), "select name");
    press(&mut editor, KeyCode::Char('y'), KeyModifiers::CONTROL);
    assert_eq!(query(&editor), "select ");
    press(&mut editor, KeyCode::Char('z'), KeyModifiers::CONTROL);
    press(
        &mut editor,
        KeyCode::Char('Z'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(query(&editor), "select ");
}

#[test]
fn shift_selects_and_cut_paste_moves_text() {
    let mut editor = editor();
    type_str(&mut editor, "abcd");
    press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Char('x'), KeyModifiers::CONTROL);
    assert_eq!(query(&editor), "ab");

    press(&mut editor, KeyCode::Home, KeyModifiers::NONE);
    press(&mut editor, KeyCode::Char('v'), KeyModifiers::CONTROL);
    assert_eq!(query(&editor), "cdab");

    // Without a running query, ctrl-c copies.
    press(&mut editor, KeyCode::Home, KeyModifiers::NONE);
    press(&mut editor, KeyCode::Right, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Char('c'), KeyModifiers::CONTROL);
    press(&mut editor, KeyCode::End, KeyModifiers::NONE);
    press(&mut editor, KeyCode::Char('v'), KeyModifiers::CONTROL);
    assert_eq!(query(&editor), "cdabc");
}

#[test]
fn plain_movement_drops_the_selection() {
    let mut editor = editor();
    type_str(&mut editor, "abcd");
    press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    press(&mut editor, KeyCode::Left, KeyModifiers::NONE);
    // Nothing is selected any more, so typing inserts instead of replacing.
    type_str(&mut editor, "X");
    assert_eq!(query(&editor), "abXcd");
}
