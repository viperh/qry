//! What the help overlay lists, built from the live keymap, after tachy's
//! `help_model.rs`: groups of `(keys, description)` rows, the footer's
//! cheat-sheet and the help's own keys.
//!
//! Nothing here is hardcoded per key: remapping a key changes the help, and
//! an action bound to nothing is left out. T401 builds
//! `docs/keybindings.md` from the same model.

use crate::{
    action::Action,
    keymap::{KeyContext, Keymap, chord_to_string, display_chord},
};

/// One group of rows under a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpGroup {
    pub title: &'static str,
    /// `(keys, description)`, in the actions' order.
    pub entries: Vec<(String, String)>,
    pub notes: Vec<&'static str>,
    /// Drawn right under the group before it, with no blank line, and
    /// never in another column.
    pub continues: bool,
}

/// Which actions of a context a group lists.
#[derive(Debug, Clone, Copy)]
enum Pick {
    All,
    Only(&'static [Action]),
    /// Everything not in these, so nothing bound is hidden.
    Except(&'static [&'static [Action]]),
}

struct GroupSpec {
    title: &'static str,
    context: KeyContext,
    pick: Pick,
    notes: &'static [&'static str],
    continues: bool,
}

const SESSIONS: &[Action] = &[
    Action::SwitchSession(1),
    Action::NextSession,
    Action::PrevSession,
    Action::CloseSession,
];
const EDITOR_RUNNING: &[Action] = &[
    Action::RunStatement,
    Action::RunAll,
    Action::RunSelection,
    Action::CancelQuery,
];
const EDITOR_EDITING: &[Action] = &[
    Action::NewLine,
    Action::Indent,
    Action::Left,
    Action::Right,
    Action::Up,
    Action::Down,
    Action::WordLeft,
    Action::WordRight,
    Action::LineStart,
    Action::LineEnd,
    Action::Top,
    Action::Bottom,
    Action::PageUp,
    Action::PageDown,
    Action::SelectLeft,
    Action::SelectRight,
    Action::SelectUp,
    Action::SelectDown,
    Action::SelectWordLeft,
    Action::SelectWordRight,
    Action::SelectLineStart,
    Action::SelectLineEnd,
    Action::SelectAll,
    Action::DeleteBack,
    Action::DeleteForward,
    Action::DeleteWordBack,
    Action::DeleteWordForward,
    Action::DeleteToLineEnd,
    Action::Undo,
    Action::Redo,
    Action::Cut,
    Action::Copy,
    Action::Paste,
];
const RESULTS_NAV: &[Action] = &[
    Action::PrevResult,
    Action::NextResult,
    Action::Up,
    Action::Down,
    Action::Left,
    Action::Right,
    Action::PageUp,
    Action::PageDown,
    Action::Top,
    Action::Bottom,
    Action::LineStart,
    Action::LineEnd,
];
const RESULTS_COPY: &[Action] = &[
    Action::CopyCell,
    Action::CopyRow,
    Action::CopyMenu,
    Action::Search,
    Action::NextMatch,
    Action::PrevMatch,
    Action::Inspect,
    Action::ToggleInspector,
    Action::ExportResults,
];

const GROUPS: &[GroupSpec] = &[
    GroupSpec::new("Global", KeyContext::Global, Pick::Except(&[SESSIONS])),
    GroupSpec::new("Tree", KeyContext::Tree, Pick::All),
    GroupSpec::new(
        "Editor · Editing",
        KeyContext::Editor,
        Pick::Only(EDITOR_EDITING),
    ),
    GroupSpec::new(
        "Editor · Running",
        KeyContext::Editor,
        Pick::Only(EDITOR_RUNNING),
    ),
    GroupSpec::new(
        "Editor · Other",
        KeyContext::Editor,
        Pick::Except(&[EDITOR_EDITING, EDITOR_RUNNING]),
    )
    .continuing(),
    GroupSpec::new(
        "Results · Navigation",
        KeyContext::Results,
        Pick::Only(RESULTS_NAV),
    ),
    GroupSpec::new(
        "Results · Copy & search",
        KeyContext::Results,
        Pick::Only(RESULTS_COPY),
    ),
    GroupSpec::new(
        "Results · Other",
        KeyContext::Results,
        Pick::Except(&[RESULTS_NAV, RESULTS_COPY]),
    )
    .continuing(),
    GroupSpec::new("Inspector", KeyContext::Inspector, Pick::All),
    GroupSpec::new("Sessions", KeyContext::Global, Pick::Only(SESSIONS))
        .with_notes(&["shift-Enter in the tree opens another session on a connection"]),
    GroupSpec::new("Dialogs", KeyContext::Dialog, Pick::All),
    GroupSpec::new("Command palette", KeyContext::Palette, Pick::All),
    GroupSpec::new("Jobs", KeyContext::Jobs, Pick::All),
];

