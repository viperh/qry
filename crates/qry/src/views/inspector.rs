//! The record inspector, after tachy's `components/inspector.rs`: the
//! cursor's column (name, type, NULLs and distinct values), its most
//! frequent values with eighth-block bars, and the selected row as a
//! record. It is docked on the right from [`INSPECTOR_MIN_WIDTH`] columns,
//! and drawn over the right half of the body below that.

use qry_core::{
    QueryResult,
    inspect::{ColumnProfile, Profiler},
};
use ratatui::{
    prelude::*,
    widgets::{Clear, Paragraph},
};

use super::{
    help::truncate,
    layout::{INSPECTOR_MIN_WIDTH, ScreenLayout, render_vertical_divider},
    overlay::SPINNER,
    pane_title,
    status_line::thousands,
};
use crate::{
    app::state::{Focus, ResultsState, State},
    theme::Theme,
};

/// Cells a TOP VALUES bar takes at most.
const BAR_WIDTH: usize = 8;
/// The partial blocks, an eighth of a cell more each.
const EIGHTHS: [&str; 8] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉"];
/// RECORD's field names are cut at this many cells.
const FIELD_WIDTH: usize = 14;

/// Where the inspector goes on this screen, divider first: the layout's
/// slot when it is docked, else the right half of the body as an overlay.
/// `None` while it is hidden.
pub(crate) fn placement(state: &State, screen: &ScreenLayout, area: Rect) -> Option<(Rect, Rect)> {
    if !state.chrome.inspector {
        return None;
    }
    if let (Some(divider), Some(inspector)) = (screen.inspector_divider, screen.inspector) {
        return Some((divider, inspector));
    }
    // Below the docking width the body's right half, the full body height.
    let body = screen.tree;
    let x = area.x + area.width / 2;
    let width = area.right().saturating_sub(x);
    if area.width >= INSPECTOR_MIN_WIDTH || width < 2 {
        return None;
    }
    let divider = Rect::new(x, body.y, 1, body.height);
    let inspector = Rect::new(x + 1, body.y, width - 1, body.height);
    Some((divider, inspector))
}

/// Draws the inspector where [`placement`] puts it, if anywhere.
pub(crate) fn draw_placed(
    state: &State,
    theme: &Theme,
    frame: &mut Frame,
    screen: &ScreenLayout,
    area: Rect,
) {
    let Some((divider, inspector)) = placement(state, screen, area) else {
        return;
    };
    let focused = state.focus == Focus::Inspector;
    let style = if focused {
        theme.divider_focused()
    } else {
        theme.divider()
    };
    frame.render_widget(Clear, divider.union(inspector));
    frame.render_widget(
        ratatui::widgets::Block::new().style(theme.base()),
        inspector,
    );
    render_vertical_divider(frame, divider, style);
    draw(state, theme, frame, inspector);
}

/// The inspector in `area`: its title row, then the sections.
pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    if area.height == 0 || area.width < 4 {
        return;
    }
    let focused = state.focus == Focus::Inspector;
    let [title, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    pane_title::draw("INSPECTOR", "", focused, theme, frame, title);
    let body = body.inner(Margin::new(1, 0));
    let results = state.results();
    let Some(result) = results.result.as_deref().filter(|r| !r.columns.is_empty()) else {
        frame.render_widget(Paragraph::new(Span::styled("no result", theme.dim())), body);
        return;
    };
    let width = usize::from(body.width);
    let label_style = if focused {
        theme.inspector_label_focused()
    } else {
        theme.inspector_label()
    };
    let label = |text: &'static str, detail: String| {
        let mut spans = vec![Span::styled(text, label_style)];
        if !detail.is_empty() {
            spans.push(Span::styled(format!(" · {detail}"), theme.dim()));
        }
        Line::from(spans)
    };

    let mut lines = column_lines(result, results, width, &label, theme);
    lines.push(Line::default());

    // RECORD takes whatever is left, and scrolls to keep the field in view.
    let rows = result.rows.len();
    let detail = if rows == 0 {
        "no rows".to_string()
    } else {
        format!("row {} / {}", thousands(results.row + 1), thousands(rows))
    };
    lines.push(label("RECORD", detail));
    let room = usize::from(body.height).saturating_sub(lines.len());
    let fields = result.columns.len();
    let mut scroll = results.record_scroll.get();
    if results.col < scroll {
        scroll = results.col;
    } else if room > 0 && results.col >= scroll + room {
        scroll = results.col + 1 - room;
    }
    scroll = scroll.min(fields.saturating_sub(room));
    results.record_scroll.set(scroll);
    let row = result.rows.get(results.data_row(results.row));
    let name_width = result
        .columns
        .iter()
        .map(|c| qry_core::grid::width(&c.name))
        .max()
        .unwrap_or(1)
        .clamp(1, FIELD_WIDTH.min(width / 2));
    let value_width = width.saturating_sub(name_width + 2);
    for (c, column) in result.columns.iter().enumerate().skip(scroll).take(room) {
        let name = truncate(&column.name, name_width);
        let pad = name_width.saturating_sub(qry_core::grid::width(&name));
        let cell = row.and_then(|r| r.get(c)).and_then(Option::as_deref);
        let value = Span::styled(
            truncate(&qry_core::grid::display_text(cell), value_width),
            if cell.is_some() || row.is_none() {
                theme.record_value()
            } else {
                theme.null_value()
            },
        );
        let mut line = Line::from(vec![
            Span::styled(name, theme.record_field()),
            Span::raw(" ".repeat(pad + 2)),
            if row.is_some() { value } else { Span::raw("") },
        ]);
        if focused && c == results.col {
            // The selection runs the panel's whole width.
            let used = line.width();
            line.push_span(Span::raw(" ".repeat(width.saturating_sub(used))));
            line = line.style(theme.row_selected());
        }
        lines.push(line);
    }
    frame.render_widget(Paragraph::new(lines), body);
}

