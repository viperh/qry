#![doc(hidden)]
//! Entry points for the cargo-fuzz targets in `fuzz/` and their proptest
//! twins in `fuzz_tests.rs`. Each must never panic, whatever the input, and
//! checks the invariants its code promises: SQL token spans tile the text,
//! statements are ordered and cover all the code, a sort gives a
//! permutation, and so on.
//!
//! The SQL bodies run on every keystroke in the editor, and the grid and
//! copy bodies on whatever a database sends back: a panic in any of them
//! takes the terminal down with the user's unsaved query.

use crate::{
    ColumnMeta, QueryResult, ValueKind,
    copy::{self, CopyFormat, Selection},
    grid::{self, Order},
    sql::{self, Dialect, TokenKind},
};

/// The lexer, in every dialect: no panic, and the tokens tile the input
/// (non-empty, contiguous, on char boundaries), so their texts joined are
/// the input.
pub fn fuzz_sql_lex(data: &[u8]) {
    let src = String::from_utf8_lossy(data);
    for dialect in Dialect::ALL {
        let tokens = sql::lex(dialect, &src);
        let mut at = 0;
        let mut joined = String::with_capacity(src.len());
        for token in &tokens {
            assert_eq!(token.span.start, at, "{dialect:?}: gap or overlap at {at}");
            assert!(
                token.span.end > token.span.start,
                "{dialect:?}: empty token"
            );
            let text = src
                .get(token.span.clone())
                .unwrap_or_else(|| panic!("{dialect:?}: {:?} off a char boundary", token.span));
            joined.push_str(text);
            at = token.span.end;
        }
        assert_eq!(at, src.len(), "{dialect:?}: the tokens stop short");
        assert_eq!(joined, src, "{dialect:?}");
    }
}

/// The splitter, in every dialect: no panic; statement spans are in
/// bounds, on char boundaries, ascending and non-overlapping, and hold
/// their text. In the dialects without a statement-changing line (PostgreSQL
/// and SQLite), every byte that is not whitespace, a comment or a `;` is in
/// exactly one statement. `statement_at` answers for every offset.
pub fn fuzz_sql_split(data: &[u8]) {
    let src = String::from_utf8_lossy(data);
    for dialect in Dialect::ALL {
        let statements = sql::split(dialect, &src);
        let mut end = 0;
        for statement in &statements {
            let span = statement.span.clone();
            assert!(
                span.start >= end,
                "{dialect:?}: {span:?} overlaps the one before"
            );
            assert!(
                span.start < span.end,
                "{dialect:?}: empty statement {span:?}"
            );
            let text = src
                .get(span.clone())
                .unwrap_or_else(|| panic!("{dialect:?}: {span:?} out of bounds or mid-char"));
            assert_eq!(text, statement.text, "{dialect:?}");
            let _ = sql::is_ddl(dialect, text);
            end = span.end;
        }
        if matches!(dialect, Dialect::Postgres | Dialect::Sqlite) {
            for token in sql::lex(dialect, &src) {
                let last = match token.kind {
                    TokenKind::Whitespace | TokenKind::Comment | TokenKind::Delimiter => continue,
                    TokenKind::Unterminated => {
                        token.span.start + src[token.span.clone()].trim_end().len()
                    }
                    _ => token.span.end,
                };
                for byte in token.span.start..last {
                    let holders = statements.iter().filter(|s| s.span.contains(&byte)).count();
                    assert_eq!(
                        holders, 1,
                        "{dialect:?}: byte {byte} is in {holders} statements"
                    );
                }
            }
        }
        for offset in [0, src.len() / 2, src.len()] {
            if let Some(i) = sql::statement_at(&statements, offset) {
                assert!(i < statements.len());
            } else {
                assert!(statements.is_empty());
            }
        }
    }
}

const KINDS: [ValueKind; 11] = [
    ValueKind::Integer,
    ValueKind::Decimal,
    ValueKind::Float,
    ValueKind::Text,
    ValueKind::Bool,
    ValueKind::Date,
    ValueKind::Time,
    ValueKind::Timestamp,
    ValueKind::Json,
    ValueKind::Binary,
    ValueKind::Other,
];

/// A result read from fuzz bytes: the first byte gives the number of
/// columns (1 to 4), the next one a kind per column; the rest, split on
/// `0xFF`, are the column names and then the cells, row by row, where a
/// lone `0xFE` is NULL. Also hands back the two bytes after the kinds, for
/// the caller's own choices.
pub fn result_from_bytes(data: &[u8]) -> (QueryResult, [u8; 2]) {
    let byte = |i: usize| data.get(i).copied().unwrap_or(0);
    let ncols = usize::from(byte(0) % 4) + 1;
    let choices = [byte(ncols + 1), byte(ncols + 2)];
    let mut cells = data
        .get(ncols + 3..)
        .unwrap_or_default()
        .split(|&b| b == 0xFF);
    let columns: Vec<ColumnMeta> = (0..ncols)
        .map(|c| ColumnMeta {
            name: String::from_utf8_lossy(cells.next().unwrap_or_default()).into_owned(),
            type_name: String::new(),
            kind: KINDS[usize::from(byte(c + 1)) % KINDS.len()],
        })
        .collect();
    let values: Vec<Option<String>> = cells
        .map(|cell| (cell != [0xFE]).then(|| String::from_utf8_lossy(cell).into_owned()))
        .collect();
    let rows = values.chunks(ncols).map(<[_]>::to_vec).collect();
    let result = QueryResult {
        columns,
        rows,
        affected: None,
        elapsed: std::time::Duration::ZERO,
        truncated: false,
    };
    (result, choices)
}

