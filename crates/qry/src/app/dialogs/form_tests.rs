//! Unit tests for `form.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::buffer::Buffer;

use super::*;
use crate::{
    config::Config,
    keymap::{KeyContext, Keymap},
    testing::{buffer_lines, render_with},
    theme::Theme,
    views::form::{draw, height},
};

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Sends `key` to `form` the way the reducer does: typed if it is typing,
/// else with what the `Dialog` keymap binds it to.
pub(crate) fn press(form: &mut impl Form, key: KeyEvent) {
    let action = Config::embedded()
        .keymap
        .exact(&[KeyContext::Dialog], &[key])
        .filter(|_| !(form.is_typing() && typed_char(key).is_some()));
    form.handle_key(key, action);
}

pub(crate) fn type_str(form: &mut impl Form, text: &str) {
    text.chars()
        .for_each(|c| press(form, key(KeyCode::Char(c))));
}

/// Draws `form` as a popup over a `width`x`height` terminal, with the
/// built-in keys in its footer.
pub(crate) fn render(form: &impl Form, width: u16, height: u16) -> Buffer {
    render_keyed(form, &Config::embedded().keymap, width, height)
}

fn render_keyed(form: &impl Form, keymap: &Keymap, width: u16, height: u16) -> Buffer {
    render_with(width, height, |frame| {
        draw(form, keymap, 0, &Theme::DARK, frame, frame.area());
    })
}

/// A two-field form, so the trait can be tested without either real one.
#[derive(Debug)]
struct TestForm {
    fields: [Field; 3],
    focus: usize,
    error: Option<String>,
}

impl Default for TestForm {
    fn default() -> Self {
        Self {
            fields: [
                Field::text(),
                Field::choice(&["red", "green"], 0),
                Field::checkbox(true, "in keychain"),
            ],
            focus: 0,
            error: None,
        }
    }
}

impl Form for TestForm {
    fn title(&self) -> &'static str {
        "Test"
    }
    fn labels(&self) -> &'static [&'static str] {
        &["Name", "Colour", "Save"]
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
    fn submit(&self) -> Result<Submission, String> {
        if self.text(0).is_empty() {
            return Err("Name is required".into());
        }
        Ok(Submission::ExportConnections(self.text(0).to_string()))
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
    assert_eq!(form.focus(), 2);
    press(&mut form, key(KeyCode::Tab));
    assert_eq!(form.focus(), 0);
    press(&mut form, key(KeyCode::BackTab));
    assert_eq!(form.focus(), 2);
}

#[test]
fn editing_clears_the_error() {
    let mut form = TestForm::default();
    form.set_error(Some("Name is required".into()));
    press(&mut form, key(KeyCode::Char('a')));
    assert_eq!(form.error(), None);
}

#[test]
fn the_popup_shows_labels_the_footer_and_errors() {
    let mut form = TestForm::default();
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains(" Test "), "{shown}");
    assert!(shown.contains("› Name"), "{shown}");
    assert!(shown.contains("  Colour"), "{shown}");
    assert!(shown.contains("Enter ok · Esc cancel"), "{shown}");

    form.set_error(Some("Name is required".into()));
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("Name is required"), "{shown}");
    assert!(!shown.contains("Enter ok"), "{shown}");
}

#[test]
fn footer_keys_follow_the_dialog_keymap() {
    let form = TestForm::default();
    let mut keymap = Config::embedded().keymap;
    keymap.unbind(KeyContext::Dialog, &[key(KeyCode::Enter)]);
    keymap.bind(KeyContext::Dialog, &[key(KeyCode::F(2))], Action::Submit);
    keymap.unbind(KeyContext::Dialog, &[key(KeyCode::Esc)]);
    let shown = buffer_lines(&render_keyed(&form, &keymap, 80, 24)).join("\n");
    assert!(shown.contains("F2 ok"), "{shown}");
    assert!(
        !shown.contains("cancel"),
        "an unbound action is left out: {shown}"
    );
}

#[test]
fn the_popup_is_sized_from_the_field_count() {
    let form = TestForm::default();
    // 3 fields, a blank row, the footer and the border.
    assert_eq!(height(&form), 7);

    let buffer = render(&form, 80, 24);
    let top = (0..24).find(|&y| buffer[(8, y)].symbol() == "┌").unwrap();
    assert_eq!(buffer[(8 + 64 - 1, top + 7 - 1)].symbol(), "┘");
}

#[test]
fn a_short_terminal_scrolls_to_the_focused_field() {
    let mut form = TestForm::default();
    form.set_focus(2);
    let shown = buffer_lines(&render(&form, 80, 6)).join("\n");
    assert!(shown.contains("› Save"), "{shown}");
    assert!(!shown.contains("Name"), "{shown}");
}

#[test]
fn only_the_focused_field_has_marker_and_cursor() {
    let mut form = TestForm::default();
    type_str(&mut form, "ab");
    let buffer = render(&form, 80, 24);
    let lines = buffer_lines(&buffer);
    let cursor = Theme::DARK.cursor_cell();
    let cursors: Vec<_> = (0..24u16)
        .flat_map(|y| (0..80u16).map(move |x| (x, y)))
        .filter(|&(x, y)| buffer[(x, y)].bg == cursor.bg.unwrap())
        .collect();
    assert_eq!(cursors.len(), 1, "one block cursor: {cursors:?}");
    let (x, y) = cursors[0];
    assert!(lines[usize::from(y)].contains("› Name"));
    assert_eq!(
        buffer[(x - 1, y)].symbol(),
        "b",
        "just after what was typed"
    );
    assert_eq!(
        lines.iter().filter(|l| l.contains('›')).count(),
        2,
        "› marker and the choice's ›"
    );
}

#[test]
fn long_text_scrolls_to_keep_cursor_visible_with_leading_ellipsis() {
    let mut form = TestForm::default();
    let long: String = (0..100)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    type_str(&mut form, &long);
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    let row = shown.lines().find(|l| l.contains("› Name")).unwrap();
    assert!(row.contains("Name    …"), "{row}");
    assert!(
        row.contains(&long[long.len() - 10..]),
        "the end is in view: {row}"
    );
}

#[test]
fn choice_and_checkbox_render_and_cycle() {
    let mut form = TestForm::default();
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("‹ red ›"), "{shown}");
    assert!(shown.contains("[x] in keychain"), "{shown}");
    form.set_focus(2);
    press(&mut form, key(KeyCode::Char(' ')));
    assert!(!form.checked(2));
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("[ ] in keychain"), "{shown}");
}