/// The COLUMN section and, when it says something, TOP VALUES.
fn column_lines<'a>(
    result: &QueryResult,
    results: &ResultsState,
    width: usize,
    label: &dyn Fn(&'static str, String) -> Line<'a>,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let col = results.col.min(result.columns.len() - 1);
    let column = &result.columns[col];
    let mut lines = vec![label("COLUMN", String::new())];
    let mut about = vec![column.name.clone()];
    if !column.type_name.is_empty() {
        about.push(column.type_name.clone());
    }
    about.push(format!("col {}/{}", col + 1, result.columns.len()));
    lines.push(Line::styled(
        truncate(&about.join(" · "), width),
        theme.muted(),
    ));

    let rows = result.rows.len();
    let profiler = results.profile.as_ref().filter(|p| p.column() == col);
    let Some(profile) = profiler.map(Profiler::profile).filter(|p| p.complete) else {
        // Still counting, or about to start.
        let (frame, done) = profiler.map_or((0, 0), |p| (p.steps(), p.profile().sampled));
        let spinner = SPINNER[usize::try_from(frame).unwrap_or(0) % SPINNER.len()];
        lines.push(Line::from(vec![
            Span::styled(spinner, theme.spinner()),
            Span::styled(
                truncate(
                    &format!(
                        " counting… {} / {}",
                        thousands(usize::try_from(done).unwrap_or(0)),
                        thousands(rows)
                    ),
                    width.saturating_sub(1),
                ),
                theme.dim(),
            ),
        ]));
        return lines;
    };
    let sampled = usize::try_from(profile.sampled).unwrap_or(usize::MAX);
    let n = |v: u64| thousands(usize::try_from(v).unwrap_or(usize::MAX));
    lines.push(Line::from(vec![
        Span::styled("nulls ", theme.dim()),
        Span::styled(
            format!("{} / {}", n(profile.nulls), n(profile.sampled)),
            theme.muted(),
        ),
        Span::styled(" · distinct ", theme.dim()),
        Span::styled(n(profile.distinct), theme.muted()),
    ]));
    if sampled < rows {
        lines.push(Line::styled(
            format!("sampled {} rows", thousands(sampled)),
            theme.dim(),
        ));
    }
    if shows_top_values(profile) {
        lines.push(Line::default());
        lines.push(label(
            "TOP VALUES",
            format!("{} fetched rows", thousands(sampled)),
        ));
        lines.extend(top_lines(profile, width, theme));
    }
    lines
}

/// TOP VALUES is left out when every value counted is different, or there
/// are none.
fn shows_top_values(profile: &ColumnProfile) -> bool {
    let values = profile.sampled - profile.nulls;
    !profile.top.is_empty() && profile.distinct < values
}

/// One line per top value: the value, a bar as long as its share of the
/// most frequent one, and its count.
fn top_lines<'a>(profile: &ColumnProfile, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let most = profile.top.first().map_or(1, |(_, n)| (*n).max(1));
    let count_width = profile
        .top
        .iter()
        .map(|(_, n)| n.to_string().len())
        .max()
        .unwrap_or(1);
    let value_width = width.saturating_sub(BAR_WIDTH + count_width + 3);
    profile
        .top
        .iter()
        .map(|(value, count)| {
            let shown = truncate(&qry_core::grid::display_text(Some(value)), value_width);
            let pad = value_width.saturating_sub(qry_core::grid::width(&shown));
            Line::from(vec![
                Span::styled(shown, theme.record_value()),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(bar(*count, most), theme.gauge(theme.teal)),
                Span::styled(format!(" {count:>count_width$}"), theme.muted()),
            ])
        })
        .collect()
}

/// `count` of `most` as [`BAR_WIDTH`] cells of eighth blocks, padded with
/// spaces so the track shows behind the rest.
pub(crate) fn bar(count: u64, most: u64) -> String {
    let total = BAR_WIDTH as u64 * 8;
    let eighths = (count.min(most) * total).div_ceil(most.max(1));
    let full = usize::try_from(eighths / 8).unwrap_or(BAR_WIDTH);
    let part = EIGHTHS[usize::try_from(eighths % 8).unwrap_or(0)];
    let mut bar = "█".repeat(full);
    bar.push_str(part);
    let used = full + usize::from(!part.is_empty());
    bar.push_str(&" ".repeat(BAR_WIDTH.saturating_sub(used)));
    bar
}

#[cfg(test)]
#[path = "inspector_tests.rs"]
pub(crate) mod inspector_tests;
