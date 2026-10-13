use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::theme::Theme;

#[derive(Default)]
pub(crate) struct TextInput {
    pub value: String,
    pub cursor: usize,
    pub masked: bool,
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
        let visible: String = if self.masked {
            "•".repeat(self.len().saturating_sub(offset).min(width))
        } else {
            self.value.chars().skip(offset).take(width).collect()
        };

        match (visible.is_empty(), self.placeholder) {
            (true, Some(placeholder)) => {
                frame.render_widget(
                    Paragraph::new(placeholder).style(theme.field_placeholder()),
                    area,
                );
            }
            _ => frame.render_widget(Paragraph::new(visible).style(theme.field()), area),
        }
        if focused {
            let x = area.x + u16::try_from(self.cursor - offset).unwrap_or(0);
            frame.set_cursor_position((x, area.y));
        }
    }
}

#[cfg(test)]
#[path = "textinput_tests.rs"]
mod textinput_tests;