impl GroupSpec {
    const fn new(title: &'static str, context: KeyContext, pick: Pick) -> Self {
        Self {
            title,
            context,
            pick,
            notes: &[],
            continues: false,
        }
    }

    const fn continuing(mut self) -> Self {
        self.continues = true;
        self
    }

    const fn with_notes(mut self, notes: &'static [&'static str]) -> Self {
        self.notes = notes;
        self
    }

    fn lists(&self, action: Action) -> bool {
        // Every numbered session counts as the first.
        let action = match action {
            Action::SwitchSession(_) => Action::SwitchSession(1),
            action => action,
        };
        match self.pick {
            Pick::All => true,
            Pick::Only(actions) => actions.contains(&action),
            Pick::Except(lists) => !lists.iter().any(|list| list.contains(&action)),
        }
    }
}

/// Descriptions that read better in one context than the action's own.
const CONTEXT_DESCRIPTIONS: &[(KeyContext, Action, &str)] = &[
    (
        KeyContext::Tree,
        Action::Open,
        "connect / switch to its session",
    ),
    (KeyContext::Tree, Action::Toggle, "expand / collapse"),
    (KeyContext::Tree, Action::Refresh, "read the tables again"),
    (KeyContext::Results, Action::Inspect, "inspect the value"),
    (KeyContext::Results, Action::Search, "search the results"),
    (KeyContext::Tree, Action::Filter, "filter the tree"),
];

