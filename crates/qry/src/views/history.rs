//! The history dialog and the Manage snippets list: purple choosers in the
//! palette's frame, since both are command-like. The history shows the
//! query, then one row per entry: `✓`/`✗`, when it ran, the first line of
//! its SQL with the matched characters amber, and its rows and time, dim
//! and right-aligned.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────────┐
//! │ › sel usr▏                                         local-app · 238 entries   │
//! │ ✓ 09:12  SELECT * FROM users WHERE id = 3                      1 row  4 ms   │
//! │ ✗ yday   SELECT * FROM user_s                                  error         │
//! └─ Enter insert  ctrl-enter run  tab all connections  ctrl-d delete  Esc close ┘
//! ```

use ratatui::{
    prelude::*,
    widgets::{Block, Clear},
};

use super::palette::palette_rect;
use crate::{
    action::Action,
    app::{
        history::{HistoryDialog, SnippetList},
        state::State,
    },
    keymap::{KeyContext, display_chord},
    theme::Theme,
};

/// The time column's width: `09:12`, `yday`, `Oct 03`.
const TIME_WIDTH: usize = 6;
/// The connection column's width, in "all connections" mode.
const CONNECTION_WIDTH: usize = 14;
/// The rows-and-time column's width.
const STATS_WIDTH: usize = 16;

/// `text` cut to `width` display cells, with `…` where it was cut, then
/// padded to `width`. Only as much of `text` as fits is looked at, so a
/// 100 KB statement costs no more than a short one.
fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let w = Span::raw(c.to_string()).width();
        let last = chars.peek().is_none();
        if used + w > width || (!last && used + w == width) {
            if used < width {
                out.push('…');
                used += 1;
            }
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str(&" ".repeat(width.saturating_sub(used)));
    out
}

/// What a connection is called: its tree title, else an open session's,
/// else its id.
fn connection_title(state: &State, connection: &str) -> String {
    if let Some(i) = state.tree.position(connection) {
        return state.tree.connections[i].title().to_string();
    }
    state
        .sessions
        .iter()
        .find(|s| s.connection_id == connection)
        .map_or_else(|| connection.to_string(), |s| s.title.clone())
}

/// `key label` pairs for the bottom border, keys from the `Palette`
/// context; an unbound action is left out, a literal key is shown as is.
/// Those that would not fit in `width` cells are left off the end.
fn footer<'a>(
    state: &State,
    theme: &Theme,
    pairs: &[(Result<Action, &'a str>, &'a str)],
    width: u16,
) -> Line<'a> {
    let keymap = &state.config.keymap;
    let mut spans = vec![Span::styled("─ ", theme.palette_border())];
    // The corners, the leading `─ ` and a cell to spare.
    let mut room = usize::from(width).saturating_sub(5);
    for (key, label) in pairs {
        let key = match key {
            Ok(action) => {
                let chords = keymap.chords_for(KeyContext::Palette, action);
                if chords.is_empty() {
                    continue;
                }
                display_chord(&chords)
            }
            Err(literal) => (*literal).to_string(),
        };
        let label = format!(" {label} ");
        let needed = Span::raw(key.as_str()).width() + label.chars().count();
        if needed > room {
            break;
        }
        room -= needed + 1;
        spans.push(Span::styled(key, theme.hint_key()));
        spans.push(Span::styled(label, theme.hint()));
        spans.push(Span::styled(" ", theme.hint()));
    }
    // The border goes on after the last label.
    spans.pop();
    Line::from(spans)
}

/// The frame, cleared, with `footer` in its bottom border; returns the
/// inner rect, one cell in from each side.
fn frame_with(frame: &mut Frame, theme: &Theme, rect: Rect, footer: Line<'_>) -> Rect {
    let block = Block::bordered()
        .style(theme.dialog())
        .border_style(theme.palette_border())
        .title_bottom(footer);
    let inner = block.inner(rect).inner(Margin::new(1, 0));
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    inner
}

