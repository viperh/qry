//! The results grid.

use ratatui::{
    prelude::*,
    widgets::{Cell, Paragraph, Row, Table, TableState},
};

use crate::{
    app::state::{Focus, State},
    theme::Theme,
    views::status_line::thousands,
};

const COLUMN_SPACING: u16 = 1;

/// `204 rows` for the title row, once there is a result.
pub(crate) fn summary(state: &State) -> String {
    let Some(result) = &state.results().result else {
        return String::new();
    };
    if result.columns.is_empty() {
        return match result.affected {
            Some(1) => "1 row affected".into(),
            Some(n) => format!(
                "{} rows affected",
                thousands(usize::try_from(n).unwrap_or(usize::MAX))
            ),
            None => "statement executed".into(),
        };
    }
    match result.rows.len() {
        1 => "1 row".into(),
        n => format!("{} rows", thousands(n)),
    }
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let results = state.results();
    let focused = state.focus == Focus::Results;

    let Some(result) = &results.result else {
        return;
    };
    // A statement such as CREATE or INSERT returns no columns.
    if result.columns.is_empty() {
        frame.render_widget(
            Paragraph::new("Statement executed")
                .style(theme.muted())
                .centered(),
            area,
        );
        return;
    }

    let inner = area;
    let total_rows = result.rows.len();
    let (row, col) = (results.row, results.col);
    // One line is taken by the header.
    let page_rows = usize::from(inner.height.saturating_sub(1)).max(1);
    let row_offset = scroll_rows(results.row_offset.get(), row, page_rows, total_rows);
    let col_offset = scroll_columns(&results.widths, results.col_offset.get(), col, inner.width);
    results.page_rows.set(page_rows);
    results.row_offset.set(row_offset);
    results.col_offset.set(col_offset);
    let visible = visible_columns(&results.widths, col_offset, inner.width);

    let header = Row::new(visible.iter().map(|&(c, _)| {
        let name = Cell::from(result.columns[c].as_str());
        if c == col {
            name.style(theme.header_active())
        } else {
            name
        }
    }))
    .style(theme.header());
    let end = (row_offset + page_rows).min(total_rows);
    let rows =
        result.rows[row_offset..end]
            .iter()
            .zip(row_offset..)
            .map(|(row, index)| {
                Row::new(visible.iter().map(
                    |&(c, _)| match row.get(c).and_then(Option::as_deref) {
                        Some(value) => Cell::from(value),
                        None => Cell::from("NULL").style(theme.null_value()),
                    },
                ))
                .style(theme.row(index))
            });
    // The selected row always shows; the cursor cell only while the
    // pane has focus, so it is clear where the keys go.
    let table = Table::new(rows, visible.iter().map(|&(_, w)| Constraint::Length(w)))
        .header(header)
        .column_spacing(COLUMN_SPACING)
        .row_highlight_style(theme.row_selected())
        .cell_highlight_style(if focused {
            theme.cursor_cell()
        } else {
            theme.row_selected()
        });

    let mut table_state = TableState::default();
    if total_rows > 0 {
        table_state.select_cell(Some((row - row_offset, col - col_offset)));
    }
    frame.render_stateful_widget(table, inner, &mut table_state);
}

/// Width of all columns side by side, including the gaps between them.
fn total_width(widths: &[u16]) -> usize {
    widths
        .iter()
        .map(|&w| usize::from(w + COLUMN_SPACING))
        .sum::<usize>()
        - usize::from(COLUMN_SPACING)
}

/// Moves the first visible row as little as possible to keep `selected` in
/// view, without leaving empty space below the last row.
fn scroll_rows(offset: usize, selected: usize, page: usize, total: usize) -> usize {
    let offset = if selected < offset {
        selected
    } else if selected >= offset + page {
        selected + 1 - page
    } else {
        offset
    };
    offset.min(total.saturating_sub(page))
}

/// Moves the first visible column as little as possible so the selected
/// column fits entirely within `available`.
fn scroll_columns(widths: &[u16], offset: usize, selected: usize, available: u16) -> usize {
    let mut offset = offset.min(selected);
    while offset < selected && total_width(&widths[offset..=selected]) > usize::from(available) {
        offset += 1;
    }
    offset
}

/// The columns that fit from `offset` on, as (index, width). The last one may
/// be cut short to fill the remaining space.
fn visible_columns(widths: &[u16], offset: usize, available: u16) -> Vec<(usize, u16)> {
    let mut visible = Vec::new();
    let mut used: u16 = 0;
    for (c, &width) in widths.iter().enumerate().skip(offset) {
        let gap = if visible.is_empty() {
            0
        } else {
            COLUMN_SPACING
        };
        let room = available.saturating_sub(used + gap);
        if room == 0 {
            break;
        }
        let width = width.min(room);
        visible.push((c, width));
        used += gap + width;
    }
    visible
}

#[cfg(test)]
#[path = "results_tests.rs"]
pub(crate) mod results_tests;
