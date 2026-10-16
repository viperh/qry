//! The record inspector's logic, without a terminal: a column's profile
//! (its NULLs, distinct values and most frequent values) counted a batch of
//! rows at a time, and one value laid out for the value popup as wrapped
//! text, pretty JSON or a hex dump.

use std::collections::HashMap;

use unicode_width::UnicodeWidthChar;

use crate::{QueryResult, ValueKind, grid};

/// How many of the most frequent values a profile keeps.
pub const TOP_VALUES: usize = 5;
/// Deeper JSON than this is shown as text, so a hostile value cannot
/// exhaust the stack.
const MAX_JSON_DEPTH: usize = 256;

/// What a column holds, over the rows looked at so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColumnProfile {
    pub nulls: u64,
    pub distinct: u64,
    /// The most frequent non-NULL values with their counts, the most
    /// frequent first and ties in value order; filled in once complete.
    pub top: Vec<(String, u64)>,
    /// Rows looked at. Below the result's row count once the cap is hit.
    pub sampled: u64,
    pub complete: bool,
}

/// Counts one column a batch of rows at a time, so a million-row result
/// never holds up input. Stops after `cap` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiler {
    col: usize,
    cap: usize,
    next: usize,
    steps: u64,
    counts: HashMap<String, u64>,
    profile: ColumnProfile,
}

impl Profiler {
    /// A profiler for column `col` that looks at no more than `cap` rows.
    pub fn new(col: usize, cap: usize) -> Self {
        Self {
            col,
            cap,
            next: 0,
            steps: 0,
            counts: HashMap::new(),
            profile: ColumnProfile::default(),
        }
    }

    /// The column it counts.
    pub fn column(&self) -> usize {
        self.col
    }

    /// The profile so far.
    pub fn profile(&self) -> &ColumnProfile {
        &self.profile
    }

    /// How many batches it has counted, for a spinner's frame.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Counts up to `budget` more rows of `result`. Returns the profile
    /// once every row up to the cap has been counted.
    pub fn step(&mut self, result: &QueryResult, budget: usize) -> Option<&ColumnProfile> {
        if !self.profile.complete {
            let last = result.rows.len().min(self.cap);
            let end = last.min(self.next.saturating_add(budget.max(1)));
            for row in result.rows.get(self.next..end).unwrap_or_default() {
                match row.get(self.col).and_then(Option::as_deref) {
                    None => self.profile.nulls += 1,
                    Some(value) => match self.counts.get_mut(value) {
                        Some(n) => *n += 1,
                        None => _ = self.counts.insert(value.to_string(), 1),
                    },
                }
            }
            self.next = end.max(self.next);
            self.steps += 1;
            self.profile.sampled = self.next as u64;
            self.profile.distinct = self.counts.len() as u64;
            if self.next >= last {
                self.profile.complete = true;
                self.profile.top = top_values(std::mem::take(&mut self.counts));
            }
        }
        self.profile.complete.then_some(&self.profile)
    }
}

/// The [`TOP_VALUES`] most frequent values, by count and then by value.
fn top_values(counts: HashMap<String, u64>) -> Vec<(String, u64)> {
    let mut all: Vec<(String, u64)> = counts.into_iter().collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    all.truncate(TOP_VALUES);
    all
}

/// What a piece of pretty JSON is, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonTok {
    Key,
    String,
    Number,
    /// `true`, `false` and `null`.
    Literal,
    /// Brackets, commas, colons and indentation.
    Punct,
}

/// One value laid out for the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendered {
    Text(Vec<String>),
    Json(Vec<Vec<(JsonTok, String)>>),
    Hex(Vec<String>),
    Null,
}

impl Rendered {
    /// How many lines it takes.
    pub fn len(&self) -> usize {
        match self {
            Self::Text(lines) | Self::Hex(lines) => lines.len(),
            Self::Json(lines) => lines.len(),
            Self::Null => 1,
        }
    }

