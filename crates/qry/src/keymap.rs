//! The keymap: which chord runs which [`Action`] in which [`KeyContext`].
//!
//! There is one map, filled from the `keybindings` section of the config
//! (see [`crate::config`]). The reducer looks a key up with a context stack,
//! innermost first: the open overlay's context alone (`Help`, `Palette` or
//! `Dialog`), or the focused pane's and then `Global`. A pane's chord
//! therefore shadows the same chord in `Global`.
//!
//! A binding may be a sequence of chords, such as `g g`. The keys typed so
//! far wait in `State::pending_keys` until they complete a binding, stop
//! being the start of one, or the reducer's one-second timer runs out.
//!
//! Chords are compared after [`normalise`]: terminals disagree on whether
//! `G`, `$` or `?` carry Shift, so it is dropped for characters typed
//! without Ctrl (the character already says it). Typing a character is not a
//! binding: the editor and the dialogs' text fields insert unbound
//! characters themselves.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter};

pub use crate::action::Action;
use crate::theme::Pill;

/// Where a binding applies. See the module docs for the order they are
/// looked up in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, EnumIter, Display)]
pub enum KeyContext {
    Global,
    Tree,
    Editor,
    Results,
    Inspector,
    Dialog,
    Help,
    Palette,
    Jobs,
}

impl KeyContext {
    /// The mode pill this context shows, or `None` for one that keeps the
    /// pill of whatever is underneath (the help, the dialogs and `Global`).
    pub fn pill(self) -> Option<Pill> {
        match self {
            Self::Tree | Self::Results | Self::Inspector | Self::Jobs => Some(Pill::Normal),
            Self::Editor => Some(Pill::Edit),
            Self::Palette => Some(Pill::Command),
            Self::Global | Self::Dialog | Self::Help => None,
        }
    }
}

/// What the keys typed so far mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// They complete this binding.
    Match(Action),
    /// They start a longer binding: wait for more, or for the timeout.
    Prefix,
    /// Nothing is bound to them.
    None,
}

/// Every binding, by context. A context's bindings are a plain map from the
/// chord sequence to its action: there are a few dozen, so a prefix is found
/// by looking at each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    by_context: HashMap<KeyContext, HashMap<Vec<KeyEvent>, Action>>,
}

impl Keymap {
    /// Binds `chord` (normalised here) in `context`, replacing what it ran.
    pub(crate) fn bind(&mut self, context: KeyContext, chord: &[KeyEvent], action: Action) {
        let chord = chord.iter().copied().map(normalise).collect();
        self.by_context
            .entry(context)
            .or_default()
            .insert(chord, action);
    }

    /// Removes `chord` from `context`, if it was bound.
    pub(crate) fn unbind(&mut self, context: KeyContext, chord: &[KeyEvent]) {
        let chord: Vec<_> = chord.iter().copied().map(normalise).collect();
        if let Some(bindings) = self.by_context.get_mut(&context) {
            bindings.remove(&chord);
        }
    }

    /// What `pending` means in the first context of `contexts` that knows
    /// it. A sequence that is both bound itself and the start of a longer
    /// one is a [`Lookup::Prefix`]: it only runs once the timeout passes
    /// (see [`exact`](Self::exact)).
    pub fn resolve(&self, contexts: &[KeyContext], pending: &[KeyEvent]) -> Lookup {
        let pending: Vec<_> = pending.iter().copied().map(normalise).collect();
        for context in contexts {
            let Some(bindings) = self.by_context.get(context) else {
                continue;
            };
            let longer = bindings
                .keys()
                .any(|chord| chord.len() > pending.len() && chord.starts_with(&pending));
            if longer {
                return Lookup::Prefix;
            }
            if let Some(action) = bindings.get(&pending) {
                return Lookup::Match(*action);
            }
        }
        Lookup::None
    }

    /// The action bound to exactly `pending`, ignoring longer bindings: what
    /// a waiting sequence runs once the timeout passes.
    pub fn exact(&self, contexts: &[KeyContext], pending: &[KeyEvent]) -> Option<Action> {
        let pending: Vec<_> = pending.iter().copied().map(normalise).collect();
        contexts
            .iter()
            .find_map(|context| self.by_context.get(context)?.get(&pending).copied())
    }

