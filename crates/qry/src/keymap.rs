//! Keys that act inside a single pane.
//!
//! Global keys (quit, help, switching panes, …) are `Config::keybindings`:
//! `App` turns them into [`Action`](crate::action::Action)s whatever has focus.
//! The maps here come from the `panes` section of the config and are looked
//! up by the pane that has focus. Typing a character is not a binding: the
//! editor and the form's text fields insert unbound characters themselves.

use std::{collections::HashMap, hash::Hash};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Deserializer, de};
use strum::{EnumIter, IntoEnumIterator};

use crate::config::parse_key_sequence;

/// Something a pane can do. The help lists commands in declaration order.
pub trait Command: Copy + Eq + Hash + IntoEnumIterator {
    fn description(self) -> &'static str;
}

/// Which key runs which command. Pane keys are single keys, not sequences.
#[derive(Clone, Debug)]
pub struct Keymap<C>(HashMap<KeyEvent, C>);

impl<C> Default for Keymap<C> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<C: Command> Keymap<C> {
    pub fn get(&self, key: KeyEvent) -> Option<C> {
        // Compare code and modifiers only; kind and state vary by terminal.
        // BackTab *is* Shift-Tab, but not every terminal reports the Shift.
        let mut modifiers = key.modifiers;
        if key.code == KeyCode::BackTab {
            modifiers |= KeyModifiers::SHIFT;
        }
        self.0.get(&KeyEvent::new(key.code, modifiers)).copied()
    }

    /// One `(keys, description)` line per bound command, in declaration
    /// order. Several keys for the same command share a line.
    pub fn describe(&self) -> Vec<(String, &'static str)> {
        C::iter()
            .filter_map(|command| {
                let mut keys: Vec<String> = self
                    .0
                    .iter()
                    .filter(|&(_, c)| *c == command)
                    .map(|(key, _)| display_key(key))
                    .collect();
                // Named keys (arrows, Home, …) before letters: "↑ / i".
                keys.sort_by_key(|k| (k.len() == 1, k.clone()));
                (!keys.is_empty()).then(|| (keys.join(" / "), command.description()))
            })
            .collect()
    }

    /// Adds every default binding whose key the user's config left unbound.
    pub fn merge_defaults(&mut self, defaults: &Self) {
        for (key, command) in &defaults.0 {
            self.0.entry(*key).or_insert(*command);
        }
    }
}

impl<'de, C: Deserialize<'de>> Deserialize<'de> for Keymap<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        HashMap::<String, C>::deserialize(deserializer)?
            .into_iter()
            .map(|(raw, command)| {
                let keys = parse_key_sequence(&raw)
                    .map_err(|e| de::Error::custom(format!("invalid key `{raw}`: {e}")))?;
                match keys.as_slice() {
                    [key] => Ok((*key, command)),
                    _ => Err(de::Error::custom(format!(
                        "`{raw}`: a pane key must be a single key, not a sequence"
                    ))),
                }
            })
            .collect::<Result<_, _>>()
            .map(Keymap)
    }
}

/// The `panes` section of the config.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PaneKeys {
    #[serde(default)]
    pub editor: Keymap<EditorCommand>,
    #[serde(default)]
    pub results: Keymap<ResultsCommand>,
    /// Shared by every modal form.
    #[serde(default)]
    pub form: Keymap<FormCommand>,
    #[serde(default)]
    pub help: Keymap<HelpCommand>,
}

impl PaneKeys {
    pub fn merge_defaults(&mut self, defaults: &Self) {
        self.editor.merge_defaults(&defaults.editor);
        self.results.merge_defaults(&defaults.results);
        self.form.merge_defaults(&defaults.form);
        self.help.merge_defaults(&defaults.help);
    }
}

/// How a key is written in the help, e.g. `Ctrl-u`, `Shift-Tab`, `↑`, `G`.
pub fn display_key(key: &KeyEvent) -> String {
    let code = match key.code {
        KeyCode::Up => "↑".to_string(),
        KeyCode::Down => "↓".to_string(),
        KeyCode::Left => "←".to_string(),
        KeyCode::Right => "→".to_string(),
        KeyCode::BackTab => "Shift-Tab".to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    };
    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("Alt".to_string());
    }
    // A letter's case already shows Shift, and BackTab spells it out.
    if key.modifiers.contains(KeyModifiers::SHIFT)
        && !matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab)
    {
        parts.push("Shift".to_string());
    }
    parts.push(code);
    parts.join("-")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, EnumIter)]
pub enum EditorCommand {
    RunQuery,
    NewLine,
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    Top,
    Bottom,
    PageUp,
    PageDown,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectLineStart,
    SelectLineEnd,
    SelectAll,
    DeleteBack,
    DeleteForward,
    DeleteWordBack,
    DeleteWordForward,
    DeleteToLineEnd,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
}

