//! The query history: every statement sent to a database, per connection,
//! as JSON lines.
//!
//! ```text
//! {"ts":"2026-10-13T09:12:44Z","sql":"SELECT * FROM users WHERE id = 3","ok":true,"rows":1,"ms":4}
//! {"ts":"2026-10-13T09:13:02Z","sql":"DROP TABLE tmp","ok":false,"error":"table \"tmp\" does not exist","ms":2}
//! ```
//!
//! Pure data and text: reading and writing the files is the app's business.
//! A file is appended to one line at a time, so a statement run twice in a
//! row is two lines on disk until the next compaction; reading it back
//! merges them again, the same way [`History::push`] does.

use std::{collections::VecDeque, ops::Range};

use serde::{Deserialize, Serialize};

use crate::{
    fuzzy,
    sql::{Dialect, TokenKind, lex},
};

/// How many entries a connection keeps unless the config says otherwise.
pub const DEFAULT_CAP: usize = 1000;

/// The words whose literal is redacted unless the config says otherwise.
pub const DEFAULT_REDACT: [&str; 3] = ["PASSWORD", "IDENTIFIED BY", "SECRET"];

/// What a redacted literal becomes.
pub const REDACTED: &str = "'•••'";

/// How much of a statement the search looks at. The fuzzy match tries
/// every start of the query's first character, so a 100 KB statement in
/// full would make each keystroke slow; what is past this is not on screen
/// anyway.
pub const SEARCH_CHARS: usize = 2000;

/// One statement that was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// When it finished, in seconds since the Unix epoch (UTC); `None` when
    /// the file held no readable time.
    pub ts: Option<i64>,
    pub sql: String,
    pub ok: bool,
    /// The rows it returned or changed, when the server said.
    pub rows: Option<u64>,
    /// How long it took.
    pub ms: u64,
    /// Why it failed.
    pub error: Option<String>,
}

/// A line of a history file that could not be read, and was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    /// From 1.
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// One connection's history, oldest first, at most `cap` entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    entries: VecDeque<Entry>,
    cap: usize,
}

impl Default for History {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

/// One line of the file, as written.
#[derive(Serialize, Deserialize)]
struct Line {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    sql: String,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rows: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    ms: u64,
}

impl History {
    /// An empty history keeping at most `cap` entries (at least one).
    pub fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            cap: cap.max(1),
        }
    }

    /// Keeps at most `cap` entries from now on, dropping the oldest.
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap.max(1);
        self.trim();
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry `i`, counting from the oldest.
    pub fn get(&self, i: usize) -> Option<&Entry> {
        self.entries.get(i)
    }

    /// Oldest first.
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> + ExactSizeIterator + '_ {
        self.entries.iter()
    }

    /// Adds `e` as the newest entry. The same statement as the newest one
    /// (by [`normalise`]) only brings that one up to date, so running a
    /// query again and again keeps one entry. Returns whether it merged.
    pub fn push(&mut self, e: Entry) -> bool {
        if let Some(last) = self.entries.back_mut()
            && normalise(&last.sql) == normalise(&e.sql)
        {
            last.ts = e.ts;
            last.ok = e.ok;
            last.ms = e.ms;
            last.rows = e.rows;
            last.error = e.error;
            return true;
        }
        self.entries.push_back(e);
        self.trim();
        false
    }

    /// Takes entry `i` out.
    pub fn remove(&mut self, i: usize) -> Option<Entry> {
        self.entries.remove(i)
    }

    fn trim(&mut self) {
        while self.entries.len() > self.cap {
            self.entries.pop_front();
        }
    }

    /// The entries `q` fuzzily matches, best first and, between equals,
    /// newest first: the entry's index, its score and the matched
    /// characters, as char indices into [`one_line`] of its SQL. An empty
    /// query matches everything, newest first.
    pub fn search(&self, q: &str) -> Vec<(usize, i32, Vec<usize>)> {
        let mut found: Vec<(usize, i32, Vec<usize>)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let m = fuzzy::score(q, &searched(&e.sql))?;
                Some((i, m.score, m.positions))
            })
            .collect();
        found.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        found
    }

    /// The whole history as a file: one line per entry, oldest first.
    pub fn to_jsonl(&self) -> String {
        self.entries.iter().map(to_line).collect()
    }

    /// Reads a file, keeping [`DEFAULT_CAP`] entries. A line that cannot be
    /// read (a crash in the middle of an append leaves half of one) is
    /// skipped with a warning, and the rest still loads. Consecutive
    /// duplicates merge, as they would have on [`push`](Self::push).
    pub fn from_jsonl(s: &str) -> (Self, Vec<ParseWarning>) {
        let mut history = Self::default();
        let mut warnings = Vec::new();
        for (n, line) in s.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Line>(line) {
                Ok(read) => {
                    history.push(Entry {
                        ts: read.ts.as_deref().and_then(parse_ts),
                        sql: read.sql,
                        ok: read.ok,
                        rows: read.rows,
                        ms: read.ms,
                        error: read.error,
                    });
                }
                Err(e) => warnings.push(ParseWarning {
                    line: n + 1,
                    message: e.to_string(),
                }),
            }
        }
        (history, warnings)
    }
}

/// `e` as one line of a file, with its newline.
pub fn to_line(e: &Entry) -> String {
    let line = Line {
        ts: e.ts.map(format_ts),
        sql: e.sql.clone(),
        ok: e.ok,
        rows: e.rows,
        error: e.error.clone(),
        ms: e.ms,
    };
    // A struct of strings, numbers and bools always serialises.
    let mut out = serde_json::to_string(&line).unwrap_or_default();
    out.push('\n');
    out
}

