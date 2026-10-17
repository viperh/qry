//! The Export dialog: where a result goes, in which format, and which of
//! its rows.
//!
//! Its rows depend on the format (a separator only for CSV and Text, an
//! encoding only for text, a header for all but JSON), so they are laid
//! out again after every change; a row that goes away keeps its value for
//! when it comes back. The last line previews the file the path names,
//! from what the runtime found there (see [`ExportProbe`]).

use qry_core::{
    ExportOptions, ExportType, exporter::parse_separator, jobs::format_rows, sql::Dialect,
};

use super::{
    Submission,
    form::{Field, Form, Note, Tone},
    textinput::TextInput,
};
use crate::{action::Action, app::event::ExportProbe, theme::Accent};

/// The dialog's rows, in the order they show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Path,
    Format,
    Rows,
    Separator,
    Custom,
    LineEnds,
    Header,
    NullAs,
    Table,
    Encoding,
}

impl Key {
    const ALL: [Key; 10] = [
        Key::Path,
        Key::Format,
        Key::Rows,
        Key::Separator,
        Key::Custom,
        Key::LineEnds,
        Key::Header,
        Key::NullAs,
        Key::Table,
        Key::Encoding,
    ];

    fn label(self) -> &'static str {
        match self {
            Key::Path => "Path",
            Key::Format => "Format",
            Key::Rows => "Rows",
            Key::Separator => "Separator",
            Key::Custom => "Custom",
            Key::LineEnds => "Line ends",
            Key::Header => "Header",
            Key::NullAs => "NULL as",
            Key::Table => "Table",
            Key::Encoding => "Encoding",
        }
    }
}

/// Same order as `ExportType::ALL`.
const FORMATS: [&str; 8] = [
    "CSV",
    "TSV",
    "JSON",
    "JSON lines",
    "Excel",
    "Markdown",
    "SQL INSERT",
    "Text",
];

/// Which rows: those in hand, or every row of the statement, run again.
const SCOPES: [&str; 2] = ["fetched rows", "all rows (re-run)"];
const FETCHED: usize = 0;
const ALL_ROWS: usize = 1;

const SEPARATORS: [&str; 5] = [", comma", "; semicolon", "| pipe", "tab", "other"];
/// What each of those writes, in the same order. `other` has no entry: its
/// value comes from the Custom row.
const SEPARATOR_VALUES: [char; 4] = [',', ';', '|', '\t'];
const OTHER: usize = SEPARATOR_VALUES.len();

const LINE_ENDS: [&str; 2] = ["LF", "CRLF"];
const YES_NO: [&str; 2] = ["yes", "no"];
const ENCODINGS: [&str; 2] = ["UTF-8", "UTF-8 with BOM"];

/// Which rows an export writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// The rows in memory.
    Fetched,
    /// Every row of the statement, run again with no limit.
    All,
}

/// What the dialog asks for, before the reducer adds the rows or the
/// statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExportChoice {
    /// As typed.
    pub(crate) path: String,
    pub(crate) etype: ExportType,
    pub(crate) options: ExportOptions,
    pub(crate) scope: Scope,
}

/// The result being exported, as the dialog describes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExportSubject {
    /// Its tab's number and the table it read (`result` when none).
    pub(crate) tab: usize,
    pub(crate) table: Option<String>,
    pub(crate) rows: usize,
    /// More rows exist than were fetched.
    pub(crate) truncated: bool,
    /// The statement only reads (`SELECT`, `WITH`, …), so running it again
    /// changes nothing.
    pub(crate) read_only: bool,
    pub(crate) dialect: Option<Dialect>,
}

/// What the dialog remembers for its session: the folder of the last path,
/// the format and the separator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExportMemory {
    pub(crate) dir: Option<String>,
    pub(crate) format: usize,
    pub(crate) separator: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExportForm {
    subject: ExportSubject,
    /// Every row's field, shown or not, in `Key::ALL` order.
    store: Vec<Field>,
    /// The rows shown, their labels and their fields.
    keys: Vec<Key>,
    labels: Vec<&'static str>,
    fields: Vec<Field>,
    focus: usize,
    error: Option<String>,
    /// Set while the database worker is writing the file.
    pending: Option<&'static str>,
    /// What the runtime last found at the path, and what it was last asked.
    probe: Option<ExportProbe>,
    asked: Option<(String, ExportType)>,
    /// The user said to replace the file that is there.
    pub(crate) overwrite: bool,
    /// The user said to run a statement that may change data again.
    pub(crate) rerun_confirmed: bool,
}

impl Default for ExportForm {
    fn default() -> Self {
        Self::new(ExportSubject::default(), &ExportMemory::default())
    }
}

