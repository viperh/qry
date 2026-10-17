//! Writes a query result to a file, a batch of rows at a time.
//!
//! [`sink`] opens a [`RowSink`] for one of the [`ExportType`]s: it is told
//! the columns, then handed rows as they come, then finished. Nothing is
//! written over the target until the end: the rows go to a temporary file
//! beside it, renamed into place by [`RowSink::finish`], so a failed or
//! abandoned export leaves any file already there as it was, and no
//! temporary file behind.

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow, bail};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, Worksheet};

use crate::{
    ColumnMeta, ExportConfig, ExportType, QueryResult, ValueKind,
    copy::{csv_field, json_string, literal, markdown_field, quote_ident, tsv_field},
    shared::StreamItem,
    sql::Dialect,
};

/// The most data rows a worksheet holds: 1 048 576 rows, one of them the
/// header.
pub const EXCEL_MAX_ROWS: usize = 1_048_575;

/// The longest text an Excel cell holds.
const EXCEL_MAX_TEXT: usize = 32_767;

/// How a file is written. Each format reads the options that make sense
/// for it (see [`ExportType::uses_separator`], [`ExportType::has_header`],
/// [`ExportType::uses_null_as`]) and ignores the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    /// Between the values of CSV and Text.
    pub separator: char,
    /// Whether the column names go first.
    pub header: bool,
    /// What a NULL is written as, where the format has no NULL of its own.
    pub null_as: String,
    /// Lines end with `\r\n` rather than `\n`.
    pub crlf: bool,
    /// The table SQL `INSERT`s go into; `result` when unset.
    pub table: Option<String>,
    /// How SQL `INSERT`s quote names and strings.
    pub dialect: Dialect,
    /// Rows per SQL `INSERT` statement.
    pub batch: usize,
    /// A text file starts with a UTF-8 byte-order mark, which some
    /// spreadsheet programs want before they read UTF-8.
    pub bom: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            separator: ',',
            header: true,
            null_as: String::new(),
            crlf: false,
            table: None,
            dialect: Dialect::Postgres,
            batch: 100,
            bom: false,
        }
    }
}

impl ExportOptions {
    /// What an [`ExportConfig`] asks for: its separator's first character
    /// (a tab for Text without one), and the defaults for the rest.
    pub fn from_config(config: &ExportConfig) -> Self {
        let fallback = if config.etype == ExportType::Text {
            '\t'
        } else {
            ','
        };
        Self {
            separator: config.separator.chars().next().unwrap_or(fallback),
            ..Self::default()
        }
    }
}

/// Reads a separator typed by hand: exactly one character, and not a quote
/// or a line break, which would make the rows unreadable.
pub fn parse_separator(text: &str) -> Result<char, String> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (None, _) => Err("Custom separator is required".into()),
        (Some(_), Some(_)) => Err("A separator is a single character".into()),
        (Some('"' | '\n' | '\r'), None) => {
            Err("A separator cannot be a quote or a line break".into())
        }
        (Some(c), None) => Ok(c),
    }
}

/// Where the rows of an export go, as they arrive.
pub trait RowSink: Send {
    /// The columns, once, before any row.
    fn begin(&mut self, cols: &[ColumnMeta]) -> Result<()>;
    /// The next rows, in order.
    fn rows(&mut self, rows: &[Vec<Option<String>>]) -> Result<()>;
    /// Something to tell the user about the file, though it was written.
    fn warning(&self) -> Option<String> {
        None
    }
    /// Writes what is left and puts the file in place; returns the rows
    /// written. Dropping a sink without finishing it leaves the target as
    /// it was.
    fn finish(self: Box<Self>) -> Result<u64>;
}

/// Opens a sink writing `etype` to `path`. The folder must exist and the
/// path must not be a folder; both are checked now, before any row.
pub fn sink(etype: ExportType, path: &Path, opts: &ExportOptions) -> Result<Box<dyn RowSink>> {
    let file = AtomicFile::create(path)?;
    Ok(match etype {
        ExportType::Excel => Box::new(ExcelSink::new(file, opts, EXCEL_MAX_ROWS)),
        etype => Box::new(TextSink::new(file, etype, opts)),
    })
}

/// What [`write_result`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub rows: u64,
    pub warning: Option<String>,
}

