//! The results grid's logic, without a terminal: how wide each column is
//! drawn, which columns fit, sorting through a permutation, searching in
//! steps, and what a cell shows.

use std::{borrow::Cow, cmp::Ordering};

use unicode_width::UnicodeWidthStr;

use crate::{QueryResult, ValueKind};

/// Rows looked at to size a column; the rest are cut off when drawn.
pub const WIDTH_SAMPLE: usize = 1000;
/// The space between columns.
pub const COLUMN_SPACING: u16 = 1;

/// What a cell shows: `NULL` for a NULL, and the control characters that
/// would break a row as visible glyphs (`↵`, `→`, `␀`, `·`).
pub fn display_text(cell: Option<&str>) -> Cow<'_, str> {
    let Some(text) = cell else {
        return Cow::Borrowed("NULL");
    };
    if !text.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| match c {
                '\n' => '↵',
                '\t' => '→',
                '\0' => '␀',
                c if (c as u32) < 0x20 || c == '\u{7f}' => '·',
                c => c,
            })
            .collect(),
    )
}

/// Whether `c` is one of the glyphs [`display_text`] puts in place of a
/// control character, which the grid draws dim.
pub fn is_control_glyph(c: char) -> bool {
    matches!(c, '↵' | '→' | '␀')
}

/// How many cells `text` takes.
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Each column's width: its widest value among the first
/// [`WIDTH_SAMPLE`] rows or its name, at least 1 and at most `cap`.
pub fn column_widths(result: &QueryResult, cap: u16) -> Vec<u16> {
    result
        .columns
        .iter()
        .enumerate()
        .map(|(c, column)| {
            let widest = result
                .rows
                .iter()
                .take(WIDTH_SAMPLE)
                .map(|row| width(&display_text(row.get(c).and_then(Option::as_deref))))
                .fold(
                    width(&column.name).max(width(&column.type_name)),
                    usize::max,
                );
            u16::try_from(widest)
                .unwrap_or(u16::MAX)
                .clamp(1, cap.max(1))
        })
        .collect()
}

/// The columns that fit in `available` cells from `offset` on, as
/// `(index, width)`; with `frozen`, the first column comes first whatever
/// the offset. The last may be cut short to fill the space.
pub fn visible_columns(
    widths: &[u16],
    offset: usize,
    frozen: bool,
    available: u16,
) -> Vec<(usize, u16)> {
    let mut visible = Vec::new();
    let mut used: u16 = 0;
    let order = (frozen && !widths.is_empty())
        .then_some(0)
        .into_iter()
        .chain(
            (offset.max(usize::from(frozen))..widths.len()).filter(move |&c| !(frozen && c == 0)),
        );
    for c in order {
        let gap = if visible.is_empty() {
            0
        } else {
            COLUMN_SPACING
        };
        let room = available.saturating_sub(used + gap);
        if room == 0 {
            break;
        }
        let w = widths[c].min(room);
        visible.push((c, w));
        used += gap + w;
    }
    visible
}

/// A sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Asc,
    Desc,
}

/// Natural order for text: runs of digits compare as numbers, so `a2`
/// comes before `a10`; otherwise case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (m, n) = (take(&mut a), take(&mut b));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let ord = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x
                    .to_lowercase()
                    .cmp(y.to_lowercase())
                    .then_with(|| x.cmp(&y));
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Compares two values of a column of `kind`: numbers as numbers (text
/// that is not one goes after them), everything else in natural order.
fn compare_values(kind: ValueKind, a: &str, b: &str) -> Ordering {
    if kind.is_numeric() {
        // NaN is not a number here: it compares equal to every number,
        // which is no order at all, and the standard sort panics on such a
        // comparison. As text it goes after the numbers.
        let number = |s: &str| s.trim().parse::<f64>().ok().filter(|x| !x.is_nan());
        match (number(a), number(b)) {
            (Some(x), Some(y)) => return x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            _ => {}
        }
    }
    natural_cmp(a, b)
}

/// The rows' order under `keys` (column, direction), most important first,
/// as indexes into `result.rows`. The result is not changed. NULLs go last
/// ascending and first descending; equal rows keep their order.
pub fn sort_permutation(result: &QueryResult, keys: &[(usize, Order)]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..u32::try_from(result.rows.len()).unwrap_or(u32::MAX)).collect();
    order.sort_by(|&i, &j| {
        let (ri, rj) = (&result.rows[i as usize], &result.rows[j as usize]);
        for &(col, dir) in keys {
            let kind = result.columns.get(col).map_or(ValueKind::Other, |c| c.kind);
            let (a, b) = (
                ri.get(col).and_then(Option::as_deref),
                rj.get(col).and_then(Option::as_deref),
            );
            let ord = match (a, b) {
                (None, None) => Ordering::Equal,
                // NULL is the greatest value: last ascending, first
                // descending.
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => compare_values(kind, a, b),
            };
            let ord = if dir == Order::Desc {
                ord.reverse()
            } else {
                ord
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    });
    order
}

/// A case-insensitive substring search over every cell's shown text, run a
/// few rows at a time so a large result never holds up input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    needle: String,
    /// The next row to look at.
    next: usize,
    /// Every match so far, `(row, column)` in row order.
    pub matches: Vec<(usize, usize)>,
}

impl Search {
    pub fn new(needle: &str) -> Self {
        Self {
            needle: needle.to_lowercase(),
            next: 0,
            matches: Vec::new(),
        }
    }

    pub fn needle(&self) -> &str {
        &self.needle
    }

    /// Whether every row has been looked at.
    pub fn done(&self, result: &QueryResult) -> bool {
        self.next >= result.rows.len()
    }

    /// Looks at up to `budget` more rows. Returns whether it is done.
    pub fn step(&mut self, result: &QueryResult, budget: usize) -> bool {
        if self.needle.is_empty() {
            self.next = result.rows.len();
            return true;
        }
        let end = (self.next + budget).min(result.rows.len());
        for r in self.next..end {
            for (c, cell) in result.rows[r].iter().enumerate() {
                let text = display_text(cell.as_deref());
                if text.to_lowercase().contains(&self.needle) {
                    self.matches.push((r, c));
                }
            }
        }
        self.next = end;
        self.done(result)
    }

    /// The match after (or before) `at`, wrapping around; `None` without
    /// matches. `at` is compared by `(row, column)`.
    pub fn next_from(&self, at: (usize, usize), forward: bool) -> Option<(usize, usize)> {
        if forward {
            self.matches
                .iter()
                .find(|&&m| m > at)
                .or_else(|| self.matches.first())
                .copied()
        } else {
            self.matches
                .iter()
                .rev()
                .find(|&&m| m < at)
                .or_else(|| self.matches.last())
                .copied()
        }
    }
}

#[cfg(test)]
#[path = "grid_tests.rs"]
mod grid_tests;