/// The grid's logic on any result: widths stay within the cap, the visible
/// columns fit the room, the sort is a permutation (whatever the keys,
/// NaNs and mixed text included), and the search's matches are real cells
/// in row order.
pub fn fuzz_grid(data: &[u8]) {
    let (result, [keys, budget]) = result_from_bytes(data);
    let ncols = result.columns.len();

    let cap = u16::from(budget).max(1);
    for (c, &w) in grid::column_widths(&result, cap).iter().enumerate() {
        assert!((1..=cap).contains(&w), "column {c} is {w} wide");
    }
    // Any widths and any room, read straight from the bytes: the first two
    // are the room, the rest the widths.
    let words: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    if let Some((&available, widths)) = words.split_first() {
        let visible = grid::visible_columns(widths, usize::from(keys), keys % 2 == 0, available);
        let used: u32 = visible.iter().map(|&(_, w)| u32::from(w)).sum::<u32>()
            + u32::from(grid::COLUMN_SPACING)
                * u32::try_from(visible.len().saturating_sub(1)).unwrap_or(0);
        assert!(
            used <= u32::from(available),
            "{used} cells drawn in {available}"
        );
        assert!(
            visible
                .iter()
                .all(|&(c, w)| c < widths.len() && w <= widths[c])
        );
    }

    // Up to two sort keys, one past the last column included.
    let key = |bits: u8| {
        let col = usize::from(bits & 0x7) % (ncols + 1);
        let dir = if bits & 0x8 == 0 {
            Order::Asc
        } else {
            Order::Desc
        };
        (col, dir)
    };
    let sort_keys = [key(keys), key(keys >> 4)];
    let order = grid::sort_permutation(&result, &sort_keys[..1 + usize::from(budget % 2)]);
    let mut seen = order.clone();
    seen.sort_unstable();
    assert!(
        seen.iter()
            .copied()
            .eq(0..u32::try_from(result.rows.len()).unwrap_or(u32::MAX)),
        "not a permutation: {order:?}"
    );

    let needle = result.columns.first().map_or("", |c| c.name.as_str());
    let mut search = grid::Search::new(needle);
    let step = usize::from(budget % 7) + 1;
    let mut rounds = 0;
    while !search.step(&result, step) {
        rounds += 1;
        assert!(rounds <= result.rows.len(), "the search never finishes");
    }
    assert!(search.matches.windows(2).all(|w| w[0] < w[1]));
    for &(r, c) in &search.matches {
        let text = grid::display_text(result.rows[r][c].as_deref()).to_lowercase();
        assert!(text.contains(search.needle()));
    }
    if let Some(&first) = search.matches.first() {
        assert!(search.next_from(first, true).is_some());
        assert!(search.next_from(first, false).is_some());
    }
    for row in &result.rows {
        for cell in row {
            let shown = grid::display_text(cell.as_deref());
            assert!(!shown.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}'));
            let _ = grid::width(&shown);
        }
    }
}

/// The copy writers on any result, in every format and dialect, with rows
/// and columns that may be out of range: no panic, and the TSV and
/// Markdown writers, which escape line breaks, give one line per row plus
/// their header lines.
pub fn fuzz_copy_render(data: &[u8]) {
    let (result, [pick, extra]) = result_from_bytes(data);
    let rows: Vec<usize> = (0..result.rows.len() + usize::from(extra % 2)).collect();
    let mut selection = Selection::all(&result, rows);
    if pick % 2 == 1 {
        selection.cols.push(result.columns.len());
    }
    let table = result.columns.last().map_or("t", |c| c.name.as_str());
    selection.header = pick & 2 == 0;
    let lines = |text: &str| text.matches('\n').count() + 1;
    for format in CopyFormat::ALL {
        for dialect in Dialect::ALL {
            let text = copy::render(&result, &selection, format, dialect, table);
            let rows = selection.rows.len();
            match format {
                CopyFormat::Markdown => assert_eq!(lines(&text), rows + 2, "{text:?}"),
                CopyFormat::Tsv if rows + usize::from(selection.header) > 0 => {
                    assert_eq!(
                        lines(&text),
                        rows + usize::from(selection.header),
                        "{text:?}"
                    );
                }
                _ => {}
            }
        }
    }
    for dialect in Dialect::ALL {
        let quoted = copy::quote_ident(dialect, table);
        assert!(!quoted.is_empty());
    }
}

#[cfg(test)]
#[path = "fuzz_tests.rs"]
mod fuzz_tests;