impl Command for EditorCommand {
    fn description(self) -> &'static str {
        match self {
            Self::RunQuery => "Run query",
            Self::NewLine => "New line",
            Self::Left => "Cursor left",
            Self::Right => "Cursor right",
            Self::Up => "Cursor up",
            Self::Down => "Cursor down",
            Self::WordLeft => "Previous word",
            Self::WordRight => "Next word",
            Self::LineStart => "Start of line",
            Self::LineEnd => "End of line",
            Self::Top => "Start of query",
            Self::Bottom => "End of query",
            Self::PageUp => "Page up",
            Self::PageDown => "Page down",
            Self::SelectLeft => "Select left",
            Self::SelectRight => "Select right",
            Self::SelectUp => "Select up",
            Self::SelectDown => "Select down",
            Self::SelectWordLeft => "Select previous word",
            Self::SelectWordRight => "Select next word",
            Self::SelectLineStart => "Select to start of line",
            Self::SelectLineEnd => "Select to end of line",
            Self::SelectAll => "Select all",
            Self::DeleteBack => "Delete character before cursor",
            Self::DeleteForward => "Delete character after cursor",
            Self::DeleteWordBack => "Delete word before cursor",
            Self::DeleteWordForward => "Delete word after cursor",
            Self::DeleteToLineEnd => "Delete to end of line",
            Self::Copy => "Copy selection",
            Self::Cut => "Cut selection",
            Self::Paste => "Paste",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, EnumIter)]
pub enum ResultsCommand {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    FirstColumn,
    LastColumn,
    FirstRow,
    LastRow,
}

impl Command for ResultsCommand {
    fn description(self) -> &'static str {
        match self {
            Self::Up => "Up a row",
            Self::Down => "Down a row",
            Self::Left => "Left a column",
            Self::Right => "Right a column",
            Self::PageUp => "Up a page",
            Self::PageDown => "Down a page",
            Self::FirstColumn => "First column",
            Self::LastColumn => "Last column",
            Self::FirstRow => "First row",
            Self::LastRow => "Last row",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, EnumIter)]
pub enum FormCommand {
    NextField,
    PrevField,
    Submit,
    /// Moves the cursor in a text field, or picks the previous choice.
    Left,
    /// Moves the cursor in a text field, or picks the next choice.
    Right,
    /// Only acts on choice fields; text fields type the key instead.
    NextChoice,
    LineStart,
    LineEnd,
    DeleteBack,
    DeleteForward,
}

impl Command for FormCommand {
    fn description(self) -> &'static str {
        match self {
            Self::NextField => "Next field",
            Self::PrevField => "Previous field",
            Self::Submit => "Submit the form",
            Self::Left => "Cursor left, or previous choice",
            Self::Right => "Cursor right, or next choice",
            Self::NextChoice => "Next choice",
            Self::LineStart => "Start of field",
            Self::LineEnd => "End of field",
            Self::DeleteBack => "Delete character before cursor",
            Self::DeleteForward => "Delete character after cursor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, EnumIter)]
pub enum HelpCommand {
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Close,
}

impl Command for HelpCommand {
    fn description(self) -> &'static str {
        match self {
            Self::ScrollUp => "Scroll up",
            Self::ScrollDown => "Scroll down",
            Self::PageUp => "Page up",
            Self::PageDown => "Page down",
            Self::Close => "Close help",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap(json: &str) -> Result<Keymap<ResultsCommand>, json5::Error> {
        json5::from_str(json)
    }

    #[test]
    fn looks_up_keys_by_code_and_modifiers() {
        let map = keymap(r#"{ "<up>": "Up", "<i>": "Up", "<shift-g>": "LastRow" }"#).unwrap();
        assert_eq!(map.get(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)), Some(ResultsCommand::Up));
        assert_eq!(map.get(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)), Some(ResultsCommand::Up));
        assert_eq!(
            map.get(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            Some(ResultsCommand::LastRow)
        );
        assert_eq!(map.get(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)), None);
    }

    #[test]
    fn rejects_sequences_and_unknown_commands() {
        assert!(keymap(r#"{ "<g><g>": "FirstRow" }"#).is_err());
        assert!(keymap(r#"{ "<g>": "Teleport" }"#).is_err());
        assert!(keymap(r#"{ "<ctrl-notakey>": "Up" }"#).is_err());
    }

    #[test]
    fn describe_groups_keys_in_command_order() {
        let map = keymap(r#"{ "<i>": "Up", "<up>": "Up", "<shift-g>": "LastRow", "<home>": "FirstColumn" }"#)
            .unwrap();
        assert_eq!(
            map.describe(),
            [
                ("↑ / i".to_string(), "Up a row"),
                ("Home".to_string(), "First column"),
                ("G".to_string(), "Last row"),
            ]
        );
    }

    #[test]
    fn user_keys_win_over_defaults() {
        let mut user = keymap(r#"{ "<g>": "LastRow" }"#).unwrap();
        user.merge_defaults(&keymap(r#"{ "<g>": "FirstRow", "<up>": "Up" }"#).unwrap());
        assert_eq!(user.get(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)), Some(ResultsCommand::LastRow));
        assert_eq!(user.get(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)), Some(ResultsCommand::Up));
    }

    #[test]
    fn keys_display_readably() {
        let show = |code, mods| display_key(&KeyEvent::new(code, mods));
        assert_eq!(show(KeyCode::Char('u'), KeyModifiers::CONTROL), "Ctrl-u");
        assert_eq!(show(KeyCode::BackTab, KeyModifiers::SHIFT), "Shift-Tab");
        assert_eq!(show(KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::SHIFT), "Ctrl-Shift-←");
        assert_eq!(show(KeyCode::F(8), KeyModifiers::NONE), "F8");
        assert_eq!(show(KeyCode::Char(' '), KeyModifiers::NONE), "Space");
        assert_eq!(show(KeyCode::PageDown, KeyModifiers::NONE), "PageDown");
    }
}
