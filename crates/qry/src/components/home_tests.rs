//! Unit tests for `home.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

use super::*;
use crate::{action::StatusCode, components::form_tests::key};

fn render(home: &mut Home, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| home.draw(frame, frame.area()).unwrap())
        .unwrap();
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn shown(home: &mut Home, width: u16, height: u16) -> String {
    let buffer = render(home, width, height);
    (0..buffer.area.height)
        .map(|y| row_text(&buffer, y) + "\n")
        .collect()
}

/// A Home set up with the built-in config, as `App` would.
fn home() -> Home {
    let mut home = Home::new();
    home.register_config_handler(Config::embedded()).unwrap();
    home
}

fn open(home: &mut Home, mode: Mode) {
    home.update(Action::ChangeMode(mode)).unwrap();
}

#[test]
fn each_mode_opens_its_own_modal() {
    let mut home = home();
    assert!(home.modal.is_none());

    open(&mut home, Mode::AddConnModal);
    assert!(shown(&mut home, 80, 50).contains("New Connection"));

    open(&mut home, Mode::ExpoModal);
    let text = shown(&mut home, 80, 50);
    assert!(
        text.contains("Export") && !text.contains("New Connection"),
        "{text}"
    );

    open(&mut home, Mode::Home);
    assert!(home.modal.is_none());
    assert!(!shown(&mut home, 80, 50).contains("Export"));
}

#[test]
fn keys_reach_the_open_modal_and_it_reopens_empty() {
    let mut home = home();
    open(&mut home, Mode::ExpoModal);
    type_str_home(&mut home, "rows.csv");
    assert!(shown(&mut home, 80, 50).contains("rows.csv"));

    // Reopening builds a fresh form.
    open(&mut home, Mode::Home);
    open(&mut home, Mode::ExpoModal);
    assert!(!shown(&mut home, 80, 50).contains("rows.csv"));
}

fn type_str_home(home: &mut Home, text: &str) {
    for c in text.chars() {
        home.handle_key_event(key(KeyCode::Char(c))).unwrap();
    }
}

#[test]
fn a_failed_export_keeps_the_modal_and_the_path() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();
    open(&mut home, Mode::ExpoModal);
    type_str_home(&mut home, "no/such/dir/rows.csv");

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    let Ok(Action::Export(config)) = rx.try_recv() else {
        panic!("expected an export action");
    };
    assert_eq!(config.path, "no/such/dir/rows.csv");
    assert!(rx.try_recv().is_err(), "the mode must not change yet");
    assert!(shown(&mut home, 80, 50).contains("Exporting"));

    home.update(Action::ExportFailed(
        "could not write no/such/dir/rows.csv".into(),
    ))
    .unwrap();
    let text = shown(&mut home, 80, 50);
    assert!(text.contains("could not write"), "{text}");
    assert!(text.contains("no/such/dir/rows.csv"), "{text}");
}

#[test]
fn an_import_waits_for_the_file_and_keeps_the_path_on_failure() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();

    // Ctrl-o opens it from the main screen.
    let ctrl_o = crossterm::event::KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(
        home.config.keybindings.0[&Mode::Home].get(&vec![ctrl_o]),
        Some(&Action::ChangeMode(Mode::ImportModal))
    );
    open(&mut home, Mode::ImportModal);
    type_str_home(&mut home, "~/nope.json");

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    let Ok(Action::Import { path, .. }) = rx.try_recv() else {
        panic!("expected an import action");
    };
    assert_eq!(path, "~/nope.json");
    assert!(shown(&mut home, 80, 50).contains("Importing"));

    home.update(Action::ImportFailed("cannot read nope.json".into()))
        .unwrap();
    let text = shown(&mut home, 80, 50);
    assert!(
        text.contains("cannot read nope.json") && text.contains("~/nope.json"),
        "{text}"
    );

    // A second try that works closes it.
    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::Import { .. })));
    home.update(Action::Imported(Box::new(crate::import::Imported {
        format: crate::import::Format::Qry,
        connections: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    })))
    .unwrap();
    assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
}