    /// Whether it has no lines, as an empty binary value has.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether `value` is shown as JSON: a JSON column, or text that parses as
/// an object or an array.
pub fn is_json(value: &str, kind: ValueKind) -> bool {
    let trimmed = value.trim_start();
    let looks = kind == ValueKind::Json || trimmed.starts_with('{') || trimmed.starts_with('[');
    looks && parse_json(value).is_some()
}

/// Lays `value` out `width` cells wide: NULL, a hex dump for binary, pretty
/// JSON (or the raw text when `pretty` is off), or text, word-wrapped when
/// `wrap` is on. Invalid JSON is shown as text.
pub fn render_value(
    value: Option<&str>,
    kind: ValueKind,
    width: u16,
    wrap: bool,
    pretty: bool,
) -> Rendered {
    let Some(value) = value else {
        return Rendered::Null;
    };
    if kind == ValueKind::Binary {
        return Rendered::Hex(hex_dump(&binary_bytes(value)));
    }
    if pretty
        && is_json(value, kind)
        && let Some(json) = parse_json(value)
    {
        let mut lines = vec![Vec::new()];
        pretty_json(&json, 0, &mut lines);
        return Rendered::Json(lines);
    }
    Rendered::Text(text_lines(value, usize::from(width), wrap))
}

/// The lines of `text`, each wrapped to `width` when `wrap` is on. Tabs
/// become four spaces and other control characters the grid's glyphs.
fn text_lines(text: &str, width: usize, wrap: bool) -> Vec<String> {
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let line = line
            .strip_suffix('\r')
            .unwrap_or(line)
            .replace('\t', "    ");
        let line = grid::display_text(Some(&line)).into_owned();
        if wrap {
            lines.extend(wrap_words(&line, width));
        } else {
            lines.push(line);
        }
    }
    lines
}

/// The bytes a backend's text for a binary value stands for: SQLite's and
/// Oracle's `x'48656c'`, Postgres's `\x48656c` or `0x48656c` are decoded;
/// anything else is taken as it is.
pub fn binary_bytes(text: &str) -> Vec<u8> {
    let hex = text
        .strip_prefix("x'")
        .or_else(|| text.strip_prefix("X'"))
        .and_then(|rest| rest.strip_suffix('\''))
        .or_else(|| text.strip_prefix("\\x"))
        .or_else(|| text.strip_prefix("0x"));
    hex.and_then(|hex| hex::decode(hex).ok())
        .unwrap_or_else(|| text.as_bytes().to_vec())
}

/// `hexdump -C`'s layout: an offset, 16 bytes in two groups of 8, and the
/// printable ASCII between bars, one line per 16 bytes.
pub fn hex_dump(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(i, chunk)| {
            let mut line = format!("{:08x}  ", i * 16);
            for j in 0..16 {
                match chunk.get(j) {
                    Some(b) => line.push_str(&format!("{b:02x} ")),
                    None => line.push_str("   "),
                }
                if j == 7 {
                    line.push(' ');
                }
            }
            line.push(' ');
            line.push('|');
            line.extend(chunk.iter().map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    char::from(b)
                } else {
                    '.'
                }
            }));
            line.push('|');
            line
        })
        .collect()
}