/// Writes all of `result` to `path` as `etype`. A result too big for the
/// format is refused before anything is written.
pub fn write_result(
    result: &QueryResult,
    etype: ExportType,
    path: &Path,
    opts: &ExportOptions,
) -> Result<Written> {
    if let Some(limit) = etype.row_limit()
        && result.rows.len() > limit
    {
        bail!(too_many_rows(result.rows.len(), limit));
    }
    let written = (|| {
        let mut sink = sink(etype, path, opts)?;
        sink.begin(&result.columns)?;
        sink.rows(&result.rows)?;
        let warning = sink.warning();
        let rows = sink.finish()?;
        Ok::<_, anyhow::Error>(Written { rows, warning })
    })();
    written.with_context(|| format!("could not write {}", path.display()))
}

/// Runs `sql` on `driver` with no row limit and streams every row into a
/// sink writing `etype` to `path`: the query hands on `fetch_size` rows at
/// a time to a writer on a thread of its own, through a channel of one, so
/// no more than a few batches are ever held. `progress` hears the rows
/// written so far after each batch.
///
/// The file is put in place only if the query and the writing both
/// succeed. A failed or cancelled query, or this future dropped part-way
/// (its session closed), leaves the target as it was and no temporary file.
pub async fn export_query(
    driver: &crate::Driver,
    sql: &str,
    fetch_size: usize,
    etype: ExportType,
    path: &Path,
    opts: &ExportOptions,
    progress: impl Fn(u64) + Send + 'static,
) -> Result<Written> {
    let sink =
        sink(etype, path, opts).with_context(|| format!("could not write {}", path.display()))?;
    let (rows_tx, mut rows_rx) = tokio::sync::mpsc::channel::<StreamItem>(1);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<bool>();
    // Dropping this future (a cancelled job, a closed session) stops the
    // writer at its next batch, and with its receiver gone a driver that
    // reads on a thread of its own (Oracle) stops reading too.
    // Kept, not read, until the end of this future.
    let _abandoned = Abandoned::default();
    let given_up = _abandoned.0.clone();
    // `None` when the query did not finish: the sink goes unfinished.
    let writer = tokio::task::spawn_blocking(move || -> Result<Option<Written>> {
        let mut sink = sink;
        let (mut begun, mut rows) = (false, 0u64);
        while let Some(item) = rows_rx.blocking_recv() {
            if given_up.load(Ordering::SeqCst) {
                return Ok(None);
            }
            match item {
                StreamItem::Columns(columns) => {
                    sink.begin(&columns)?;
                    begun = true;
                }
                StreamItem::Rows(batch) => {
                    sink.rows(&batch)?;
                    rows += batch.len() as u64;
                    progress(rows);
                }
            }
        }
        if done_rx.blocking_recv() != Ok(true) {
            return Ok(None);
        }
        if !begun {
            sink.begin(&[])?;
        }
        let warning = sink.warning();
        let rows = sink.finish()?;
        Ok(Some(Written { rows, warning }))
    });
    let streamed = driver.query_stream(sql, fetch_size, rows_tx).await;
    let _ = done_tx.send(streamed.is_ok());
    let written = writer
        .await
        .map_err(|e| anyhow!("the export's writer stopped: {e}"))?
        .with_context(|| format!("could not write {}", path.display()));
    match (streamed, written) {
        (_, Ok(Some(written))) => Ok(written),
        // The writer's own failure is what stopped the query.
        (_, Err(e)) => Err(e),
        (Err(e), Ok(None)) => Err(e),
        (Ok(_), Ok(None)) => bail!("the export stopped before its end"),
    }
}

/// Set when the export that owns it is dropped.
#[derive(Default)]
struct Abandoned(Arc<AtomicBool>);

