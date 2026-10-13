use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::{secrets::Password, theme::Theme};

#[derive(Default)]
pub(crate) struct TextInput {
    pub value: String,
    pub cursor: usize,
    /// Shown dimmed while the field is empty, for what an empty field means.
    pub placeholder: Option<&'static str>,
}

/// A password field. What is typed goes key by key into a [`Password`], so
/// it is cleared when dropped and never printed; the screen shows a dot per
/// character.
#[derive(Default)]
pub(crate) struct SecretInput {
    pub value: Password,
    pub cursor: usize,
    /// Shown dimmed while the field is empty, for what an empty field means.
    pub placeholder: Option<&'static str>,
}

/// The character a key types, if it is plain typing rather than a shortcut.
///
/// Windows reports AltGr as Ctrl+Alt, and on many layouts AltGr is how you
/// type `\`, `@` or `{`. So only a lone Ctrl or Alt makes a shortcut; both
/// together is someone typing.
pub(super) fn typed_char(key: KeyEvent) -> Option<char> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char(c) if ctrl == alt => Some(c),
        _ => None,
    }
}

// Editing operations. Which key runs which is decided by the config (see
// `crate::keymap::FormCommand`), not here.
impl TextInput {
    pub(crate) fn insert(&mut self, c: char) {
        let at = self.byte_index();
        self.value.insert(at, c);
        self.cursor += 1;
    }

    pub(crate) fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_index();
            self.value.remove(at);
        }
    }

    pub(crate) fn delete_forward(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index();
            self.value.remove(at);
        }
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.len();
    }

    pub(crate) fn len(&self) -> usize {
        self.value.chars().count()
    }

    pub(crate) fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    /// Draws the input, scrolled so the cursor stays visible. The terminal
    /// cursor is only placed on the focused input.
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let width = usize::from(area.width).max(1);
        let offset = (self.cursor + 1).saturating_sub(width);
        let visible: String = self.value.chars().skip(offset).take(width).collect();
        render_line(
            frame,
            area,
            (visible, self.placeholder),
            focused.then_some(self.cursor - offset),
            theme,
        );
    }
}

impl SecretInput {
    pub(crate) fn insert(&mut self, c: char) {
        self.value.insert(self.cursor, c);
        self.cursor += 1;
    }

    pub(crate) fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.value.remove(self.cursor);
        }
    }

    pub(crate) fn delete_forward(&mut self) {
        self.value.remove(self.cursor);
    }

    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub(crate) fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub(crate) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn move_end(&mut self) {
        self.cursor = self.len();
    }

    pub(crate) fn len(&self) -> usize {
        self.value.chars()
    }

    /// Draws a dot per character, scrolled like a [`TextInput`].
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect, focused: bool, theme: &Theme) {
        let width = usize::from(area.width).max(1);
        let offset = (self.cursor + 1).saturating_sub(width);
        let visible = "•".repeat(self.len().saturating_sub(offset).min(width));
        render_line(
            frame,
            area,
            (visible, self.placeholder),
            focused.then_some(self.cursor - offset),
            theme,
        );
    }
}

/// Draws the visible text of an input, or its placeholder when there is
/// none, and places the terminal cursor at `cursor` cells in, if given. The
/// terminal cursor is only placed on the focused input.
fn render_line(
    frame: &mut Frame,
    area: Rect,
    (visible, placeholder): (String, Option<&'static str>),
    cursor: Option<usize>,
    theme: &Theme,
) {
    match (visible.is_empty(), placeholder) {
        (true, Some(placeholder)) => {
            frame.render_widget(
                Paragraph::new(placeholder).style(theme.field_placeholder()),
                area,
            );
        }
        _ => frame.render_widget(Paragraph::new(visible).style(theme.field()), area),
    }
    if let Some(cursor) = cursor {
        let x = area.x + u16::try_from(cursor).unwrap_or(0);
        frame.set_cursor_position((x, area.y));
    }
}

#[cfg(test)]
#[path = "textinput_tests.rs"]
mod textinput_tests;
