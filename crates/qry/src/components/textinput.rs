use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;


#[derive(Default)]
pub(crate) struct TextInput {
    pub value: String,
    pub cursor: usize,
    pub masked: bool,
}

/// The character a key types, if it is plain typing rather than a shortcut.
///
/// Windows reports AltGr as Ctrl+Alt, and on many layouts AltGr is how you
/// type `\`, `@` or `{`. So only a lone Ctrl or Alt makes a shortcut; both
/// together is someone typing.
pub fn typed_char(key: KeyEvent) -> Option<char> {
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
    pub fn insert(&mut self, c: char) {
        let at = self.byte_index();
        self.value.insert(at, c);
        self.cursor += 1;
    }

    pub fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte_index();
            self.value.remove(at);
        }
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte_index();
            self.value.remove(at);
        }
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.len();
    }

    pub fn len(&self) -> usize {
        self.value.chars().count()
    }

    pub fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .nth(self.cursor)
            .map_or(self.value.len(), |(i, _)| i)
    }

    /// Draws the input, scrolled so the cursor stays visible. The terminal
    /// cursor is only placed on the focused input.
    pub fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let width = usize::from(area.width).max(1);
        let offset = (self.cursor + 1).saturating_sub(width);
        let visible: String = if self.masked {
            "•".repeat(self.len().saturating_sub(offset).min(width))
        } else {
            self.value.chars().skip(offset).take(width).collect()
        };

        frame.render_widget(Paragraph::new(visible), area);
        if focused {
            let x = area.x + u16::try_from(self.cursor - offset).unwrap_or(0);
            frame.set_cursor_position((x, area.y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(c: char, modifiers: KeyModifiers) -> Option<char> {
        typed_char(KeyEvent::new(KeyCode::Char(c), modifiers))
    }

    #[test]
    fn altgr_characters_are_typed_but_shortcuts_are_not() {
        assert_eq!(typed('\\', KeyModifiers::NONE), Some('\\'));
        assert_eq!(typed('A', KeyModifiers::SHIFT), Some('A'));
        // AltGr, as Windows reports it.
        assert_eq!(typed('\\', KeyModifiers::CONTROL | KeyModifiers::ALT), Some('\\'));
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
}