impl Drop for Abandoned {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn too_many_rows(rows: usize, limit: usize) -> String {
    format!(
        "{} rows do not fit in an Excel worksheet, which holds {}",
        crate::jobs::format_rows(rows as u64),
        crate::jobs::format_rows(limit as u64)
    )
}

/// The file `input` names: `~` and `~/` are the home directory (not
/// `~user`, nor anything on Windows), and the type's extension is added
/// unless it is already there, in any case.
pub fn resolve_path(input: &str, etype: ExportType, home: &Path) -> PathBuf {
    let path = etype.with_extension(input.trim());
    if let Some(rest) = path.strip_prefix("~/") {
        // One component at a time, so `~/a/b` takes the platform's
        // separator throughout.
        return rest
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(home.to_path_buf(), |path, part| path.join(part));
    }
    PathBuf::from(path)
}

/// `path` as it reads best: under the home directory, from `~/`.
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

/// A file written beside its target under a name of its own, and renamed
/// over the target only once complete. Dropped before that, it removes
/// itself.
struct AtomicFile {
    target: PathBuf,
    temp: PathBuf,
    out: Option<BufWriter<File>>,
}

/// Tells this process's temporary files apart.
static TEMPS: AtomicU64 = AtomicU64::new(0);

impl AtomicFile {
    fn create(target: &Path) -> Result<Self> {
        if target.is_dir() {
            bail!("{} is a folder", target.display());
        }
        let dir = match target.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
        if !dir.is_dir() {
            bail!("the folder {} does not exist", dir.display());
        }
        let name = target
            .file_name()
            .ok_or_else(|| anyhow!("{} names no file", target.display()))?
            .to_string_lossy()
            .into_owned();
        loop {
            let n = TEMPS.fetch_add(1, Ordering::Relaxed);
            let temp = dir.join(format!(".{name}.{}-{n}.part", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&temp) {
                Ok(file) => {
                    return Ok(Self {
                        target: target.to_path_buf(),
                        temp,
                        out: Some(BufWriter::new(file)),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    return Err(e).with_context(|| format!("cannot write in {}", dir.display()));
                }
            }
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        match &mut self.out {
            Some(out) => Ok(out.write_all(bytes)?),
            None => bail!("the file is already closed"),
        }
    }

    /// Flushes the file to disk and moves it over the target.
    fn commit(mut self) -> Result<()> {
        let out = self
            .out
            .take()
            .ok_or_else(|| anyhow!("the file is already closed"))?;
        let file = out.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.temp, &self.target)?;
        // Renamed: there is no temporary file left to remove.
        self.temp = PathBuf::new();
        Ok(())
    }
}

impl Drop for AtomicFile {
    fn drop(&mut self) {
        drop(self.out.take());
        if !self.temp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

/// The text formats: delimited, JSON, Markdown and SQL.
struct TextSink {
    file: AtomicFile,
    etype: ExportType,
    opts: ExportOptions,
    columns: Vec<ColumnMeta>,
    rows: u64,
    eol: &'static str,
    /// The rows of the INSERT not written yet, as `(…)` tuples.
    pending: Vec<String>,
}

impl TextSink {
    fn new(file: AtomicFile, etype: ExportType, opts: &ExportOptions) -> Self {
        Self {
            file,
            etype,
            opts: opts.clone(),
            columns: Vec::new(),
            rows: 0,
            eol: if opts.crlf { "\r\n" } else { "\n" },
            pending: Vec::new(),
        }
    }

    fn line(&mut self, text: &str) -> Result<()> {
        self.file.write_all(text.as_bytes())?;
        self.file.write_all(self.eol.as_bytes())
    }

    fn separator(&self) -> String {
        match self.etype {
            ExportType::Tsv => "\t".into(),
            _ => self.opts.separator.to_string(),
        }
    }

    /// One value as the delimited formats write it.
    fn field(&self, value: Option<&str>) -> String {
        let Some(value) = value else {
            return match self.etype {
                ExportType::Csv => csv_field(&self.opts.null_as, self.opts.separator),
                ExportType::Tsv => tsv_field(&self.opts.null_as),
                ExportType::Markdown => markdown_field(&self.opts.null_as),
                _ => self.opts.null_as.clone(),
            };
        };
        match self.etype {
            // An empty string is quoted, so it reads apart from a NULL
            // written as nothing, and a row of one never reads as a blank
            // line.
            ExportType::Csv if value.is_empty() => "\"\"".into(),
            ExportType::Csv => csv_field(value, self.opts.separator),
            ExportType::Tsv => tsv_field(value),
            ExportType::Markdown => markdown_field(value),
            // A tab or newline inside a value would break the columns.
            _ => value.replace(['\t', '\r', '\n'], " "),
        }
    }

    /// A row as a JSON object, numbers and booleans unquoted where the
    /// column says they are.
    fn object(&self, row: &[Option<String>]) -> String {
        let fields: Vec<String> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                let value = row.get(i).and_then(Option::as_deref);
                format!(
                    "{}: {}",
                    json_string(&column.name),
                    json_value(value, column.kind)
                )
            })
            .collect();
        format!("{{{}}}", fields.join(", "))
    }

    fn table(&self) -> String {
        let table = self.opts.table.as_deref().unwrap_or("result");
        table
            .split('.')
            .map(|part| quote_ident(self.opts.dialect, part))
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Writes the waiting rows as one INSERT.
    fn flush_insert(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let table = self.table();
        let columns = if self.opts.header {
            let names: Vec<String> = self
                .columns
                .iter()
                .map(|c| quote_ident(self.opts.dialect, &c.name))
                .collect();
            format!(" ({})", names.join(", "))
        } else {
            String::new()
        };
        let pending = std::mem::take(&mut self.pending);
        let eol = self.eol;
        let text = if self.opts.dialect == Dialect::Oracle {
            // Oracle has no multi-row VALUES.
            let intos: Vec<String> = pending
                .iter()
                .map(|values| format!("  INTO {table}{columns} VALUES {values}"))
                .collect();
            format!("INSERT ALL{eol}{}{eol}SELECT 1 FROM DUAL;", intos.join(eol))
        } else {
            format!(
                "INSERT INTO {table}{columns} VALUES{eol}  {};",
                pending.join(&format!(",{eol}  "))
            )
        };
        self.line(&text)
    }
}

/// A value in JSON: `null`, a number or boolean as it is when the column
/// holds those and the text is one, else a string.
fn json_value(value: Option<&str>, kind: ValueKind) -> String {
    match value {
        None => "null".into(),
        Some(v) if kind.is_numeric() && is_json_number(v) => v.to_string(),
        Some(v @ ("true" | "false")) if kind == ValueKind::Bool => v.to_string(),
        Some(v) => json_string(v),
    }
}

/// Whether `text` is a number as JSON writes one: `-0.5e3` yes, `+1`,
/// `.5`, `01` and `NaN` no.
fn is_json_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (int, rest) = digits
        .find(['.', 'e', 'E'])
        .map_or((digits, ""), |i| digits.split_at(i));
    let int_ok = int == "0"
        || (!int.is_empty() && !int.starts_with('0') && int.bytes().all(|b| b.is_ascii_digit()));
    if !int_ok {
        return false;
    }
    let (frac, exp) = match rest.strip_prefix('.') {
        Some(after) => after
            .find(['e', 'E'])
            .map_or((Some(after), ""), |i| (Some(&after[..i]), &after[i..])),
        None => (None, rest),
    };
    if frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    match exp.strip_prefix(['e', 'E']) {
        None => exp.is_empty(),
        Some(e) => {
            let e = e.strip_prefix(['+', '-']).unwrap_or(e);
            !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

impl RowSink for TextSink {
    fn begin(&mut self, cols: &[ColumnMeta]) -> Result<()> {
        self.columns = cols.to_vec();
        if self.opts.bom {
            self.file.write_all("\u{feff}".as_bytes())?;
        }
        match self.etype {
            ExportType::Csv | ExportType::Tsv | ExportType::Text if self.opts.header => {
                let names: Vec<String> = cols.iter().map(|c| self.field(Some(&c.name))).collect();
                let line = names.join(&self.separator());
                self.line(&line)
            }
            ExportType::Markdown => {
                let names: Vec<String> = cols
                    .iter()
                    .map(|c| match self.opts.header {
                        true => markdown_field(&c.name),
                        false => String::new(),
                    })
                    .collect();
                self.line(&format!("| {} |", names.join(" | ")))?;
                self.line(&format!("|{}|", vec!["---"; cols.len()].join("|")))
            }
            ExportType::Json => self.line("["),
            _ => Ok(()),
        }
    }

    fn rows(&mut self, rows: &[Vec<Option<String>>]) -> Result<()> {
        let separator = self.separator();
        for row in rows {
            let width = self.columns.len().max(row.len());
            let cell = |i: usize| row.get(i).and_then(Option::as_deref);
            match self.etype {
                ExportType::Json => {
                    let lead = if self.rows == 0 { "" } else { "," };
                    if !lead.is_empty() {
                        self.line(lead)?;
                    }
                    let object = self.object(row);
                    self.file.write_all(format!("  {object}").as_bytes())?;
                }
                ExportType::JsonLines => {
                    let object = self.object(row);
                    self.line(&object)?;
                }
                ExportType::Markdown => {
                    let cells: Vec<String> = (0..width).map(|i| self.field(cell(i))).collect();
                    self.line(&format!("| {} |", cells.join(" | ")))?;
                }
                ExportType::SqlInsert => {
                    let dialect = self.opts.dialect;
                    let values: Vec<String> = (0..width)
                        .map(|i| {
                            let numeric = self.columns.get(i).is_some_and(|c| c.kind.is_numeric());
                            literal(dialect, cell(i), numeric)
                        })
                        .collect();
                    self.pending.push(format!("({})", values.join(", ")));
                    if self.pending.len() >= self.opts.batch.max(1) {
                        self.flush_insert()?;
                    }
                }
                ExportType::Csv | ExportType::Tsv | ExportType::Text | ExportType::Excel => {
                    let fields: Vec<String> = (0..width).map(|i| self.field(cell(i))).collect();
                    self.line(&fields.join(&separator))?;
                }
            }
            self.rows += 1;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<u64> {
        match self.etype {
            // `[` and `]` on lines of their own, the objects between.
            ExportType::Json if self.rows > 0 => {
                self.line("")?;
                self.line("]")?;
            }
            ExportType::Json => self.line("]")?,
            ExportType::SqlInsert => self.flush_insert()?,
            _ => {}
        }
        let rows = self.rows;
        self.file.commit()?;
        Ok(rows)
    }
}

/// A workbook of one sheet, built in memory (as `rust_xlsxwriter` does)
/// and written out when finished.
struct ExcelSink {
    file: AtomicFile,
    sheet: Worksheet,
    opts: ExportOptions,
    columns: Vec<ColumnMeta>,
    /// The sheet row the next data row goes on.
    next: u32,
    /// Data rows so far, and the most the sheet may take.
    rows: u64,
    limit: usize,
    /// The widest value seen in each column, in characters.
    widths: Vec<usize>,
    /// Characters Excel cannot hold were taken out of some value.
    stripped: bool,
    bold: Format,
    date: Format,
    time: Format,
    stamp: Format,
}

/// Values measured for the column widths; the rest are not looked at.
const WIDTH_SAMPLE: u64 = 1000;
/// The widest a column is made.
const MAX_WIDTH: usize = 60;

impl ExcelSink {
    fn new(file: AtomicFile, opts: &ExportOptions, limit: usize) -> Self {
        Self {
            file,
            sheet: Worksheet::new(),
            opts: opts.clone(),
            columns: Vec::new(),
            next: 0,
            rows: 0,
            limit,
            widths: Vec::new(),
            stripped: false,
            bold: Format::new().set_bold(),
            date: Format::new().set_num_format("yyyy-mm-dd"),
            time: Format::new().set_num_format("hh:mm:ss"),
            stamp: Format::new().set_num_format("yyyy-mm-dd hh:mm:ss"),
        }
    }

    /// `text` without the control characters Excel refuses (all below
    /// 0x20 but tab and newline), and no longer than a cell holds.
    fn clean(&mut self, text: &str) -> String {
        let mut out: String = text
            .chars()
            .filter(|&c| c >= ' ' || c == '\t' || c == '\n')
            .collect();
        if out.chars().count() > EXCEL_MAX_TEXT {
            out = out.chars().take(EXCEL_MAX_TEXT).collect();
        }
        if out.len() != text.len() {
            self.stripped = true;
        }
        out
    }

    fn measure(&mut self, col: usize, text: &str) {
        if self.rows < WIDTH_SAMPLE
            && let Some(width) = self.widths.get_mut(col)
        {
            *width = (*width).max(text.chars().count().min(MAX_WIDTH));
        }
    }

    fn cell(&mut self, row: u32, col: usize, value: Option<&str>) -> Result<()> {
        let c = u16::try_from(col).map_err(|_| anyhow!("too many columns for Excel"))?;
        let kind = self.columns.get(col).map_or(ValueKind::Other, |m| m.kind);
        let Some(value) = value else {
            if !self.opts.null_as.is_empty() {
                let text = self.clean(&self.opts.null_as.clone());
                self.sheet.write_string(row, c, text)?;
            }
            return Ok(());
        };
        self.measure(col, value);
        let trimmed = value.trim();
        match kind {
            ValueKind::Integer | ValueKind::Decimal | ValueKind::Float => {
                if let Ok(n) = trimmed.parse::<f64>()
                    && n.is_finite()
                {
                    self.sheet.write_number(row, c, n)?;
                    return Ok(());
                }
            }
            ValueKind::Bool => {
                let truth = match trimmed.to_ascii_lowercase().as_str() {
                    "true" | "t" | "1" => Some(true),
                    "false" | "f" | "0" => Some(false),
                    _ => None,
                };
                if let Some(truth) = truth {
                    self.sheet.write_boolean(row, c, truth)?;
                    return Ok(());
                }
            }
            ValueKind::Date | ValueKind::Time | ValueKind::Timestamp => {
                if let Ok(when) = ExcelDateTime::parse_from_str(trimmed) {
                    let format = match kind {
                        ValueKind::Date => &self.date,
                        ValueKind::Time => &self.time,
                        _ => &self.stamp,
                    };
                    self.sheet
                        .write_datetime_with_format(row, c, &when, format)?;
                    return Ok(());
                }
            }
            _ => {}
        }
        let text = self.clean(value);
        self.sheet.write_string(row, c, text)?;
        Ok(())
    }
}

impl RowSink for ExcelSink {
    fn begin(&mut self, cols: &[ColumnMeta]) -> Result<()> {
        self.columns = cols.to_vec();
        self.widths = cols.iter().map(|c| c.name.chars().count()).collect();
        if self.opts.header {
            for (i, column) in cols.iter().enumerate() {
                let c = u16::try_from(i).map_err(|_| anyhow!("too many columns for Excel"))?;
                let name = self.clean(&column.name);
                self.sheet
                    .write_string_with_format(0, c, name, &self.bold.clone())?;
            }
            self.sheet.set_freeze_panes(1, 0)?;
            self.next = 1;
        }
        Ok(())
    }

    fn rows(&mut self, rows: &[Vec<Option<String>>]) -> Result<()> {
        if self.rows as usize + rows.len() > self.limit {
            bail!(too_many_rows(self.rows as usize + rows.len(), self.limit));
        }
        for row in rows {
            let r = self.next;
            for (col, value) in row.iter().enumerate() {
                self.cell(r, col, value.as_deref())?;
            }
            self.next += 1;
            self.rows += 1;
        }
        Ok(())
    }

    fn warning(&self) -> Option<String> {
        self.stripped
            .then(|| "characters Excel cannot hold were removed".to_string())
    }

    fn finish(mut self: Box<Self>) -> Result<u64> {
        let last_col = u16::try_from(self.columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
        if self.opts.header && !self.columns.is_empty() {
            let last_row = self.next.saturating_sub(1);
            self.sheet.autofilter(0, 0, last_row, last_col)?;
        }
        for (i, width) in self.widths.iter().enumerate() {
            let c = u16::try_from(i).unwrap_or(u16::MAX);
            // Room for the filter button beside the name.
            self.sheet
                .set_column_width(c, (*width + 3).min(MAX_WIDTH) as f64)?;
        }
        let mut workbook = Workbook::new();
        workbook.push_worksheet(std::mem::replace(&mut self.sheet, Worksheet::new()));
        let bytes = workbook.save_to_buffer()?;
        self.file.write_all(&bytes)?;
        let rows = self.rows;
        let Self { file, .. } = *self;
        file.commit()?;
        Ok(rows)
    }
}

/// An Excel sink holding at most `limit` rows, for the tests of the limit.
#[cfg(test)]
pub(crate) fn excel_sink_with_limit(path: &Path, limit: usize) -> Result<Box<dyn RowSink>> {
    let file = AtomicFile::create(path)?;
    Ok(Box::new(ExcelSink::new(
        file,
        &ExportOptions::default(),
        limit,
    )))
}

#[cfg(test)]
#[path = "exporter_tests.rs"]
mod exporter_tests;