/// `sql` trimmed, with each run of whitespace made one space: what makes two
/// statements the same for [`History::push`].
pub fn normalise(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `sql` on one line: every line break and tab a space, so char indices
/// stay those of `sql`. The history dialog matches and highlights this.
pub fn one_line(sql: &str) -> String {
    sql.chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect()
}

/// What the search matches a statement against: the start of
/// [`one_line`].
fn searched(sql: &str) -> String {
    one_line(sql).chars().take(SEARCH_CHARS).collect()
}

/// `sql` with the literal after each of `words` replaced by [`REDACTED`],
/// so a password typed into `ALTER USER … PASSWORD '…'` never reaches the
/// history. The words are matched against the lexer's tokens, not the text,
/// so one inside a string or a comment is left alone; a word may be several
/// (`IDENTIFIED BY`). What is redacted is the string literal that follows,
/// past an `=` or `(` (`SET PASSWORD = '…'`, `PASSWORD('…')`) or a `FOR
/// user` (`SET PASSWORD FOR 'u'@'h' = '…'`). After `IDENTIFIED …` a bare or
/// quoted identifier is redacted too: Oracle takes the password unquoted.
pub fn redact(dialect: Dialect, sql: &str, words: &[&str]) -> String {
    let tokens: Vec<_> = lex(dialect, sql)
        .into_iter()
        .filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Comment))
        .collect();
    let text = |i: usize| &sql[tokens[i].span.clone()];
    let is_word = |i: usize, word: &str| {
        matches!(tokens[i].kind, TokenKind::Keyword | TokenKind::Ident)
            && text(i).eq_ignore_ascii_case(word)
    };
    let phrases: Vec<Vec<&str>> = words
        .iter()
        .map(|w| w.split_whitespace().collect::<Vec<_>>())
        .filter(|p| !p.is_empty())
        .collect();

    let mut cuts: Vec<Range<usize>> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let matched = phrases
            .iter()
            .filter(|p| {
                i + p.len() <= tokens.len() && p.iter().enumerate().all(|(k, w)| is_word(i + k, w))
            })
            .max_by_key(|p| p.len());
        let Some(phrase) = matched else {
            i += 1;
            continue;
        };
        let mut j = i + phrase.len();
        if j < tokens.len() && is_word(j, "FOR") {
            while j < tokens.len() && text(j) != "=" && tokens[j].kind != TokenKind::Delimiter {
                j += 1;
            }
        }
        while j < tokens.len() && matches!(text(j), "=" | "(") {
            j += 1;
        }
        let bare = phrase.iter().any(|w| w.eq_ignore_ascii_case("IDENTIFIED"));
        let secret = tokens.get(j).is_some_and(|t| match t.kind {
            TokenKind::String | TokenKind::Unterminated => true,
            TokenKind::Ident | TokenKind::QuotedIdent => bare,
            _ => false,
        });
        if secret {
            cuts.push(tokens[j].span.clone());
            i = j + 1;
        } else {
            i += phrase.len();
        }
    }

    let mut out = String::with_capacity(sql.len());
    let mut at = 0;
    for cut in cuts {
        out.push_str(&sql[at..cut.start]);
        out.push_str(REDACTED);
        at = cut.end;
    }
    out.push_str(&sql[at..]);
    out
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day counted from 1970-01-01: `(year, month, day)`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// `2026-10-13T09:12:44Z`.
pub fn format_ts(unix: i64) -> String {
    let (y, m, d) = civil_from_days(unix.div_euclid(86_400));
    let s = unix.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// Reads an RFC 3339 time (`2026-10-13T09:12:44Z`, with or without
/// fractional seconds, `Z` or an offset such as `+02:00`).
pub fn parse_ts(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: Range<usize>| -> Option<i64> {
        let part = s.get(r)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut at = 19;
    if b[at] == b'.' {
        at += 1;
        let start = at;
        while at < b.len() && b[at].is_ascii_digit() {
            at += 1;
        }
        if at == start {
            return None;
        }
    }
    let offset = match b.get(at)? {
        b'Z' | b'z' if at + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if at + 6 == b.len() && b[at + 3] == b':' => {
            let minutes = num(at + 1..at + 3)? * 60 + num(at + 4..at + 6)?;
            if *sign == b'+' { minutes } else { -minutes }
        }
        _ => return None,
    };
    let mo = u32::try_from(mo).ok()?;
    let d = u32::try_from(d).ok()?;
    let days = days_from_civil(y, mo, d);
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - offset * 60)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// When an entry ran, as the history dialog says it: `09:12` today,
/// `yday`, then `Oct 03`; `?` without a time. `now` is the current time
/// and `offset` the local zone's, in seconds east of UTC.
pub fn relative(ts: Option<i64>, now: i64, offset: i32) -> String {
    let Some(ts) = ts else {
        return "?".into();
    };
    let local = ts + i64::from(offset);
    let day = local.div_euclid(86_400);
    let today = (now + i64::from(offset)).div_euclid(86_400);
    if day == today {
        let s = local.rem_euclid(86_400);
        format!("{:02}:{:02}", s / 3600, s / 60 % 60)
    } else if day + 1 == today {
        "yday".into()
    } else {
        let (_, m, d) = civil_from_days(day);
        let month = MONTHS[usize::try_from(m - 1).unwrap_or(0) % 12];
        format!("{month} {d:02}")
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
