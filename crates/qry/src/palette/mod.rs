//! The command palette's model: its items, what a typed query matches, and
//! what running an item means. Plain data, like the rest of the app state;
//! `reducer/palette.rs` gathers the items and runs them, and
//! `views/palette.rs` draws them.

pub mod fuzzy;

use std::cell::Cell;

use qry_core::ExportType;

use self::fuzzy::{Match, score};
use crate::{action::Action, app::dialogs::textinput::TextInput};

/// The most rows of items the palette shows at once.
pub const MAX_ROWS: usize = 17;
/// How many items run lately are remembered, newest first.
pub const RECENT: usize = 10;

/// What a parametrised command does with its argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    /// Write the results to the path, as this type with this separator.
    Export(ExportType, &'static str),
    /// Select this row of the results, from 1.
    Goto,
    /// Draw with the theme of this name.
    Theme,
}

/// What an item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteTarget {
    /// Run this action, as its key would.
    Action(Action),
    /// Open the stored connection with this id, or switch to its session.
    OpenConnection(String),
    /// Insert the saved snippet with this index into the editor; ctrl-enter
    /// runs it too.
    Snippet(usize),
    /// A command that takes the text typed after `prefix`.
    Argument {
        prefix: &'static str,
        /// What is wanted, for `needs <path>`.
        hint: &'static str,
        kind: ArgKind,
    },
}

/// One line of the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    pub label: String,
    pub category: &'static str,
    /// The key that runs it, as the help shows it, if one is bound.
    pub key: Option<String>,
    pub target: PaletteTarget,
}

/// The parametrised commands, in the order the palette lists them.
pub const ARGUMENTS: &[(&str, &str, ArgKind, &str)] = &[
    (
        "export csv ",
        "path",
        ArgKind::Export(ExportType::Csv, ","),
        "results",
    ),
    (
        "export tsv ",
        "path",
        ArgKind::Export(ExportType::Text, "\t"),
        "results",
    ),
    (
        "export json ",
        "path",
        ArgKind::Export(ExportType::Json, ""),
        "results",
    ),
    (
        "export xlsx ",
        "path",
        ArgKind::Export(ExportType::Excel, ""),
        "results",
    ),
    ("goto ", "row", ArgKind::Goto, "results"),
    ("theme ", "name", ArgKind::Theme, "view"),
];

impl PaletteItem {
    /// The parametrised commands, as items: `export csv <path>`.
    pub fn arguments() -> Vec<Self> {
        ARGUMENTS
            .iter()
            .map(|&(prefix, hint, kind, category)| Self {
                label: format!("{prefix}<{hint}>"),
                category,
                key: None,
                target: PaletteTarget::Argument { prefix, hint, kind },
            })
            .collect()
    }

    /// What the query is matched against.
    fn haystack(&self) -> String {
        format!("{} {}", self.label, self.category)
    }
}

/// The open palette.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaletteState {
    pub(crate) input: TextInput,
    pub items: Vec<PaletteItem>,
    /// The items the query matches, best first: the item's index, and
    /// where it matched (char indices into its label).
    pub matches: Vec<(usize, Match)>,
    pub selected: usize,
    /// The first match in view; kept by the view.
    pub offset: Cell<usize>,
    /// Why the last Enter did not run anything, shown in the counter.
    pub error: Option<String>,
    /// The labels of the items run lately, newest first.
    pub recent: Vec<String>,
}

impl PaletteState {
    pub fn new(items: Vec<PaletteItem>, recent: Vec<String>) -> Self {
        let mut state = Self {
            items,
            recent,
            ..Self::default()
        };
        state.refilter();
        state
    }

    pub fn query(&self) -> &str {
        &self.input.value
    }

    /// Where `label` is in the recent list, or past its end.
    fn recency(&self, label: &str) -> usize {
        self.recent
            .iter()
            .position(|r| r == label)
            .unwrap_or(usize::MAX)
    }

    /// The parametrised item the query has started to fill in, and what
    /// was typed after its prefix.
    pub fn argument(&self) -> Option<(usize, &str)> {
        let query = self.query();
        self.items
            .iter()
            .enumerate()
            .find_map(|(i, item)| match item.target {
                PaletteTarget::Argument { prefix, .. }
                    if query.len() >= prefix.len()
                        && query.is_char_boundary(prefix.len())
                        && query[..prefix.len()].eq_ignore_ascii_case(prefix) =>
                {
                    Some((i, &query[prefix.len()..]))
                }
                _ => None,
            })
    }

    /// Matches the items against the query again, from the top. An empty
    /// query lists the recent items first, then the rest in order. A query
    /// that starts a parametrised command lists that command alone.
    pub fn refilter(&mut self) {
        self.selected = 0;
        self.offset.set(0);
        self.error = None;
        if let Some((i, _)) = self.argument() {
            self.matches = vec![(
                i,
                Match {
                    score: 0,
                    positions: Vec::new(),
                },
            )];
            return;
        }
        let query = self.query().to_string();
        let mut matches: Vec<(usize, Match)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let mut m = score(&query, &item.haystack())?;
                let label = item.label.chars().count();
                m.positions.retain(|&p| p < label);
                Some((i, m))
            })
            .collect();
        matches.sort_by_key(|(i, m)| {
            let recency = self.recency(&self.items[*i].label);
            if query.is_empty() {
                (0, recency, *i)
            } else {
                (-m.score, recency, *i)
            }
        });
        self.matches = matches;
    }

    /// The selected item, if anything matches.
    pub fn selected_item(&self) -> Option<&PaletteItem> {
        self.matches
            .get(self.selected)
            .map(|(i, _)| &self.items[*i])
    }

    /// Moves the selection by `delta` rows, stopping at either end.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.matches.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// Fills the input with the selected parametrised command's prefix, so
    /// the argument can be typed. Returns whether it did.
    pub fn fill(&mut self) -> bool {
        let prefix = match self.selected_item().map(|item| &item.target) {
            Some(PaletteTarget::Argument { prefix, .. }) => *prefix,
            _ => return false,
        };
        self.input = TextInput::default();
        for c in prefix.chars() {
            self.input.insert(c);
        }
        self.refilter();
        true
    }
}

/// Remembers `label` as the newest item run, keeping [`RECENT`] of them.
pub fn remember(recent: &mut Vec<String>, label: &str) {
    recent.retain(|r| r != label);
    recent.insert(0, label.to_string());
    recent.truncate(RECENT);
}

#[cfg(test)]
#[path = "palette_tests.rs"]
mod palette_tests;