impl ExportForm {
    /// A dialog for `subject`, with what the session remembers.
    pub(crate) fn new(subject: ExportSubject, memory: &ExportMemory) -> Self {
        let path = memory.dir.clone().unwrap_or_default();
        let table = subject.table.clone().unwrap_or_else(|| "result".into());
        let store = Key::ALL
            .iter()
            .map(|key| match key {
                Key::Path => text(path.clone()),
                Key::Format => Field::choice(&FORMATS, memory.format.min(FORMATS.len() - 1)),
                Key::Rows => Field::choice(&SCOPES, FETCHED),
                Key::Separator => {
                    Field::choice(&SEPARATORS, memory.separator.min(SEPARATORS.len() - 1))
                }
                Key::Custom => Field::text(),
                Key::LineEnds => Field::choice(&LINE_ENDS, 0),
                Key::Header => Field::choice(&YES_NO, 0),
                Key::NullAs => Field::text_with_placeholder("(empty)"),
                Key::Table => text(table.clone()),
                Key::Encoding => Field::choice(&ENCODINGS, 0),
            })
            .collect();
        let mut form = Self {
            subject,
            store,
            keys: Vec::new(),
            labels: Vec::new(),
            fields: Vec::new(),
            focus: 0,
            error: None,
            pending: None,
            probe: None,
            asked: None,
            overwrite: false,
            rerun_confirmed: false,
        };
        form.relayout();
        form
    }

    fn stored(&self, key: Key) -> &Field {
        &self.store[key as usize]
    }

    fn choice(&self, key: Key) -> usize {
        match self.stored(key) {
            Field::Choice { selected, .. } => *selected,
            _ => 0,
        }
    }

    fn value(&self, key: Key) -> &str {
        match self.stored(key) {
            Field::Text(input) => &input.value,
            _ => "",
        }
    }

    pub(crate) fn etype(&self) -> ExportType {
        ExportType::ALL[self.choice(Key::Format).min(ExportType::ALL.len() - 1)]
    }

    pub(crate) fn scope(&self) -> Scope {
        match self.choice(Key::Rows) {
            ALL_ROWS => Scope::All,
            _ => Scope::Fetched,
        }
    }

    /// The path as typed, without the spaces around it.
    pub(crate) fn path(&self) -> &str {
        self.value(Key::Path).trim()
    }

    pub(crate) fn subject(&self) -> &ExportSubject {
        &self.subject
    }

    /// What the session should remember of this dialog.
    pub(crate) fn memory(&self) -> ExportMemory {
        let path = self.path();
        let dir = path
            .rfind(['/', '\\'])
            .map(|i| path[..=i].to_string())
            .filter(|dir| !dir.is_empty());
        ExportMemory {
            dir,
            format: self.choice(Key::Format),
            separator: self.choice(Key::Separator),
        }
    }

    /// The rows a format shows.
    fn wanted(&self) -> Vec<Key> {
        let etype = self.etype();
        Key::ALL
            .into_iter()
            .filter(|key| match key {
                Key::Path | Key::Format | Key::Rows => true,
                Key::Separator => etype.uses_separator(),
                Key::Custom => etype.uses_separator() && self.choice(Key::Separator) == OTHER,
                Key::LineEnds => etype == ExportType::Csv,
                Key::Header => etype.has_header(),
                Key::NullAs => etype.uses_null_as(),
                Key::Table => etype == ExportType::SqlInsert,
                Key::Encoding => etype.is_text(),
            })
            .collect()
    }

    /// Keeps what the shown rows hold, then shows the rows the format
    /// wants, the focus staying on the same row where it can.
    fn relayout(&mut self) {
        for (key, field) in self.keys.iter().zip(&self.fields) {
            self.store[*key as usize] = field.clone();
        }
        let focused = self.keys.get(self.focus).copied();
        self.keys = self.wanted();
        self.labels = self.keys.iter().map(|key| key.label()).collect();
        self.fields = self
            .keys
            .iter()
            .map(|key| self.store[*key as usize].clone())
            .collect();
        self.focus = focused
            .and_then(|key| self.keys.iter().position(|k| *k == key))
            .unwrap_or(self.focus)
            .min(self.keys.len() - 1);
    }

    /// The path and format to look up, when they are not what was asked
    /// last. The reducer sends them to the runtime, and remembers they
    /// were asked.
    pub(crate) fn probe_wanted(&self) -> Option<(String, ExportType)> {
        let wanted = (self.path().to_string(), self.etype());
        (!wanted.0.is_empty() && self.asked.as_ref() != Some(&wanted)).then_some(wanted)
    }

    pub(crate) fn probe_asked(&mut self, input: String, etype: ExportType) {
        self.asked = Some((input, etype));
    }

    /// What the runtime found; kept only while it is about the path and
    /// format shown.
    pub(crate) fn probed(&mut self, probe: ExportProbe) {
        if probe.input == self.path() && probe.etype == self.etype() {
            self.probe = Some(probe);
        }
    }

    /// What is known of the file the dialog would write.
    pub(crate) fn current_probe(&self) -> Option<&ExportProbe> {
        self.probe
            .as_ref()
            .filter(|p| p.input == self.path() && p.etype == self.etype())
    }