#[test]
fn exporting_connections_waits_for_the_file() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();

    let ctrl_s = crossterm::event::KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(
        home.config.keybindings.0[&Mode::Home].get(&vec![ctrl_s]),
        Some(&Action::ChangeMode(Mode::ConnExportModal))
    );
    open(&mut home, Mode::ConnExportModal);
    assert!(shown(&mut home, 80, 50).contains("Export Connections"));
    type_str_home(&mut home, "/root/b.json");

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        rx.try_recv().unwrap(),
        Action::ExportConnections("/root/b.json".into())
    );
    assert!(shown(&mut home, 80, 50).contains("Exporting"));

    home.update(Action::ConnectionsExportFailed(
        "cannot write /root/b.json".into(),
    ))
    .unwrap();
    assert!(shown(&mut home, 80, 50).contains("cannot write /root/b.json"));

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::ExportConnections(_))));
    home.update(Action::ConnectionsExported("/tmp/b.json".into()))
        .unwrap();
    assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
}

#[test]
fn a_written_export_closes_the_modal() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();
    open(&mut home, Mode::ExpoModal);
    type_str_home(&mut home, "rows.csv");

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::Export(_))));

    home.update(Action::Exported("rows.csv".into())).unwrap();
    assert!(home.modal.is_none());
    assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
}

#[test]
fn an_answer_meant_for_another_modal_is_ignored() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();

    // An export is waiting, and a connection's answer arrives late.
    open(&mut home, Mode::ExpoModal);
    type_str_home(&mut home, "rows.csv");
    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    home.update(Action::Connected(":memory:".into())).unwrap();
    home.update(Action::ConnectFailed("nope".into())).unwrap();

    let text = shown(&mut home, 80, 50);
    assert!(
        home.modal.is_some(),
        "the export modal was closed by a connection"
    );
    assert!(
        text.contains("Exporting") && !text.contains("nope"),
        "{text}"
    );
}

#[test]
fn a_refused_modal_stays_open_with_its_error() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();
    open(&mut home, Mode::ExpoModal);

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(rx.try_recv().is_err());
    assert!(shown(&mut home, 80, 50).contains("Path is required"));
}

/// Fills in enough of the New Connection form to be submittable.
fn fill_connection(home: &mut Home) {
    for (field, value) in [(2, "db.local"), (4, "alice"), (6, "app")] {
        home.modal.as_mut().unwrap().set_focus(field);
        type_str_home(home, value);
    }
}

#[test]
fn a_failed_connection_keeps_the_modal_and_everything_typed() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();
    open(&mut home, Mode::AddConnModal);
    fill_connection(&mut home);

    // Submitting sends the connection but leaves the modal waiting.
    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::Connect(..))));
    assert!(rx.try_recv().is_err(), "the mode must not change yet");
    assert!(shown(&mut home, 80, 50).contains("Connecting"));

    // A second Enter while waiting does not try again.
    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(rx.try_recv().is_err());

    home.update(Action::ConnectFailed(
        "password authentication failed".into(),
    ))
    .unwrap();
    let text = shown(&mut home, 80, 50);
    assert!(text.contains("password authentication failed"), "{text}");
    assert!(
        text.contains("db.local") && text.contains("alice"),
        "{text}"
    );
    assert!(!text.contains("Connecting"), "{text}");

    // Correcting and submitting again sends a second attempt.
    home.handle_key_event(key(KeyCode::Char('x'))).unwrap();
    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::Connect(..))));
}

#[test]
fn a_working_connection_closes_the_modal() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();
    open(&mut home, Mode::AddConnModal);
    fill_connection(&mut home);

    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert!(matches!(rx.try_recv(), Ok(Action::Connect(..))));

    home.update(Action::Connected("alice@db.local:5432/app".into()))
        .unwrap();
    assert!(home.modal.is_none());
    assert_eq!(rx.try_recv().unwrap(), Action::ChangeMode(Mode::Home));
    assert!(!shown(&mut home, 80, 50).contains("New Connection"));
}

