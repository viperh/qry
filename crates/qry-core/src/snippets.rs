//! Saved snippets: named queries the user keeps, global or for one
//! connection, stored as a JSON list.
//!
//! ```text
//! [{"name":"active users","sql":"SELECT * FROM users WHERE active","connection":null,"tags":["users"]}]
//! ```
//!
//! Pure data and text: reading and writing the file is the app's business.

use serde::{Deserialize, Serialize};

/// One saved query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub sql: String,
    /// The `StoredConnection::id` it belongs to; `None` for every
    /// connection.
    #[serde(default)]
    pub connection: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Snippet {
    /// Whether it is offered while `connection` is open: a global one
    /// always is.
    pub fn applies_to(&self, connection: &str) -> bool {
        self.connection.as_deref().is_none_or(|c| c == connection)
    }
}

/// Why a snippet cannot be saved as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    NoName,
    NoSql,
    /// Another snippet in the same scope has this name.
    Taken(String),
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoName => f.write_str("A snippet needs a name"),
            Self::NoSql => f.write_str("There is no SQL to save"),
            Self::Taken(name) => write!(f, "A snippet called \"{name}\" is already saved here"),
        }
    }
}

/// Checks `snippet` against `list` before it goes in. `replacing` is the
/// index of the snippet it takes the place of (an edit or a rename), which
/// may keep its own name. Names are compared trimmed and ignoring case, and
/// only within one scope: a global and a connection's snippet may share
/// one.
pub fn validate(
    list: &[Snippet],
    snippet: &Snippet,
    replacing: Option<usize>,
) -> Result<(), Invalid> {
    let name = snippet.name.trim();
    if name.is_empty() {
        return Err(Invalid::NoName);
    }
    if snippet.sql.trim().is_empty() {
        return Err(Invalid::NoSql);
    }
    let taken = list.iter().enumerate().any(|(i, other)| {
        Some(i) != replacing
            && other.connection == snippet.connection
            && other.name.trim().eq_ignore_ascii_case(name)
    });
    if taken {
        return Err(Invalid::Taken(name.to_string()));
    }
    Ok(())
}

/// The snippets offered while `connection` is open, with their indexes into
/// `list`: its own first, then the global ones, each by name.
pub fn for_connection<'a>(list: &'a [Snippet], connection: &str) -> Vec<(usize, &'a Snippet)> {
    let mut shown: Vec<_> = list
        .iter()
        .enumerate()
        .filter(|(_, s)| s.applies_to(connection))
        .collect();
    shown.sort_by(|(_, a), (_, b)| {
        (a.connection.is_none(), a.name.to_lowercase())
            .cmp(&(b.connection.is_none(), b.name.to_lowercase()))
    });
    shown
}

/// The file's text, pretty-printed so it diffs well when synced.
pub fn to_json(list: &[Snippet]) -> String {
    let mut out = serde_json::to_string_pretty(list).unwrap_or_else(|_| "[]".into());
    out.push('\n');
    out
}

/// Reads the file's text. An empty file is an empty list.
pub fn from_json(s: &str) -> Result<Vec<Snippet>, serde_json::Error> {
    if s.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(s)
}

#[cfg(test)]
#[path = "snippets_tests.rs"]
mod snippets_tests;
