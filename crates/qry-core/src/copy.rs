//! Writing part of a result as text to copy: CSV, TSV, JSON, Markdown or
//! SQL `INSERT`s. The export (T307) reuses these writers.

use crate::{QueryResult, sql::Dialect};

/// The text format of a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFormat {
    Csv,
    Tsv,
    Json,
    Markdown,
    Insert,
}

impl CopyFormat {
    pub const ALL: [CopyFormat; 5] = [
        CopyFormat::Csv,
        CopyFormat::Tsv,
        CopyFormat::Json,
        CopyFormat::Markdown,
        CopyFormat::Insert,
    ];

    pub fn name(self) -> &'static str {
        match self {
            CopyFormat::Csv => "CSV",
            CopyFormat::Tsv => "TSV",
            CopyFormat::Json => "JSON",
            CopyFormat::Markdown => "Markdown",
            CopyFormat::Insert => "SQL INSERT",
        }
    }
}

/// Which rows and columns, in order, and whether a header line goes first
/// (CSV, TSV and Markdown; JSON keys and INSERT columns always name them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub rows: Vec<usize>,
    pub cols: Vec<usize>,
    pub header: bool,
}

impl Selection {
    pub fn cell(row: usize, col: usize) -> Self {
        Self {
            rows: vec![row],
            cols: vec![col],
            header: false,
        }
    }

    pub fn row(result: &QueryResult, row: usize) -> Self {
        Self {
            rows: vec![row],
            cols: (0..result.columns.len()).collect(),
            header: false,
        }
    }

    /// One column, in the order of `rows`.
    pub fn column(rows: Vec<usize>, col: usize) -> Self {
        Self {
            rows,
            cols: vec![col],
            header: true,
        }
    }

    /// Everything, in the order of `rows`.
    pub fn all(result: &QueryResult, rows: Vec<usize>) -> Self {
        Self {
            rows,
            cols: (0..result.columns.len()).collect(),
            header: true,
        }
    }
}

fn cell(result: &QueryResult, row: usize, col: usize) -> Option<&str> {
    result.rows.get(row)?.get(col)?.as_deref()
}

fn csv_field(text: &str, separator: char) -> String {
    if text.contains([separator, '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn tsv_field(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn json_string(text: &str) -> String {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn markdown_field(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', "<br>")
}

/// `name` as an identifier in `dialect`, quoted only if it needs it.
pub fn quote_ident(dialect: Dialect, name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(
            crate::sql::lex(dialect, name).first().map(|t| t.kind),
            Some(crate::sql::TokenKind::Keyword)
        );
    if plain {
        return name.to_string();
    }
    match dialect {
        Dialect::MySql => format!("`{}`", name.replace('`', "``")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// A value as a literal: `NULL`, a number as written, else a quoted string.
fn literal(dialect: Dialect, value: Option<&str>, numeric: bool) -> String {
    match value {
        None => "NULL".into(),
        Some(v) if numeric && v.trim().parse::<f64>().is_ok() => v.trim().to_string(),
        Some(v) => {
            let mut quoted = v.replace('\'', "''");
            if dialect == Dialect::MySql {
                quoted = quoted.replace('\\', "\\\\");
            }
            format!("'{quoted}'")
        }
    }
}

/// The text for `selection` of `result`. `table` is the table INSERTs go
/// into.
pub fn render(
    result: &QueryResult,
    selection: &Selection,
    format: CopyFormat,
    dialect: Dialect,
    table: &str,
) -> String {
    let names: Vec<&str> = selection
        .cols
        .iter()
        .map(|&c| result.columns.get(c).map_or("", |col| col.name.as_str()))
        .collect();
    match format {
        CopyFormat::Csv | CopyFormat::Tsv => {
            let field = |t: &str| {
                if format == CopyFormat::Csv {
                    csv_field(t, ',')
                } else {
                    tsv_field(t)
                }
            };
            let sep = if format == CopyFormat::Csv { "," } else { "\t" };
            let mut lines = Vec::new();
            if selection.header {
                lines.push(names.iter().map(|n| field(n)).collect::<Vec<_>>().join(sep));
            }
            for &r in &selection.rows {
                lines.push(
                    selection
                        .cols
                        .iter()
                        .map(|&c| cell(result, r, c).map_or(String::new(), field))
                        .collect::<Vec<_>>()
                        .join(sep),
                );
            }
            lines.join("\n")
        }
        CopyFormat::Json => {
            let objects: Vec<String> = selection
                .rows
                .iter()
                .map(|&r| {
                    let fields: Vec<String> = selection
                        .cols
                        .iter()
                        .zip(&names)
                        .map(|(&c, name)| {
                            let value = cell(result, r, c).map_or("null".into(), json_string);
                            format!("{}: {value}", json_string(name))
                        })
                        .collect();
                    format!("{{{}}}", fields.join(", "))
                })
                .collect();
            format!("[{}]", objects.join(",\n "))
        }
        CopyFormat::Markdown => {
            let mut lines = Vec::new();
            let header: Vec<String> = names.iter().map(|n| markdown_field(n)).collect();
            lines.push(format!("| {} |", header.join(" | ")));
            lines.push(format!("|{}|", vec!["---"; names.len()].join("|")));
            for &r in &selection.rows {
                let cells: Vec<String> = selection
                    .cols
                    .iter()
                    .map(|&c| markdown_field(cell(result, r, c).unwrap_or("NULL")))
                    .collect();
                lines.push(format!("| {} |", cells.join(" | ")));
            }
            lines.join("\n")
        }
        CopyFormat::Insert => {
            let columns: Vec<String> = names.iter().map(|n| quote_ident(dialect, n)).collect();
            let lines: Vec<String> = selection
                .rows
                .iter()
                .map(|&r| {
                    let values: Vec<String> = selection
                        .cols
                        .iter()
                        .map(|&c| {
                            let numeric = result
                                .columns
                                .get(c)
                                .is_some_and(|col| col.kind.is_numeric());
                            literal(dialect, cell(result, r, c), numeric)
                        })
                        .collect();
                    format!(
                        "INSERT INTO {} ({}) VALUES ({});",
                        quote_ident(dialect, table),
                        columns.join(", "),
                        values.join(", ")
                    )
                })
                .collect();
            lines.join("\n")
        }
    }
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod copy_tests;
