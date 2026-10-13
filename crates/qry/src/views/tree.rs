//! The connection tree.

use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};

use crate::{
    app::state::{Focus, Row, State, TreeState},
    theme::Theme,
};

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let tree = &state.tree;
    let (border, title) = if state.focus == Focus::Tree {
        (theme.focused_border(), theme.pane_title_focused())
    } else {
        (theme.border(), theme.pane_title())
    };
    let block = Block::bordered()
        .title(Line::styled(" Connection Tree ", title))
        .border_style(border);

    let rows = tree.rows();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No connections yet")
                .style(theme.dim())
                .centered()
                .block(block),
            area,
        );
        return;
    }

    let inner = block.inner(area);
    let page = usize::from(inner.height).max(1);
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
        .map(|(i, row)| line(tree, row, i == selected, theme))
        .collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// One row of the tree; `selected` is the row the keys act on.
fn line(tree: &TreeState, row: &Row, selected: bool, theme: &Theme) -> Line<'static> {
    let line = match row {
        Row::Connection(i) => {
            let connection = &tree.connections[*i];
            let marker = if connection.expanded { "▾" } else { "▸" };
            let style = if selected {
                theme.tree_connection_selected()
            } else {
                theme.tree_connection()
            };
            let line = Line::styled(format!("{marker} {}", connection.title()), style);
            // The one the worker is on stands out without taking the
            // colour, which belongs to the selection.
            if tree.active == Some(*i) {
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
            Line::styled(format!("    {table}"), theme.tree_object())
        }
        Row::Note(note) => Line::styled(format!("    {note}"), theme.tree_kind()),
    };
    // Tables and notes take the selection's background too.
    if selected && !matches!(row, Row::Connection(_)) {
        line.style(theme.row_selected())
    } else {
        line
    }
}
