//! The connection tree.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::{Focus, Row, State},
    theme::Theme,
    views::pane_title,
};

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let tree = &state.tree;
    let focused = state.focus == Focus::Tree;
    let [title, area] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    pane_title::draw("SCHEMA", "", focused, theme, frame, title);

    let rows = tree.rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No connections yet")
                .style(theme.dim())
                .centered(),
            area,
        );
        return;
    }

    let page = usize::from(area.height).max(1);
    let selected = tree.selected.min(rows.len() - 1);
    // Scroll only as far as it takes to keep the selected row in view.
    let mut offset = tree.offset.get();
    if selected < offset {
        offset = selected;
    } else if selected >= offset + page {
        offset = selected + 1 - page;
    }
    offset = offset.min(rows.len().saturating_sub(page));
    tree.offset.set(offset);

    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(page)
        .map(|(i, row)| line(state, row, i == selected, focused, theme))
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// One row of the tree; `selected` is the row the keys act on, amber only
/// while the tree has focus. A connection with a session shows how it
/// stands: `●` open, `◌` connecting, `✗` failed.
fn line(state: &State, row: &Row, selected: bool, focused: bool, theme: &Theme) -> Line<'static> {
    let tree = &state.tree;
    let line = match row {
        Row::Connection(i) => {
            let connection = &tree.connections[*i];
            let marker = if connection.expanded { "▾" } else { "▸" };
            let style = match (selected, focused) {
                (true, true) => theme.tree_connection_selected(),
                (true, false) => theme.row_selected(),
                (false, _) => theme.tree_connection(),
            };
            let dot = match state.sessions.connection_status(&connection.record.id) {
                Some(status) => {
                    let (dot_style, glyph) = theme.session_dot(status);
                    Span::styled(format!("{glyph} "), dot_style)
                }
                None => Span::raw("  "),
            };
            let mut spans = vec![
                Span::raw(format!(" {marker} ")),
                dot,
                Span::raw(connection.title().to_string()),
            ];
            if let Some(label) = connection.record.label {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", label.name()),
                    theme.conn_label(label),
                ));
            }
            let line = Line::from(spans).style(style);
            // The active session's connection stands out without taking
            // the colour, which belongs to the selection.
            if state.sessions.active().connection_id == connection.record.id {
                line.bold()
            } else {
                line
            }
        }
        Row::Table(i, t) => {
            #[allow(
                clippy::expect_used,
                reason = "table rows are only built for connections whose tables are loaded"
            )]
            let table = &tree.connections[*i].tables.as_ref().expect("expanded")[*t];
            Line::styled(format!("     {table}"), theme.tree_object())
        }
        Row::Note(note) => Line::styled(format!("     {note}"), theme.tree_kind()),
    };
    // Tables and notes take the selection's background too.
    if selected && !matches!(row, Row::Connection(_)) {
        line.style(theme.row_selected())
    } else {
        line
    }
}
