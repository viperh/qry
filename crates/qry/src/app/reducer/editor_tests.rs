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
    editor.state().editor().query()
}

/// The worker's answer to the statement it was sent.
fn done(editor: &mut App) -> Vec<Effect> {
    editor.handle(UiEvent::Db(
        SessionId::FIRST,
        crate::app::DbAnswer::QueryDone(std::sync::Arc::new(qry_core::testing::grid(1, 1))),
    ))
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
    press(&mut editor, KeyCode::F(12), KeyModifiers::NONE);
    assert_eq!(query(&editor), "ab");
}

#[test]
fn f5_runs_the_statement_and_ctrl_e_the_selection() {
    let mut editor = editor();
    type_str(&mut editor, "select 1; select 2");
    // The statement under the cursor only.
    assert_eq!(
        press(&mut editor, KeyCode::F(5), KeyModifiers::NONE),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 2".into())
        )]
    );
    done(&mut editor);
    for _ in 0.."1; select 2".len() {
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    }
    assert_eq!(
        press(&mut editor, KeyCode::Char('e'), KeyModifiers::CONTROL),
        [Effect::Db(SessionId::FIRST, DbCommand::Query("1".into()))],
        "the selection, split where it has a `;`"
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

#[test]
fn ctrl_enter_runs_statement_under_cursor_only() {
    let mut editor = editor();
    type_str(&mut editor, "select 1;");
    press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
    type_str(&mut editor, "select 2;");
    press(&mut editor, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        press(&mut editor, KeyCode::Enter, KeyModifiers::CONTROL),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 1".into())
        )]
    );
}

#[test]
fn f8_runs_all_statements_as_separate_jobs() {
    let mut editor = editor();
    type_str(&mut editor, "select 1; select 2;");
    // One at a time: the next goes once the first is answered.
    assert_eq!(
        press(&mut editor, KeyCode::F(8), KeyModifiers::NONE),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 1".into())
        )]
    );
    assert_eq!(
        done(&mut editor),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 2".into())
        )]
    );
    assert!(done(&mut editor).is_empty());
    let session = editor.state().sessions.active();
    assert_eq!(session.tabs.len(), 2, "one result tab each");
}

#[test]
fn run_selection_runs_selected_text() {
    let mut editor = editor();
    type_str(&mut editor, "select 123");
    for _ in 0..3 {
        press(&mut editor, KeyCode::Left, KeyModifiers::SHIFT);
    }
    assert_eq!(
        press(&mut editor, KeyCode::Char('e'), KeyModifiers::CONTROL),
        [Effect::Db(SessionId::FIRST, DbCommand::Query("123".into()))]
    );
}

fn fail(editor: &mut App, position: Option<usize>) -> Vec<Effect> {
    editor.handle(UiEvent::Db(
        SessionId::FIRST,
        crate::app::DbAnswer::QueryFailed {
            message: "syntax error".into(),
            position,
        },
    ))
}

#[test]
fn run_all_stops_at_first_failure_by_default() {
    let mut editor = editor();
    type_str(&mut editor, "select 1; selec 2; select 3;");
    press(&mut editor, KeyCode::F(8), KeyModifiers::NONE);
    assert_eq!(done(&mut editor).len(), 1, "the second goes");
    assert!(fail(&mut editor, None).is_empty(), "the third does not");
    let session = editor.state().sessions.active();
    assert!(session.queue.is_empty());
    assert_eq!(session.failed.as_ref().unwrap().span, 10..17);
    assert!(session.tabs.last().unwrap().error.is_some());
}

#[test]
fn continue_on_error_runs_the_rest() {
    let mut config = crate::config::Config::embedded();
    config.continue_on_error = true;
    let mut editor = App::new(config, crate::theme::Theme::DARK, 0);
    editor.state.focus = Focus::Editor;
    type_str(&mut editor, "selec 1; select 2;");
    press(&mut editor, KeyCode::F(8), KeyModifiers::NONE);
    assert_eq!(
        fail(&mut editor, None),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 2".into())
        )]
    );
}

#[test]
fn error_position_maps_into_statement() {
    let mut editor = editor();
    type_str(&mut editor, "select 1;\nselect * frm t;");
    press(&mut editor, KeyCode::F(8), KeyModifiers::NONE);
    done(&mut editor);
    // The server says byte 9 of `select * frm t`: the `frm`.
    fail(&mut editor, Some(9));
    let failed = editor.state().sessions.active().failed.clone().unwrap();
    let text = query(&editor);
    assert_eq!(&text[failed.span.clone()], "select * frm t");
    assert_eq!(
        &text[failed.position.unwrap()..failed.position.unwrap() + 3],
        "frm"
    );
}

#[test]
fn buffers_are_per_session() {
    let mut h = crate::testing::AppHarness::new(crate::config::Config::embedded());
    h.send(UiEvent::Started);
    h.keys("tab").type_str("select 1").keys("shift-enter");
    // shift-Enter in the editor is not a session key: it is typed nothing.
    h.keys("f6 f6 f6");
    h.state_mut().focus = Focus::Tree;
    h.keys("shift-enter tab").type_str("select 2");
    h.keys("ctrl-1");
    assert_eq!(h.state().editor().query(), "select 1");
    h.keys("ctrl-2");
    assert_eq!(h.state().editor().query(), "select 2");
}

#[test]
fn undo_history_is_per_session() {
    let mut h = crate::testing::AppHarness::new(crate::config::Config::embedded());
    h.send(UiEvent::Started);
    h.keys("shift-enter tab").type_str("abc");
    h.keys("ctrl-1").type_str("x").keys("ctrl-z");
    assert_eq!(
        h.state().editor().query(),
        "",
        "undoes this session's typing"
    );
    h.keys("ctrl-2 ctrl-z");
    assert_ne!(h.state().editor().query(), "abc");
}

#[test]
fn enter_auto_indents() {
    let mut editor = editor();
    type_str(&mut editor, "  select");
    press(&mut editor, KeyCode::Enter, KeyModifiers::NONE);
    type_str(&mut editor, "1");
    assert_eq!(query(&editor), "  select\n  1");
}

#[test]
fn tab_indents_shift_tab_dedents() {
    let mut editor = editor();
    type_str(&mut editor, "x");
    press(&mut editor, KeyCode::Home, KeyModifiers::NONE);
    press(&mut editor, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(query(&editor), "    x");
    press(&mut editor, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(query(&editor), "x");
    assert_eq!(editor.state().editor().textarea.cursor(), (0, 0));
    // F6 leaves the editor instead.
    press(&mut editor, KeyCode::F(6), KeyModifiers::NONE);
    assert_eq!(editor.state().focus, Focus::Results);
}

#[test]
fn crlf_paste_normalised() {
    let mut editor = editor();
    editor.handle(UiEvent::Paste("select 1\r\nfrom t\r".into()));
    assert_eq!(query(&editor), "select 1\nfrom t\n");
}
