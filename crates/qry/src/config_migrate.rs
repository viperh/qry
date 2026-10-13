//! Reads the key bindings of a config written before key contexts: a
//! `keybindings` section per mode (`Home`, `AddConnModal`, …) whose values
//! may be `{ "ChangeMode": … }`, and a `panes` section per pane with each
//! pane's own command names.
//!
//! [`Config`](crate::config::Config) converts such a file instead of
//! rejecting it, and warns once. A key that cannot be converted is skipped
//! and listed in the warning.

use crossterm::event::KeyEvent;
use serde_json::{Map, Value};

use crate::{
    action::Action,
    keymap::{KeyContext, parse_key_sequence},
};

/// What an old-format file binds, in the new terms.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Migrated {
    pub(crate) bindings: Vec<(KeyContext, Vec<KeyEvent>, Action)>,
    /// One `section key: value` line per key that could not be converted.
    pub(crate) skipped: Vec<String>,
}

/// The context an old mode's section becomes: the main screen's is
/// `Global`, and every modal's is `Dialog`.
pub(crate) fn old_context(section: &str) -> Option<KeyContext> {
    match section {
        "Home" => Some(KeyContext::Global),
        "AddConnModal" | "ExpoModal" | "PasswordModal" | "ImportModal" | "ConnExportModal" => {
            Some(KeyContext::Dialog)
        }
        _ => None,
    }
}

/// Converts one old mode's section of `keybindings`.
pub(crate) fn mode_section(section: &str, context: KeyContext, keys: &Value, out: &mut Migrated) {
    convert(section, context, keys, mode_action, out);
}

/// Converts the old `panes` section. An unknown pane is skipped whole.
pub(crate) fn panes(panes: &Map<String, Value>, out: &mut Migrated) {
    for (pane, keys) in panes {
        let section = format!("panes.{pane}");
        let (context, command): (_, fn(&Value) -> Option<Action>) = match pane.as_str() {
            "Editor" => (KeyContext::Editor, |v| named(v, editor_command)),
            "Results" => (KeyContext::Results, |v| named(v, results_command)),
            "Tree" => (KeyContext::Tree, |v| named(v, tree_command)),
            // Every modal shared these.
            "Form" => (KeyContext::Dialog, |v| named(v, form_command)),
            "Help" => (KeyContext::Help, |v| named(v, help_command)),
            _ => {
                out.skipped.push(section);
                continue;
            }
        };
        convert(&section, context, keys, command, out);
    }
}

fn convert(
    section: &str,
    context: KeyContext,
    keys: &Value,
    action: fn(&Value) -> Option<Action>,
    out: &mut Migrated,
) {
    let Value::Object(keys) = keys else {
        out.skipped.push(section.to_string());
        return;
    };
    for (raw, value) in keys {
        match (parse_key_sequence(raw), action(value)) {
            (Ok(chord), Some(action)) => out.bindings.push((context, chord, action)),
            _ => out.skipped.push(format!("{section} {raw}: {value}")),
        }
    }
}

fn named(value: &Value, table: fn(&str) -> Option<Action>) -> Option<Action> {
    value.as_str().and_then(table)
}

/// The old modes' actions. Opening a modal (`ChangeMode`) becomes the
/// action that opens it, and `ChangeMode: Home` closes it. The password
/// prompt only ever opened when asked for, and `Execute` ran fixed SQL,
/// which nothing does now.
fn mode_action(value: &Value) -> Option<Action> {
    match value {
        Value::String(name) => match name.as_str() {
            "Quit" => Some(Action::Quit),
            "Suspend" => Some(Action::Suspend),
            "ClearScreen" => Some(Action::ClearScreen),
            "Help" => Some(Action::Help),
            "FocusNext" => Some(Action::FocusNext),
            "FocusPrev" => Some(Action::FocusPrev),
            _ => None,
        },
        Value::Object(map) if map.len() == 1 => match map.get("ChangeMode")?.as_str()? {
            "Home" => Some(Action::Cancel),
            "AddConnModal" => Some(Action::NewConnection),
            "ExpoModal" => Some(Action::ExportResults),
            "ImportModal" => Some(Action::ImportConnections),
            "ConnExportModal" => Some(Action::ExportConnections),
            _ => None,
        },
        _ => None,
    }
}

fn editor_command(name: &str) -> Option<Action> {
    use Action as A;
    Some(match name {
        "RunQuery" => A::RunAll,
        "NewLine" => A::NewLine,
        "Left" => A::Left,
        "Right" => A::Right,
        "Up" => A::Up,
        "Down" => A::Down,
        "WordLeft" => A::WordLeft,
        "WordRight" => A::WordRight,
        "LineStart" => A::LineStart,
        "LineEnd" => A::LineEnd,
        "Top" => A::Top,
        "Bottom" => A::Bottom,
        "PageUp" => A::PageUp,
        "PageDown" => A::PageDown,
        "SelectLeft" => A::SelectLeft,
        "SelectRight" => A::SelectRight,
        "SelectUp" => A::SelectUp,
        "SelectDown" => A::SelectDown,
        "SelectWordLeft" => A::SelectWordLeft,
        "SelectWordRight" => A::SelectWordRight,
        "SelectLineStart" => A::SelectLineStart,
        "SelectLineEnd" => A::SelectLineEnd,
        "SelectAll" => A::SelectAll,
        "DeleteBack" => A::DeleteBack,
        "DeleteForward" => A::DeleteForward,
        "DeleteWordBack" => A::DeleteWordBack,
        "DeleteWordForward" => A::DeleteWordForward,
        "DeleteToLineEnd" => A::DeleteToLineEnd,
        "Copy" => A::Copy,
        "Cut" => A::Cut,
        "Paste" => A::Paste,
        "Undo" => A::Undo,
        "Redo" => A::Redo,
        _ => return None,
    })
}

fn results_command(name: &str) -> Option<Action> {
    Some(match name {
        "Up" => Action::Up,
        "Down" => Action::Down,
        "Left" => Action::Left,
        "Right" => Action::Right,
        "PageUp" => Action::PageUp,
        "PageDown" => Action::PageDown,
        "FirstColumn" => Action::LineStart,
        "LastColumn" => Action::LineEnd,
        "FirstRow" => Action::Top,
        "LastRow" => Action::Bottom,
        _ => return None,
    })
}

fn tree_command(name: &str) -> Option<Action> {
    Some(match name {
        "Up" => Action::Up,
        "Down" => Action::Down,
        "Toggle" => Action::Toggle,
        "Connect" => Action::Open,
        "Edit" => Action::EditConnection,
        "Delete" => Action::DeleteConnection,
        "First" => Action::Top,
        "Last" => Action::Bottom,
        _ => return None,
    })
}

fn form_command(name: &str) -> Option<Action> {
    Some(match name {
        "NextField" => Action::NextField,
        "PrevField" => Action::PrevField,
        "Submit" => Action::Submit,
        "Left" => Action::Left,
        "Right" => Action::Right,
        "NextChoice" => Action::NextChoice,
        "LineStart" => Action::LineStart,
        "LineEnd" => Action::LineEnd,
        "DeleteBack" => Action::DeleteBack,
        "DeleteForward" => Action::DeleteForward,
        _ => return None,
    })
}

fn help_command(name: &str) -> Option<Action> {
    Some(match name {
        "ScrollUp" => Action::Up,
        "ScrollDown" => Action::Down,
        "PageUp" => Action::PageUp,
        "PageDown" => Action::PageDown,
        "Close" => Action::Cancel,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "config_migrate_tests.rs"]
mod config_migrate_tests;