#[test]
fn a_connect_answer_without_a_waiting_modal_is_ignored() {
    let mut home = home();
    // The in-memory connection at startup answers with nobody waiting.
    home.update(Action::Connected(":memory:".into())).unwrap();
    home.update(Action::ConnectFailed("nope".into())).unwrap();
    assert!(home.modal.is_none());

    // An export modal is not touched by a connection's answer either.
    open(&mut home, Mode::ExpoModal);
    home.update(Action::ConnectFailed("nope".into())).unwrap();
    let text = shown(&mut home, 80, 50);
    assert!(text.contains("Export") && !text.contains("nope"), "{text}");
}

#[test]
fn a_request_for_a_password_opens_the_prompt() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut home = home();
    home.register_action_handler(tx).unwrap();

    home.update(Action::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    })
    .unwrap();
    // It asks `App` to switch modes, so Esc closes the prompt.
    assert_eq!(
        rx.try_recv().unwrap(),
        Action::ChangeMode(Mode::PasswordModal)
    );

    open(&mut home, Mode::PasswordModal);
    let text = shown(&mut home, 80, 30);
    assert!(
        text.contains("Password") && text.contains("for prod"),
        "{text}"
    );

    // Submitting waits for the worker, as a connection does.
    crate::secrets::stash(crate::secrets::Password::new("x"));
    home.handle_key_event(key(KeyCode::Enter)).unwrap();
    assert_eq!(rx.try_recv().unwrap(), Action::PasswordEntered);
    assert!(shown(&mut home, 80, 30).contains("Connecting"));

    home.update(Action::Connected(":memory:".into())).unwrap();
    assert!(home.modal.is_none());
}

#[test]
fn help_opens_scrolls_and_closes_with_configured_keys() {
    let mut home = home();
    home.update(Action::Help).unwrap();
    assert!(home.helpvisible);
    assert!(shown(&mut home, 80, 20).contains("Global:"));

    // While help is open, keys go to it, not to the focused pane.
    home.focus = Pane::Results;
    home.handle_key_event(key(KeyCode::PageDown)).unwrap();
    assert!(!shown(&mut home, 80, 20).contains("Global:"));

    home.handle_key_event(key(KeyCode::Esc)).unwrap();
    assert!(!home.helpvisible);

    home.update(Action::Help).unwrap();
    assert!(shown(&mut home, 80, 20).contains("Global:"));
    home.update(Action::Help).unwrap();
    assert!(!home.helpvisible);
}

#[test]
fn update_is_forwarded_to_children() {
    let mut home = home();
    assert!(home.statuspanel.is_empty());
    home.update(Action::Status(StatusCode::Error("e".into())))
        .unwrap();
    assert!(!home.statuspanel.is_empty());
}

#[test]
fn tree_is_focused_and_highlighted_at_startup() {
    let mut home = home();
    assert!(home.focus == Pane::Tree);

    let buf = render(&mut home, 80, 24);
    assert_eq!(buf[(0, 0)].fg, Color::Yellow);
    assert_ne!(buf[(20, 0)].fg, Color::Yellow);

    home.update(Action::FocusNext).unwrap();
    assert!(home.focus == Pane::Editor);
    let buf = render(&mut home, 80, 24);
    assert_ne!(buf[(0, 0)].fg, Color::Yellow);
    assert_eq!(buf[(20, 0)].fg, Color::Cyan);

    home.update(Action::FocusNext).unwrap();
    assert!(home.focus == Pane::Results);
    home.update(Action::FocusNext).unwrap();
    assert!(home.focus == Pane::Tree);
    home.update(Action::FocusPrev).unwrap();
    assert!(home.focus == Pane::Results);
}

#[test]
fn keys_reach_the_focused_pane_when_nothing_is_on_top() {
    let mut home = home();
    home.focus = Pane::Editor;
    home.sync_focus();
    type_str_home(&mut home, "select 1");
    assert_eq!(
        home.handle_key_event(KeyEvent::new(KeyCode::F(8), KeyModifiers::NONE))
            .unwrap(),
        Some(Action::Execute("select 1".into()))
    );
}