    /// Every chord bound to `action` in `context`, in [`display_chord`]'s
    /// order: plain ones first, then the shortest.
    pub fn chords_for(&self, context: KeyContext, action: &Action) -> Vec<Vec<KeyEvent>> {
        let mut chords: Vec<_> = self
            .bindings(context)
            .filter(|(_, a)| *a == action)
            .map(|(chord, _)| chord.clone())
            .collect();
        chords.sort_by_cached_key(|chord| display_order(chord));
        chords
    }

    /// The bindings of `context`, in no particular order.
    pub fn bindings(
        &self,
        context: KeyContext,
    ) -> impl Iterator<Item = (&Vec<KeyEvent>, &Action)> + '_ {
        self.by_context.get(&context).into_iter().flatten()
    }

    /// Every pair of bindings in one context where one is the start of the
    /// other, sorted. The shorter one only runs after the timeout.
    pub(crate) fn prefix_conflicts(&self) -> Vec<(KeyContext, Vec<KeyEvent>, Vec<KeyEvent>)> {
        let mut conflicts = Vec::new();
        for (context, bindings) in &self.by_context {
            for short in bindings.keys() {
                for long in bindings.keys() {
                    if long.len() > short.len() && long.starts_with(short) {
                        conflicts.push((*context, short.clone(), long.clone()));
                    }
                }
            }
        }
        conflicts.sort_by_cached_key(|(context, short, long)| {
            (
                *context as u8,
                chord_to_string(short),
                chord_to_string(long),
            )
        });
        conflicts
    }
}

/// A key as the keymap compares it: only Ctrl, Alt and Shift, and no event
/// kind or state.
/// - A character typed without Ctrl drops Shift: `G` is `G`, whether or not
///   the terminal says Shift.
/// - With Ctrl it is lower-cased and keeps Shift, so `ctrl-shift-z` differs
///   from `ctrl-z` on terminals that can tell them apart.
/// - An AltGr character (Ctrl+Alt on anything but a letter or digit, as
///   Windows reports AltGr) is the plain character.
/// - `BackTab` always has Shift, and Shift+Tab is `BackTab`.
pub fn normalise(key: KeyEvent) -> KeyEvent {
    let mut mods =
        key.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
    let code = match key.code {
        KeyCode::Char(c) => {
            if mods.contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !c.is_ascii_alphanumeric()
            {
                mods.remove(KeyModifiers::CONTROL | KeyModifiers::ALT);
            }
            if mods.contains(KeyModifiers::CONTROL) {
                KeyCode::Char(c.to_ascii_lowercase())
            } else {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c)
            }
        }
        KeyCode::BackTab => {
            mods.insert(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        other => other,
    };
    KeyEvent::new(code, mods)
}

/// How a chord is written for people, and in `qry --print-config`:
/// `ctrl-p`, `Enter`, `shift-Tab`, `←`, `G`, `F5`, `g g`, `alt-Del`.
/// [`parse_key_sequence`] reads it back.
pub fn chord_to_string(chord: &[KeyEvent]) -> String {
    let keys: Vec<String> = chord
        .iter()
        .map(|key| {
            let key = normalise(*key);
            let mut out = String::new();
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                out.push_str("ctrl-");
            }
            if key.modifiers.contains(KeyModifiers::ALT) {
                out.push_str("alt-");
            }
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                out.push_str("shift-");
            }
            match key.code {
                KeyCode::Char(' ') => out.push_str("Space"),
                KeyCode::Char(c) => out.push(c),
                KeyCode::Enter => out.push_str("Enter"),
                KeyCode::Esc => out.push_str("Esc"),
                KeyCode::Tab | KeyCode::BackTab => out.push_str("Tab"),
                KeyCode::Backspace => out.push_str("Backspace"),
                KeyCode::Delete => out.push_str("Del"),
                KeyCode::Insert => out.push_str("Ins"),
                KeyCode::Left => out.push('←'),
                KeyCode::Right => out.push('→'),
                KeyCode::Up => out.push('↑'),
                KeyCode::Down => out.push('↓'),
                KeyCode::Home => out.push_str("Home"),
                KeyCode::End => out.push_str("End"),
                KeyCode::PageUp => out.push_str("PgUp"),
                KeyCode::PageDown => out.push_str("PgDn"),
                KeyCode::F(n) => out.push_str(&format!("F{n}")),
                other => out.push_str(&format!("{other:?}")),
            }
            out
        })
        .collect();
    keys.join(" ")
}

/// The one chord of several to show where there is room for one, such as
/// the hint bar: one without modifiers if there is one (a terminal can
/// always send it), then the shortest. `[ctrl-Enter, F5]` gives `F5`.
/// Empty when there are none.
pub fn display_chord(chords: &[Vec<KeyEvent>]) -> String {
    chords
        .iter()
        .min_by_key(|chord| display_order(chord))
        .map(|chord| chord_to_string(chord))
        .unwrap_or_default()
}

