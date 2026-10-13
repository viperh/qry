//! Unit tests for `form.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

use super::*;
use crate::config::Config;

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Sends `key` to `form` the way `Home` does: with its configured command.
pub(crate) fn press(form: &mut impl Form, key: KeyEvent) {
    form.handle_key(key, Config::embedded().panes.form.get(key));
}

pub(crate) fn type_str(form: &mut impl Form, text: &str) {
    text.chars()
        .for_each(|c| press(form, key(KeyCode::Char(c))));
}

pub(crate) fn render(form: &impl Form, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| form.render(frame, frame.area()))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub(crate) fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// A two-field form, so the trait can be tested without either real one.
struct TestForm {
    fields: [Field; 2],
    focus: usize,
    error: Option<String>,
}

impl Default for TestForm {
    fn default() -> Self {
        Self {
            fields: [Field::text(), Field::choice(&["red", "green"], 0)],
            focus: 0,
            error: None,
        }
    }
}

impl Form for TestForm {
    fn title(&self) -> &'static str {
        "Test"
    }
    fn hint(&self) -> &'static str {
        "Enter ok"
    }
    fn labels(&self) -> &'static [&'static str] {
        &["Name", "Colour"]
    }
    fn fields(&self) -> &[Field] {
        &self.fields
    }
    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }
    fn focus(&self) -> usize {
        self.focus
    }
    fn set_focus(&mut self, focus: usize) {
        self.focus = focus
    }
    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    fn set_error(&mut self, error: Option<String>) {
        self.error = error
    }
    fn submit(&self) -> Result<Action, String> {
        if self.text(0).is_empty() {
            return Err("Name is required".into());
        }
        Ok(Action::Info(self.text(0).to_string()))
    }
}

#[test]
fn typing_and_editing_go_to_the_focused_text_field() {
    let mut form = TestForm::default();
    type_str(&mut form, "hélo");
    press(&mut form, key(KeyCode::Left));
    press(&mut form, key(KeyCode::Char('l')));
    assert_eq!(form.text(0), "héllo");

    press(&mut form, key(KeyCode::Home));
    press(&mut form, key(KeyCode::Delete));
    press(&mut form, key(KeyCode::End));
    press(&mut form, key(KeyCode::Backspace));
    assert_eq!(form.text(0), "éll");

    // Space is bound to NextChoice, but types a space in a text field.
    press(&mut form, key(KeyCode::Char(' ')));
    assert_eq!(form.text(0), "éll ");
    // Shortcuts are not typed.
    press(
        &mut form,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    );
    assert_eq!(form.text(0), "éll ");
}

#[test]
fn tab_cycles_fields_and_choices_ignore_typing() {
    let mut form = TestForm::default();
    press(&mut form, key(KeyCode::Tab));
    assert_eq!(form.focus(), 1);

    press(&mut form, key(KeyCode::Char('x')));
    assert_eq!(form.selected(1), 0);
    press(&mut form, key(KeyCode::Right));
    assert_eq!(form.selected(1), 1);
    press(&mut form, key(KeyCode::Char(' ')));
    assert_eq!(form.selected(1), 0);
    press(&mut form, key(KeyCode::Left));
    assert_eq!(form.selected(1), 1);

    press(&mut form, key(KeyCode::Tab));
    assert_eq!(form.focus(), 0);
    press(&mut form, key(KeyCode::BackTab));
    assert_eq!(form.focus(), 1);
}

#[test]
fn editing_clears_the_error() {
    let mut form = TestForm::default();
    form.set_error(Some("Name is required".into()));
    press(&mut form, key(KeyCode::Char('a')));
    assert_eq!(form.error(), None);
}

#[test]
fn the_popup_shows_labels_the_hint_and_errors() {
    let mut form = TestForm::default();
    let shown = lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("Test"), "{shown}");
    assert!(shown.contains("› Name"), "{shown}");
    assert!(shown.contains("Colour"), "{shown}");
    assert!(shown.contains("Enter ok"), "{shown}");

    form.set_error(Some("Name is required".into()));
    let shown = lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("Name is required"), "{shown}");
    assert!(!shown.contains("Enter ok"), "{shown}");
}

#[test]
fn the_popup_is_sized_from_the_field_count() {
    let form = TestForm::default();
    // 2 fields: 3 rows each, 1 row between, 1 row padding and a border.
    assert_eq!(form.height(), 11);

    let buffer = render(&form, 80, 24);
    let top = (0..24).find(|&y| buffer[(10, y)].symbol() == "┌").unwrap();
    assert_eq!(buffer[(10 + MODAL_WIDTH - 1, top + 11 - 1)].symbol(), "┘");
}

#[test]
fn a_short_terminal_scrolls_to_the_focused_field() {
    let mut form = TestForm::default();
    form.set_focus(1);
    let shown = lines(&render(&form, 80, 8)).join("\n");
    assert!(shown.contains("› Colour"), "{shown}");
    assert!(!shown.contains("Name"), "{shown}");
}
