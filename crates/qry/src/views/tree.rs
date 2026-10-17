//! The schema browser: connections, their catalogs, tables and views, and
//! those objects' columns and indexes.
//!
//! The rows were flattened by the reducer ([`schematree::flatten`]); this
//! draws only the ones in view, so a catalog of thousands of tables costs
//! no more per frame than one of ten.

use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::{
        schematree::{self, TreeNode, VisibleRow},
        state::{Focus, State},
    },
    theme::Theme,
    views::{overlay::SPINNER, pane_title},
};

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let tree = &state.tree;
    let focused = state.focus == Focus::Tree;
    let [title, area] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    pane_title::draw("SCHEMA", "", focused, theme, frame, title);

    // The filter's prompt, and under a partial filter its note, take the
    // bottom rows.
    let filter_rows = match &tree.filter {
        Some(_) if tree.filter_partial => 2,
        Some(_) => 1,
        None => 0,
    };
    let [area, bottom] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(filter_rows)]).areas(area);
    if let Some(filter) = &tree.filter {
        let mut lines = Vec::new();
        if tree.filter_partial {
            lines.push(Line::styled(
                " filter covers loaded objects only",
                theme.dim(),
            ));
        }
        let mut prompt = vec![
            Span::styled(" / ", theme.tree_filter_prompt()),
            Span::styled(filter.text.clone(), theme.tree_connection()),
        ];
        if filter.editing && focused {
            prompt.push(Span::styled(" ", theme.cursor_cell()));
        }
        lines.push(Line::from(prompt));
        frame.render_widget(Paragraph::new(lines), bottom);
    }

    let rows = &tree.rows;
    if rows.is_empty() {
        let text = if tree.filter.is_some() {
            "No matches"
        } else {
            "No connections yet"
        };
        frame.render_widget(Paragraph::new(text).style(theme.dim()).centered(), area);
        return;
    }

    let page = usize::from(area.height).max(1);
    tree.page.set(page);
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

    let needle = tree
        .filter
        .as_ref()
        .map(|f| f.text.to_lowercase())
        .filter(|t| !t.is_empty());
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(page)
        .map(|(i, row)| {
            // Children under it, whether open or kept by the filter.
            let showing = rows.get(i + 1).is_some_and(|next| next.depth > row.depth);
            line(
                state,
                *row,
                showing,
                i == selected,
                focused,
                needle.as_deref(),
                theme,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// `name` with the part that matches the filter in `match_highlight`.
fn highlighted(
    name: &str,
    needle: Option<&str>,
    style: Style,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let lower = name.to_lowercase();
    // Only where lower-casing kept the byte offsets, which is every ASCII
    // name and most others.
    let found = needle
        .filter(|_| lower.len() == name.len())
        .and_then(|n| Some((lower.find(n)?, n.len())))
        .filter(|(at, len)| name.is_char_boundary(*at) && name.is_char_boundary(at + len));
    match found {
        Some((at, len)) => vec![
            Span::styled(name[..at].to_string(), style),
            Span::styled(name[at..at + len].to_string(), theme.match_highlight()),
            Span::styled(name[at + len..].to_string(), style),
        ],
        None => vec![Span::styled(name.to_string(), style)],
    }
}

/// One row of the tree. The selected one has `selection_bg`, with bold amber
/// text while the tree has focus; the active session's connection is bold,
/// with an amber `▌` in the gutter.
fn line(
    state: &State,
    row: VisibleRow,
    showing: bool,
    selected: bool,
    focused: bool,
    needle: Option<&str>,
    theme: &Theme,
) -> Line<'static> {
    let node = row.node;
    let frame_glyph = SPINNER[state.tree.spinner % SPINNER.len()];
    let active = matches!(node, TreeNode::Connection(i)
        if state.tree.connections.get(i).is_some_and(|n| state.sessions.active().connection_id == n.record.id));
    let mut spans = vec![if active {
        Span::styled("▌", theme.tree_gutter())
    } else {
        Span::raw(" ")
    }];
    spans.push(Span::raw("  ".repeat(usize::from(row.depth))));
    if node.expandable() {
        let marker = if showing || schematree::is_open(state, node) {
            "▾ "
        } else {
            "▸ "
        };
        spans.push(Span::styled(marker, theme.dim()));
    } else {
        spans.push(Span::raw("  "));
    }
    // A selected label takes the line's colour.
    let label = |base: Style| if selected { Style::default() } else { base };
    let spinner = |spans: &mut Vec<Span<'static>>| {
        if schematree::is_loading(state, node) {
            spans.push(Span::styled(format!(" {frame_glyph}"), theme.spinner()));
        }
    };
    match node {
        TreeNode::Connection(i) => {
            let connection = &state.tree.connections[i];
            let status = state.sessions.connection_status(&connection.record.id);
            let (style, glyph) = theme.tree_state(status, frame_glyph);
            spans.push(Span::styled(format!("{glyph} "), style));
            let (style, badge) = theme.tree_badge(connection.record.driver);
            spans.push(Span::styled(format!("{badge}  "), style));
            spans.extend(highlighted(
                connection.title(),
                needle,
                label(theme.tree_connection()),
                theme,
            ));
            // `[prod]`, `[staging]`, `[dev]`, in the label's colour.
            if let Some(tag) = connection.record.label {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{}]", tag.name()),
                    theme.conn_label(tag),
                ));
            }
            spinner(&mut spans);
        }
        TreeNode::Catalog { conn, cat } => {
            if let Some(catalog) = schematree::catalog(state, conn, cat) {
                let style = if catalog.is_system {
                    theme.tree_folder()
                } else {
                    theme.tree_connection()
                };
                spans.extend(highlighted(&catalog.name, needle, label(style), theme));
            }
            spinner(&mut spans);
        }
        TreeNode::Folder { folder, .. } => {
            spans.push(Span::styled(folder.label(), label(theme.tree_folder())));
            if let Some(n) = schematree::count(state, node) {
                spans.push(Span::styled(format!("  {n}"), theme.tree_count()));
            }
        }
        TreeNode::Object { conn, cat, obj } => {
            if let Some(object) = schematree::object(state, conn, cat, obj) {
                spans.extend(highlighted(
                    &object.name,
                    needle,
                    label(theme.tree_object()),
                    theme,
                ));
            }
            spinner(&mut spans);
        }
        TreeNode::Section { section, .. } => {
            spans.push(Span::styled(section.label(), label(theme.tree_folder())));
            if let Some(n) = schematree::count(state, node) {
                spans.push(Span::styled(format!("  {n}"), theme.tree_count()));
            }
        }
        TreeNode::Column {
            conn,
            cat,
            obj,
            col,
        } => {
            if let Some(details) = schematree::details(state, conn, cat, obj)
                && let Some(column) = details.columns.get(col)
            {
                // The types line up; the pane is narrow, so the markers follow them.
                let widest = |f: fn(&qry_core::schema::Column) -> &str| {
                    details
                        .columns
                        .iter()
                        .map(|c| f(c).chars().count())
                        .max()
                        .unwrap_or(0)
                };
                let pad = widest(|c| &c.name).saturating_sub(column.name.chars().count());

                spans.extend(highlighted(
                    &column.name,
                    needle,
                    label(theme.tree_object()),
                    theme,
                ));
                spans.push(Span::raw(" ".repeat(pad + 2)));
                spans.push(Span::styled(
                    column.data_type.clone(),
                    theme.tree_column_type(),
                ));
                if column.primary_key {
                    spans.push(Span::styled("  PK", theme.tree_pk()));
                }
                if let Some((table, to)) = &column.references {
                    let target = if to.is_empty() {
                        table.clone()
                    } else {
                        format!("{table}.{to}")
                    };
                    spans.push(Span::styled(format!("  FK → {target}"), theme.tree_fk()));
                }
                if column.nullable {
                    spans.push(Span::styled("  NULL", theme.tree_null()));
                }
            }
        }
        TreeNode::Index {
            conn,
            cat,
            obj,
            idx,
        } => {
            if let Some(index) =
                schematree::details(state, conn, cat, obj).and_then(|d| d.indexes.get(idx))
            {
                spans.extend(highlighted(
                    &index.name,
                    needle,
                    label(theme.tree_object()),
                    theme,
                ));
                spans.push(Span::styled(
                    format!("  ({})", index.columns.join(", ")),
                    theme.tree_column_type(),
                ));
                if index.unique {
                    spans.push(Span::styled("  UNIQUE", theme.tree_pk()));
                }
            }
        }
        TreeNode::Error { conn, at } => {
            let message = schematree::error(state, conn, at).unwrap_or("failed");
            let message = message.lines().next().unwrap_or_default();
            spans.push(Span::styled(format!("! {message}"), theme.tree_error()));
        }
        TreeNode::Note { text, .. } => spans.push(Span::styled(text, theme.tree_kind())),
    }
    let line = Line::from(spans);
    let line = match (selected, focused) {
        (true, true) => line.style(theme.tree_connection_selected()),
        (true, false) => line.style(theme.row_selected()),
        (false, _) => line,
    };
    if active { line.bold() } else { line }
}
