//! Writes a query result to a file.

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::{Context, Result};
use rust_xlsxwriter::Workbook;

use crate::{ExportConfig, ExportType, QueryResult};

/// Rows ready to be written, the first of them the header. A `None` is a
/// NULL: the text formats write it as nothing, JSON writes it as `null`.
#[derive(Debug, Default)]
pub struct Exporter {
    data: Vec<Vec<Option<String>>>,
    separator: String,
    path: String,
}

impl Exporter {
    /// Takes the column names as the header row, then every row.
    pub fn from_result(result: &QueryResult, config: &ExportConfig) -> Self {
        let mut data = Vec::with_capacity(result.rows.len() + 1);
        data.push(result.columns.iter().cloned().map(Some).collect());
        data.extend(result.rows.iter().cloned());

        let mut exporter = Self { data, ..Self::default() };
        exporter.set_path(config.path.clone());
        exporter.set_separator(config.separator.clone());
        exporter
    }

    pub fn set_data(&mut self, data: Vec<Vec<String>>) {
        self.data = data.into_iter().map(|row| row.into_iter().map(Some).collect()).collect();
    }
    pub fn set_path(&mut self, path: String) {
        self.path = path;
    }

    pub fn set_separator(&mut self, separator: String) {
        self.separator = separator;
    }

    /// Writes the file, and answers with the number of rows written, not
    /// counting the header.
    pub fn write(&self, etype: ExportType) -> Result<usize> {
        match etype {
            ExportType::Csv => self.to_csv(),
            ExportType::Text => self.to_plain(),
            ExportType::Excel => self.to_excel(),
            ExportType::Json => self.to_json(),
        }
        .with_context(|| format!("could not write {}", self.path))?;
        Ok(self.data.len().saturating_sub(1))
    }

    pub fn to_csv(&self) -> Result<()> {
        self.write_delimited(|field| quote_csv(field, &self.separator))
    }

    pub fn to_plain(&self) -> Result<()> {
        // A tab or newline inside a value would break the columns.
        self.write_delimited(|field| field.replace(['\t', '\r', '\n'], " "))
    }

    pub fn to_excel(&self) -> Result<()> {
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        for (r, row) in self.data.iter().enumerate() {
            for (c, field) in row.iter().enumerate() {
                let field = field.as_deref().unwrap_or_default();
                sheet.write_string(u32::try_from(r)?, u16::try_from(c)?, field)?;
            }
        }
        workbook.save(&self.path)?;
        Ok(())
    }

    /// One object per row, keyed by column name, with NULL as `null`.
    pub fn to_json(&self) -> Result<()> {
        let file = File::create(&self.path)?;
        let mut out = BufWriter::new(file);

        let Some((header, rows)) = self.data.split_first() else {
            writeln!(out, "[]")?;
            return Ok(out.flush()?);
        };
        let names: Vec<&str> = header.iter().map(|name| name.as_deref().unwrap_or_default()).collect();

        writeln!(out, "[")?;
        for (i, row) in rows.iter().enumerate() {
            let fields: Vec<String> = names
                .iter()
                .zip(row)
                .map(|(name, value)| {
                    let value = match value {
                        Some(value) => json_string(value),
                        None => "null".to_string(),
                    };
                    format!("{}: {value}", json_string(name))
                })
                .collect();
            let comma = if i + 1 < rows.len() { "," } else { "" };
            writeln!(out, "  {{{}}}{comma}", fields.join(", "))?;
        }
        writeln!(out, "]")?;
        out.flush()?;
        Ok(())
    }

    fn write_delimited(&self, escape: impl Fn(&str) -> String) -> Result<()> {
        let file = File::create(&self.path)?;
        let mut out = BufWriter::new(file);
        for row in &self.data {
            let line: Vec<String> =
                row.iter().map(|field| escape(field.as_deref().unwrap_or_default())).collect();
            writeln!(out, "{}", line.join(&self.separator))?;
        }
        out.flush()?;
        Ok(())
    }
}