    /// Running the statement again might change data, and nobody has said
    /// it may.
    fn rerun_warns(&self) -> bool {
        self.scope() == Scope::All && !self.subject.read_only
    }

    fn options(&self) -> Result<ExportOptions, String> {
        let etype = self.etype();
        let separator = if etype == ExportType::Tsv {
            '\t'
        } else {
            match self.choice(Key::Separator) {
                chosen if chosen < SEPARATOR_VALUES.len() => SEPARATOR_VALUES[chosen],
                // Not trimmed: a space is a separator someone may want.
                _ if etype.uses_separator() => parse_separator(self.value(Key::Custom))?,
                _ => ',',
            }
        };
        let table = self.value(Key::Table).trim();
        Ok(ExportOptions {
            separator,
            header: self.choice(Key::Header) == 0,
            null_as: self.value(Key::NullAs).to_string(),
            crlf: self.choice(Key::LineEnds) == 1,
            table: (!table.is_empty()).then(|| table.to_string()),
            dialect: self.subject.dialect.unwrap_or(Dialect::Postgres),
            batch: 100,
            bom: self.choice(Key::Encoding) == 1,
        })
    }
}

/// A text field holding `value`, the cursor at its end.
fn text(value: String) -> Field {
    Field::Text(TextInput {
        cursor: value.chars().count(),
        value,
        placeholder: None,
    })
}

impl Form for ExportForm {
    fn title(&self) -> &'static str {
        "Export"
    }

    fn footer(&self) -> &'static [(Action, &'static str)] {
        &[
            (Action::Submit, "export"),
            (Action::NextField, "next"),
            (Action::Right, "change"),
            (Action::Cancel, "cancel"),
        ]
    }

    fn width(&self) -> u16 {
        80
    }

    fn accent(&self) -> Accent {
        Accent::Act
    }

    /// `result 2 orders · 7 rows (all fetched)`, or `(1 000 of more)` when
    /// the result was cut at the row limit.
    fn subtitle(&self) -> Option<String> {
        let subject = &self.subject;
        let name = subject.table.as_deref().unwrap_or("result");
        let rows = format_rows(subject.rows as u64);
        let fetched = if subject.truncated {
            format!("({rows} of more)")
        } else {
            "(all fetched)".to_string()
        };
        let noun = if subject.rows == 1 { "row" } else { "rows" };
        Some(format!(
            "result {} {name} · {rows} {noun} {fetched}",
            subject.tab
        ))
    }

    fn subtitle_warns(&self) -> bool {
        self.subject.truncated
    }

    fn labels(&self) -> &[&'static str] {
        &self.labels
    }

    fn fields(&self) -> &[Field] {
        &self.fields
    }

    fn fields_mut(&mut self) -> &mut [Field] {
        &mut self.fields
    }

    fn focus(&self) -> usize {
        self.focus
    }

    fn set_focus(&mut self, focus: usize) {
        self.focus = focus;
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn set_error(&mut self, error: Option<String>) {
        self.error = error;
    }

    fn pending(&self) -> Option<&'static str> {
        self.pending
    }

    fn set_pending(&mut self, pending: Option<&'static str>) {
        self.pending = pending;
    }

    fn field_error(&self, field: usize) -> Option<&str> {
        (self.keys.get(field) == Some(&Key::Rows) && self.rerun_warns())
            .then_some("re-running may change data")
    }

    /// `→ /home/ann/exports/orders.csv · exists, will ask before
    /// overwriting`: the file the path names, once the runtime has looked.
    fn note(&self) -> Option<Note> {
        let Some(probe) = self.current_probe() else {
            let text = if self.path().is_empty() {
                "→ type where the file goes"
            } else {
                "→ …"
            };
            return Some(Note::new(Tone::Dim, text));
        };
        let note = Note::new(Tone::Plain, format!("→ {}", probe.resolved));
        Some(if probe.is_dir {
            note.then(Tone::Error, " · is a folder")
        } else if probe.exists {
            note.then(Tone::Dim, " · exists, will ask before overwriting")
        } else {
            note
        })
    }

    fn after_change(&mut self) {
        // A new path or format is a new file: asking again is right.
        let before = (self.path().to_string(), self.etype());
        self.relayout();
        if (self.path().to_string(), self.etype()) != before {
            self.overwrite = false;
        }
        if self.scope() == Scope::Fetched {
            self.rerun_confirmed = false;
        }
    }

    fn submit(&self) -> Result<Submission, String> {
        let path = self.path();
        if path.is_empty() {
            return Err("Path is required".into());
        }
        Ok(Submission::Export(Box::new(ExportChoice {
            path: path.to_string(),
            etype: self.etype(),
            options: self.options()?,
            scope: self.scope(),
        })))
    }
}

#[cfg(test)]
#[path = "exportform_tests.rs"]
mod exportform_tests;
