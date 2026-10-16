//! The results grid.

use ratatui::{
    prelude::*,
    widgets::{Cell, Paragraph, Row, Table, TableState},
};

use crate::{
    app::state::{Focus, State, TabError},
    theme::Theme,
    views::status_line::thousands,
};

const COLUMN_SPACING: u16 = 1;

/// `204 rows` for the title row, once there is a result; nothing when
/// there are tabs, which say it themselves.
pub(crate) fn summary(state: &State) -> String {
    if state.sessions.active().tabs.len() > 1 {
        return String::new();
    }
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

/// With more than one result, the tabs over the label's detail: ` 1 users
/// · 42 `, the shown one underlined, a failed one coral.
pub(crate) fn draw_tabs(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let session = state.sessions.active();
    if session.tabs.len() < 2 {
        return;
    }
    let start = area.x + u16::try_from(" RESULTS  ".len()).unwrap_or(0);
    let room = area.right().saturating_sub(start);
    let mut spans = Vec::new();
    for (i, tab) in session.tabs.iter().enumerate() {
        let text = format!(" {} {} ", tab.number, tab.label);
        let style = match (i == session.active_tab, tab.error.is_some()) {
            (true, _) => theme.tab_active(),
            (false, true) => theme.tab().patch(theme.value_error()),
            (false, false) => theme.tab(),
        };
        spans.push(Span::styled(text, style));
    }
    // Keep the shown tab in view: drop tabs from the left until it fits.
    let mut first = 0;
    let width = |spans: &[Span]| spans.iter().map(Span::width).sum::<usize>();
    while first < session.active_tab
        && width(&spans[first..=session.active_tab]) > usize::from(room)
    {
        first += 1;
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans.split_off(first))).style(theme.pane_title()),
        Rect::new(start, area.y, room, 1),
    );
}

/// A failed statement: the message, and with a position, its line with a
/// `^~~~` under where the server stopped.
fn draw_error(error: &TabError, theme: &Theme, frame: &mut Frame, area: Rect) {
    let mut lines = vec![Line::styled(
        format!(" ✗ {}", error.message),
        theme.value_error(),
    )];
    if let Some(position) = error.position.filter(|p| *p <= error.sql.len()) {
        let line_start = error.sql[..position].rfind('\n').map_or(0, |n| n + 1);
        let line_end = error.sql[position..]
            .find('\n')
            .map_or(error.sql.len(), |n| position + n);
        let column = error.sql[line_start..position].chars().count();
        let rest = &error.sql[position..line_end];
        let marked = rest
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()
            .map_or(1, |w| w.chars().count().max(1));
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("   {}", &error.sql[line_start..line_end]),
            theme.muted(),
        ));
        lines.push(Line::styled(
            format!("   {}^{}", " ".repeat(column), "~".repeat(marked - 1)),
            theme.value_error(),
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let session = state.sessions.active();
    if let Some(error) = &session.tabs[session.active_tab.min(session.tabs.len() - 1)].error {
        draw_error(error, theme, frame, area);
        return;
    }
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
        let name = Cell::from(result.columns[c].name.as_str());
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