/// A JSON string, with the escapes the format requires.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
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

/// Quotes a field that would otherwise break the row, doubling any quote.
fn quote_csv(field: &str, separator: &str) -> String {
    let breaks_row = field.contains(['"', '\n', '\r'])
        || (!separator.is_empty() && field.contains(separator));
    if breaks_row {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A path of its own per test, removed when the guard is dropped.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(extension: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let name = format!(
                "qry-export-{}-{}.{extension}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            Self(std::env::temp_dir().join(name))
        }

        fn path(&self) -> String {
            self.0.display().to_string()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn result() -> QueryResult {
        QueryResult {
            columns: vec!["id".into(), "note".into()],
            rows: vec![
                vec![Some("1".into()), Some("plain".into())],
                vec![Some("2".into()), Some("has, comma and \"quotes\"".into())],
                vec![Some("3".into()), None],
            ],
            affected: Some(3),
        }
    }

    fn config(path: &str, separator: &str, etype: ExportType) -> ExportConfig {
        ExportConfig { path: path.into(), separator: separator.into(), etype }
    }

    #[test]
    fn csv_has_a_header_quotes_what_it_must_and_writes_null_as_empty() {
        let file = TempFile::new("csv");
        let config = config(&file.path(), ",", ExportType::Csv);
        let rows = Exporter::from_result(&result(), &config).write(config.etype).unwrap();

        assert_eq!(rows, 3);
        assert_eq!(
            fs::read_to_string(&file.0).unwrap(),
            "id,note\n1,plain\n2,\"has, comma and \"\"quotes\"\"\"\n3,\n"
        );
    }

    #[test]
    fn text_separates_with_tabs_and_keeps_rows_on_one_line() {
        let file = TempFile::new("txt");
        let mut result = result();
        result.rows[0][1] = Some("two\nlines".into());
        let config = config(&file.path(), "\t", ExportType::Text);
        Exporter::from_result(&result, &config).write(config.etype).unwrap();

        let written = fs::read_to_string(&file.0).unwrap();
        assert_eq!(written.lines().count(), 4);
        assert!(written.starts_with("id\tnote\n1\ttwo lines\n"), "{written}");
    }

    #[test]
    fn json_writes_one_object_per_row_with_null_and_escapes() {
        let file = TempFile::new("json");
        let mut result = result();
        result.rows[0][1] = Some("tab\there \"quoted\" and \\ back".into());
        let config = config(&file.path(), "", ExportType::Json);
        let rows = Exporter::from_result(&result, &config).write(config.etype).unwrap();

        assert_eq!(rows, 3);
        assert_eq!(
            fs::read_to_string(&file.0).unwrap(),
            concat!(
                "[\n",
                "  {\"id\": \"1\", \"note\": \"tab\\there \\\"quoted\\\" and \\\\ back\"},\n",
                "  {\"id\": \"2\", \"note\": \"has, comma and \\\"quotes\\\"\"},\n",
                "  {\"id\": \"3\", \"note\": null}\n",
                "]\n"
            )
        );
    }

    #[test]
    fn excel_writes_a_workbook() {
        let file = TempFile::new("xlsx");
        let config = config(&file.path(), "", ExportType::Excel);
        let rows = Exporter::from_result(&result(), &config).write(config.etype).unwrap();

        assert_eq!(rows, 3);
        // An .xlsx is a zip archive, so it starts with the zip magic bytes.
        assert_eq!(&fs::read(&file.0).unwrap()[..2], b"PK");
    }

    #[test]
    fn a_path_that_cannot_be_written_is_an_error() {
        let config = config("no/such/directory/rows.csv", ",", ExportType::Csv);
        let err = Exporter::from_result(&result(), &config)
            .write(config.etype)
            .unwrap_err();
        assert!(err.to_string().contains("could not write"), "{err}");
    }
}
