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

impl TextInput {
    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                let at = self.byte_index();
                self.value.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte_index();
                self.value.remove(at);
            }
            KeyCode::Delete if self.cursor < self.len() => {
                let at = self.byte_index();
                self.value.remove(at);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            _ => {}
        }
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