/// Plain before modified, then shorter, then by text, so the order does not
/// depend on a map's.
pub(crate) fn display_order(chord: &[KeyEvent]) -> (bool, usize, String) {
    let text = chord_to_string(chord);
    let modified = chord
        .iter()
        .any(|key| !normalise(*key).modifiers.is_empty());
    (modified, text.chars().count(), text)
}

/// Reads a chord sequence, normalised. Two spellings:
/// - the config's, `<ctrl-q>` or `<g><g>`;
/// - [`chord_to_string`]'s, chords separated by spaces: `ctrl-q`, `g g`.
///
/// Modifiers (`ctrl-`, `alt-`, `shift-`) and named keys (`enter`, `Esc`,
/// `PgDn`, `pagedown`, `←`, `left`, …) are case-insensitive. A single
/// character keeps its case (`G` is not `g`), and `shift-g` is `G`. Write
/// `<` and `>` as `lt` and `gt` inside `<…>`.
pub fn parse_key_sequence(raw: &str) -> Result<Vec<KeyEvent>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty key".to_string());
    }
    let bracketed = trimmed.len() > 2
        && trimmed.starts_with('<')
        && trimmed.ends_with('>')
        && !trimmed.contains(char::is_whitespace);
    let keys: Vec<&str> = if bracketed {
        trimmed[1..trimmed.len() - 1].split("><").collect()
    } else {
        trimmed.split_whitespace().collect()
    };
    keys.into_iter()
        .map(|key| parse_key_event(key).map_err(|e| format!("`{raw}`: {e}")))
        .collect()
}

/// Reads one chord of a [`parse_key_sequence`], with or without its `<…>`.
pub(crate) fn parse_key_event(raw: &str) -> Result<KeyEvent, String> {
    let raw = if raw.chars().count() > 2 && raw.starts_with('<') && raw.ends_with('>') {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };
    let (rest, mods) = split_modifiers(raw);
    let code = parse_code(rest, mods).ok_or_else(|| format!("unknown key `{raw}`"))?;
    Ok(normalise(KeyEvent::new(code, mods)))
}

/// Strips `ctrl-` / `alt-` / `shift-` prefixes, in any case. A prefix with
/// nothing after it is the key itself, so `alt--` is Alt and `-`.
fn split_modifiers(raw: &str) -> (&str, KeyModifiers) {
    let mut mods = KeyModifiers::empty();
    let mut rest = raw;
    loop {
        let lower = rest.to_ascii_lowercase();
        let (len, m) = if lower.starts_with("ctrl-") && rest.len() > 5 {
            (5, KeyModifiers::CONTROL)
        } else if lower.starts_with("alt-") && rest.len() > 4 {
            (4, KeyModifiers::ALT)
        } else if lower.starts_with("shift-") && rest.len() > 6 {
            (6, KeyModifiers::SHIFT)
        } else {
            return (rest, mods);
        };
        mods.insert(m);
        rest = &rest[len..];
    }
}

fn parse_code(raw: &str, mods: KeyModifiers) -> Option<KeyCode> {
    let named = match raw.to_ascii_lowercase().as_str() {
        "esc" | "escape" => KeyCode::Esc,
        "enter" | "return" => KeyCode::Enter,
        "left" | "←" => KeyCode::Left,
        "right" | "→" => KeyCode::Right,
        "up" | "↑" => KeyCode::Up,
        "down" | "↓" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "backtab" => KeyCode::BackTab,
        "tab" if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "space" => KeyCode::Char(' '),
        "hyphen" | "minus" => KeyCode::Char('-'),
        "lt" => KeyCode::Char('<'),
        "gt" => KeyCode::Char('>'),
        f if f.len() > 1 && f.starts_with('f') => {
            let n: u8 = f[1..].parse().ok()?;
            if !(1..=12).contains(&n) {
                return None;
            }
            KeyCode::F(n)
        }
        _ => {
            let mut chars = raw.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            // A single character keeps its case; `shift-` upper-cases it.
            if mods.contains(KeyModifiers::SHIFT) {
                KeyCode::Char(c.to_ascii_uppercase())
            } else {
                KeyCode::Char(c)
            }
        }
    };
    Some(named)
}

#[cfg(test)]
#[path = "keymap_tests.rs"]
mod keymap_tests;