/// `text` in lines at most `width` cells wide (at least 1), breaking
/// between words; a word wider than a line is broken where it must be.
/// Runs of whitespace become one space, and none is kept at a break. A
/// character wider than `width` still gets a line of its own.
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let w = grid::width(word);
        if used > 0 && used + 1 + w <= width {
            line.push(' ');
            line.push_str(word);
            used += 1 + w;
            continue;
        }
        if used > 0 {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if w <= width {
            line.push_str(word);
            used = w;
            continue;
        }
        for c in word.chars() {
            let cw = c.width().unwrap_or(0);
            if used > 0 && used + cw > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            line.push(c);
            used += cw;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// A parsed JSON value that keeps its keys' order and its scalars' text.
#[derive(Debug)]
enum Json<'a> {
    Object(Vec<(&'a str, Json<'a>)>),
    Array(Vec<Json<'a>>),
    /// With its quotes and escapes, as written.
    String(&'a str),
    Number(&'a str),
    Literal(&'a str),
}

/// `text` as one JSON value with nothing but whitespace around it.
fn parse_json(text: &str) -> Option<Json<'_>> {
    let mut parser = Parser { text, at: 0 };
    let value = parser.value(0)?;
    parser.space();
    (parser.at == text.len()).then_some(value)
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> Option<()> {
        self.space();
        (self.peek() == Some(byte)).then(|| self.at += 1)
    }

    fn value(&mut self, depth: usize) -> Option<Json<'a>> {
        if depth > MAX_JSON_DEPTH {
            return None;
        }
        self.space();
        match self.peek()? {
            b'{' => {
                self.at += 1;
                let mut members = Vec::new();
                if self.eat(b'}').is_some() {
                    return Some(Json::Object(members));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.eat(b':')?;
                    members.push((key, self.value(depth + 1)?));
                    if self.eat(b',').is_none() {
                        self.eat(b'}')?;
                        return Some(Json::Object(members));
                    }
                }
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                if self.eat(b']').is_some() {
                    return Some(Json::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b',').is_none() {
                        self.eat(b']')?;
                        return Some(Json::Array(items));
                    }
                }
            }
            b'"' => self.string().map(Json::String),
            b't' | b'f' | b'n' => {
                let rest = &self.text[self.at..];
                let word = ["true", "false", "null"]
                    .into_iter()
                    .find(|w| rest.starts_with(w))?;
                self.at += word.len();
                Some(Json::Literal(word))
            }
            _ => self.number().map(Json::Number),
        }
    }

    /// A string at the cursor, quotes and all.
    fn string(&mut self) -> Option<&'a str> {
        let start = self.at;
        if self.peek()? != b'"' {
            return None;
        }
        self.at += 1;
        loop {
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(&self.text[start..self.at]);
                }
                b'\\' => {
                    self.at += 1;
                    match self.peek()? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => self.at += 1,
                        b'u' => {
                            let hex = self.text.get(self.at + 1..self.at + 5)?;
                            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                                return None;
                            }
                            self.at += 5;
                        }
                        _ => return None,
                    }
                }
                b if b < 0x20 => return None,
                _ => self.at += 1,
            }
        }
    }

    /// `-?(0|[1-9][0-9]*)(.[0-9]+)?([eE][+-]?[0-9]+)?` at the cursor.
    fn number(&mut self) -> Option<&'a str> {
        let start = self.at;
        let digits = |p: &mut Self| {
            let from = p.at;
            while p.peek().is_some_and(|b| b.is_ascii_digit()) {
                p.at += 1;
            }
            p.at > from
        };
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        if self.peek() == Some(b'0') {
            self.at += 1;
        } else if !digits(self) {
            return None;
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !digits(self) {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !digits(self) {
                return None;
            }
        }
        Some(&self.text[start..self.at])
    }
}

/// Appends `json` to the last of `lines`, starting new lines indented two
/// spaces a level as it goes.
fn pretty_json(json: &Json<'_>, depth: usize, lines: &mut Vec<Vec<(JsonTok, String)>>) {
    let push = |lines: &mut Vec<Vec<(JsonTok, String)>>, tok: JsonTok, text: &str| {
        if let Some(line) = lines.last_mut() {
            line.push((tok, text.to_string()));
        }
    };
    let newline = |lines: &mut Vec<Vec<(JsonTok, String)>>, depth: usize| {
        let indent = "  ".repeat(depth);
        lines.push(if indent.is_empty() {
            Vec::new()
        } else {
            vec![(JsonTok::Punct, indent)]
        });
    };
    match json {
        Json::Object(members) if members.is_empty() => push(lines, JsonTok::Punct, "{}"),
        Json::Array(items) if items.is_empty() => push(lines, JsonTok::Punct, "[]"),
        Json::Object(members) => {
            push(lines, JsonTok::Punct, "{");
            for (i, (key, value)) in members.iter().enumerate() {
                newline(lines, depth + 1);
                push(lines, JsonTok::Key, key);
                push(lines, JsonTok::Punct, ": ");
                pretty_json(value, depth + 1, lines);
                if i + 1 < members.len() {
                    push(lines, JsonTok::Punct, ",");
                }
            }
            newline(lines, depth);
            push(lines, JsonTok::Punct, "}");
        }
        Json::Array(items) => {
            push(lines, JsonTok::Punct, "[");
            for (i, item) in items.iter().enumerate() {
                newline(lines, depth + 1);
                pretty_json(item, depth + 1, lines);
                if i + 1 < items.len() {
                    push(lines, JsonTok::Punct, ",");
                }
            }
            newline(lines, depth);
            push(lines, JsonTok::Punct, "]");
        }
        Json::String(text) => push(lines, JsonTok::String, text),
        Json::Number(text) => push(lines, JsonTok::Number, text),
        Json::Literal(text) => push(lines, JsonTok::Literal, text),
    }
}

#[cfg(test)]
#[path = "inspect_tests.rs"]
mod inspect_tests;
