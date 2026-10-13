use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::secrets::Password;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextInput {
    pub value: String,
    pub cursor: usize,
    /// Shown dimmed while the field is empty, for what an empty field means.
    pub placeholder: Option<&'static str>,
}

/// A password field. What is typed goes key by key into a [`Password`], so
/// it is cleared when dropped and never printed; the screen shows a dot per
/// character.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SecretInput {
    pub value: Password,
    pub cursor: usize,
    /// Shown dimmed while the field is empty, for what an empty field means.
    pub placeholder: Option<&'static str>,
}

/// The state is cloned as a whole (tests replay it), so the one copy of a
/// password a prompt keeps is made explicitly here.
impl Clone for SecretInput {
    fn clone(&self) -> Self {
        Self {
            value: self.value.duplicate(),
            cursor: self.cursor,
            placeholder: self.placeholder,
        }
    }
}

/// The character a key types, if it is plain typing rather than a shortcut.
///
/// Windows reports AltGr as Ctrl+Alt, and on many layouts AltGr is how you
/// type `\`, `@` or `{`. So only a lone Ctrl or Alt makes a shortcut; both
/// together is someone typing.
pub(crate) fn typed_char(key: KeyEvent) -> Option<char> {
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
}

#[cfg(test)]
#[path = "textinput_tests.rs"]
mod textinput_tests;