/// What `action` does in `context`, one short lower-case phrase.
pub fn description(context: KeyContext, action: &Action) -> String {
    if let Action::SwitchSession(_) = action {
        return "switch to session N".into();
    }
    let text = CONTEXT_DESCRIPTIONS
        .iter()
        .find(|(c, a, _)| *c == context && a == action)
        .map_or_else(|| action.description(), |(_, _, text)| text);
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Every chord of `action` in `context`, plain first then shorter, joined
/// by spaces: `j ↓`.
fn keys(keymap: &Keymap, context: KeyContext, action: &Action) -> String {
    keymap
        .chords_for(context, action)
        .iter()
        .map(|chord| chord_to_string(chord))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The numbered sessions on one row: `ctrl-1–9` when they are the plain
/// `ctrl-N` chords, else the first and the last.
fn session_keys(keymap: &Keymap, context: KeyContext) -> Option<String> {
    let numbered: Vec<(u8, String)> = (1..=9)
        .filter_map(|n| {
            let keys = keys(keymap, context, &Action::SwitchSession(n));
            (!keys.is_empty()).then_some((n, keys))
        })
        .collect();
    let (first, last) = (numbered.first()?, numbered.last()?);
    let plain = numbered
        .iter()
        .all(|(n, keys)| *keys == format!("ctrl-{n}"));
    Some(if numbered.len() == 1 {
        first.1.clone()
    } else if plain {
        format!("ctrl-{}–{}", first.0, last.0)
    } else {
        format!("{} … {}", first.1, last.1)
    })
}

/// The groups the help shows, from `keymap`. Empty groups are dropped.
pub fn build(keymap: &Keymap) -> Vec<HelpGroup> {
    let mut groups = Vec::new();
    for spec in GROUPS {
        let mut actions: Vec<Action> = keymap
            .bindings(spec.context)
            .map(|(_, action)| *action)
            .filter(|action| spec.lists(*action))
            .collect();
        actions.sort_by_key(Action::rank);
        actions.dedup();
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut sessions_done = false;
        for action in actions {
            let keys = match action {
                Action::SwitchSession(_) if sessions_done => continue,
                Action::SwitchSession(_) => {
                    sessions_done = true;
                    session_keys(keymap, spec.context).unwrap_or_default()
                }
                action => keys(keymap, spec.context, &action),
            };
            let text = description(spec.context, &action);
            // Rows that do the same thing share one.
            match entries.iter_mut().find(|(_, d)| *d == text) {
                Some((existing, _)) => {
                    existing.push(' ');
                    existing.push_str(&keys);
                }
                None => entries.push((keys, text)),
            }
        }
        if entries.is_empty() {
            continue;
        }
        groups.push(HelpGroup {
            title: spec.title,
            entries,
            notes: spec.notes.to_vec(),
            continues: spec.continues,
        });
    }
    // A continuation with nothing before it stands on its own.
    if let Some(first) = groups.first_mut() {
        first.continues = false;
    }
    groups
}

/// One piece of a footer item: a key, or words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Key(String),
    Text(&'static str),
}

/// One item of the footer's cheat-sheet: `run:` and its pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterItem {
    pub label: &'static str,
    pub pieces: Vec<Piece>,
}

/// A footer item's label, the context its keys come from, and its actions
/// with what each does.
type FooterSpec = (&'static str, KeyContext, &'static [(Action, &'static str)]);

/// What each footer item lists.
const FOOTER: &[FooterSpec] = &[
    (
        "run:",
        KeyContext::Editor,
        &[
            (Action::RunStatement, "statement"),
            (Action::RunAll, "all"),
            (Action::RunSelection, "selection"),
        ],
    ),
    (
        "results:",
        KeyContext::Results,
        &[
            (Action::Search, "search"),
            (Action::CopyCell, "copy cell"),
            (Action::CopyRow, "copy row"),
            (Action::Inspect, "inspect value"),
        ],
    ),
    (
        "tree:",
        KeyContext::Tree,
        &[
            (Action::Open, "connect"),
            (Action::Toggle, "expand"),
            (Action::EditConnection, "edit"),
            (Action::DeleteConnection, "delete"),
            (Action::Filter, "filter"),
        ],
    ),
];

/// The cheat-sheet under the groups, keys from `keymap`. A piece whose
/// action is bound to nothing is left out, and so is an item left empty.
pub fn footer(keymap: &Keymap) -> Vec<FooterItem> {
    FOOTER
        .iter()
        .filter_map(|(label, context, actions)| {
            let mut pieces = Vec::new();
            for (action, words) in *actions {
                let chords = keymap.chords_for(*context, action);
                if chords.is_empty() {
                    continue;
                }
                pieces.push(Piece::Key(display_chord(&chords)));
                pieces.push(Piece::Text(words));
            }
            (!pieces.is_empty()).then_some(FooterItem { label, pieces })
        })
        .collect()
}

/// The help's own keys for its bottom border: `k/j scroll  PgUp/PgDn page
/// Home/G top/bottom  ?/Esc close`.
pub fn help_keys(keymap: &Keymap) -> Vec<(String, &'static str)> {
    let pairs: [(&[Action], &str); 4] = [
        (&[Action::Up, Action::Down], "scroll"),
        (&[Action::PageUp, Action::PageDown], "page"),
        (&[Action::Top, Action::Bottom], "top/bottom"),
        (&[Action::Help, Action::Cancel], "close"),
    ];
    pairs
        .iter()
        .filter_map(|(actions, label)| {
            let keys: Vec<String> = actions
                .iter()
                .map(|a| keymap.chords_for(KeyContext::Help, a))
                .filter(|chords| !chords.is_empty())
                .map(|chords| display_chord(&chords))
                .collect();
            (!keys.is_empty()).then(|| (keys.join("/"), *label))
        })
        .collect()
}

#[cfg(test)]
#[path = "help_model_tests.rs"]
mod help_model_tests;