/// The query line: `› ` and the text with a block cursor at the left, and
/// `right` right-aligned.
fn query_line(
    frame: &mut Frame,
    theme: &Theme,
    inner: Rect,
    input: Option<(&str, usize)>,
    right: Span<'_>,
) {
    let mut spans = vec![Span::styled("› ", theme.field_focused())];
    if let Some((value, cursor)) = input {
        let chars: Vec<char> = value.chars().collect();
        let cursor = cursor.min(chars.len());
        spans.push(Span::styled(
            chars[..cursor].iter().collect::<String>(),
            theme.field(),
        ));
        let under = chars.get(cursor).copied().unwrap_or(' ');
        spans.push(Span::styled(under.to_string(), theme.cursor_cell()));
        spans.push(Span::styled(
            chars.iter().skip(cursor + 1).collect::<String>(),
            theme.field(),
        ));
    }
    frame.render_widget(
        Line::from(spans),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let width = u16::try_from(right.width()).unwrap_or(0).min(inner.width);
    frame.render_widget(right, Rect::new(inner.right() - width, inner.y, width, 1));
}

/// Keeps `selected` in view among `rows` lines, starting from `offset`.
fn scrolled(offset: usize, selected: usize, rows: usize) -> usize {
    if selected < offset {
        selected
    } else if selected >= offset + rows {
        selected + 1 - rows
    } else {
        offset
    }
}

/// `1 row  4 ms`, `42 rows 31 ms`, `error`.
fn stats(ok: bool, rows: Option<u64>, ms: u64) -> String {
    match (ok, rows) {
        (false, _) => "error".into(),
        (true, Some(1)) => format!("1 row  {ms} ms"),
        (true, Some(n)) => format!("{n} rows {ms} ms"),
        (true, None) => format!("{ms} ms"),
    }
}

pub(crate) fn draw(
    dialog: &HistoryDialog,
    state: &State,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let rect = palette_rect(area, dialog.rows.len().max(1));
    if rect.width < 20 || rect.height < 3 {
        return;
    }
    let scope = if dialog.all {
        "this connection"
    } else {
        "all connections"
    };
    let pairs = [
        (Ok(Action::Submit), "insert"),
        (Ok(Action::SubmitAndRun), "run"),
        (Ok(Action::NextField), scope),
        (Ok(Action::DeleteEntry), "delete"),
        (Ok(Action::Cancel), "close"),
    ];
    let inner = frame_with(frame, theme, rect, footer(state, theme, &pairs, rect.width));

    let right = if dialog.deleting {
        let key = display_chord(
            &state
                .config
                .keymap
                .chords_for(KeyContext::Palette, &Action::DeleteEntry),
        );
        Span::styled(format!("press {key} again"), theme.value_error())
    } else {
        let name = if dialog.all {
            "all connections".to_string()
        } else {
            connection_title(state, &dialog.connection)
        };
        let n = dialog.rows.len();
        let noun = if n == 1 { "entry" } else { "entries" };
        Span::styled(format!("{name} · {n} {noun}"), theme.dim())
    };
    query_line(
        frame,
        theme,
        inner,
        Some((&dialog.input.value, dialog.input.cursor)),
        right,
    );

    let rows = usize::from(inner.height.saturating_sub(1));
    if rows == 0 {
        return;
    }
    if dialog.rows.is_empty() {
        let note = if dialog.input.value.is_empty() {
            "(no history yet)"
        } else {
            "(nothing matches)"
        };
        frame.render_widget(
            Span::styled(note, theme.dim()),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
        return;
    }
    let offset = scrolled(dialog.offset.get(), dialog.selected, rows);
    dialog.offset.set(offset);

    let width = usize::from(inner.width);
    let connection_width = if dialog.all { CONNECTION_WIDTH } else { 0 };
    let sql_width = width.saturating_sub(2 + TIME_WIDTH + 1 + connection_width + STATS_WIDTH);
    for (r, row) in dialog.rows.iter().enumerate().skip(offset).take(rows) {
        let Some(entry) = state
            .history
            .get(&row.connection)
            .and_then(|h| h.get(row.index))
        else {
            continue;
        };
        let base = if r == dialog.selected {
            theme.palette_item_selected()
        } else {
            theme.dialog()
        };
        let (glyph, glyph_style) = if entry.ok {
            ("✓", theme.value_ok())
        } else {
            ("✗", theme.value_error())
        };
        let mut spans = vec![
            Span::styled(glyph, base.patch(glyph_style)),
            Span::styled(" ", base),
            Span::styled(
                fit(
                    &qry_core::history::relative(entry.ts, state.wall.unix, state.wall.offset),
                    TIME_WIDTH,
                ),
                base.patch(theme.dim()),
            ),
            Span::styled(" ", base),
        ];
        if dialog.all {
            spans.push(Span::styled(
                fit(
                    &connection_title(state, &row.connection),
                    CONNECTION_WIDTH - 1,
                ) + " ",
                base.patch(theme.dim()),
            ));
        }
        let first_line = entry.sql.trim_start().split('\n').next().unwrap_or("");
        // Positions count from the start of the SQL, before any trimming.
        let lead = entry.sql.chars().count() - entry.sql.trim_start().chars().count();
        for (i, c) in fit(first_line, sql_width).chars().enumerate() {
            let style = if row.positions.contains(&(i + lead)) {
                base.patch(theme.palette_match())
            } else {
                base
            };
            spans.push(Span::styled(c.to_string(), style));
        }
        let stats = stats(entry.ok, entry.rows, entry.ms);
        let stats = super::help::truncate(&stats, STATS_WIDTH);
        let pad = STATS_WIDTH.saturating_sub(Span::raw(stats.as_str()).width());
        spans.push(Span::styled(" ".repeat(pad), base));
        spans.push(Span::styled(stats, base.patch(theme.dim())));
        let y = inner.y + 1 + u16::try_from(r - offset).unwrap_or(0);
        frame.render_widget(Line::from(spans), Rect::new(inner.x, y, inner.width, 1));
    }
}

pub(crate) fn draw_snippets(
    list: &SnippetList,
    state: &State,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let rect = palette_rect(area, list.rows.len().max(1));
    if rect.width < 20 || rect.height < 3 {
        return;
    }
    let pairs = [
        (Ok(Action::Submit), "edit"),
        (Err("r"), "rename"),
        (Ok(Action::DeleteEntry), "delete"),
        (Ok(Action::SubmitAndRun), "run"),
        (Ok(Action::Cancel), "close"),
    ];
    let inner = frame_with(frame, theme, rect, footer(state, theme, &pairs, rect.width));
    let heading = Line::from(vec![Span::styled(
        "Snippets",
        theme.dialog_title(theme.accent(crate::theme::Accent::Command)),
    )]);
    frame.render_widget(heading, Rect::new(inner.x, inner.y, inner.width, 1));
    let n = list.rows.len();
    let count = Span::styled(format!("{n} saved"), theme.dim());
    let width = u16::try_from(count.width()).unwrap_or(0).min(inner.width);
    frame.render_widget(count, Rect::new(inner.right() - width, inner.y, width, 1));

    let rows = usize::from(inner.height.saturating_sub(1));
    if rows == 0 {
        return;
    }
    if list.rows.is_empty() {
        frame.render_widget(
            Span::styled(
                "(no snippets yet: ctrl-s in the editor saves one)",
                theme.dim(),
            ),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );
        return;
    }
    let offset = scrolled(list.offset.get(), list.selected, rows);
    list.offset.set(offset);
    let width = usize::from(inner.width);
    let name_width = (width / 3).max(1);
    let scope_width = CONNECTION_WIDTH.min(width.saturating_sub(name_width));
    let sql_width = width.saturating_sub(name_width + scope_width);
    for (r, &i) in list.rows.iter().enumerate().skip(offset).take(rows) {
        let Some(snippet) = state.snippets.get(i) else {
            continue;
        };
        let base = if r == list.selected {
            theme.palette_item_selected()
        } else {
            theme.dialog()
        };
        let scope = snippet
            .connection
            .as_deref()
            .map_or_else(|| "global".to_string(), |c| connection_title(state, c));
        let first_line = snippet.sql.trim_start().split('\n').next().unwrap_or("");
        let spans = vec![
            Span::styled(fit(&snippet.name, name_width), base),
            Span::styled(fit(&scope, scope_width), base.patch(theme.dim())),
            Span::styled(fit(first_line, sql_width), base.patch(theme.dim())),
        ];
        let y = inner.y + 1 + u16::try_from(r - offset).unwrap_or(0);
        frame.render_widget(Line::from(spans), Rect::new(inner.x, y, inner.width, 1));
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod history_tests;
