//! The results grid.

use qry_core::{
    grid::{self, COLUMN_SPACING, Order},
    jobs::{JobState, format_rows},
};
use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    action::Action,
    app::state::{Focus, GridSearch, State, TabError},
    db::took,
    keymap::{KeyContext, display_chord},
    theme::Theme,
    views::{overlay::SPINNER, status_line::thousands},
};

/// `204 rows` for the title row, once there is a result; nothing when
/// there are tabs, which say it themselves.
pub(crate) fn summary(state: &State) -> String {
    if let Some(running) = running(state) {
        return running;
    }
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
    let results = state.results();
    let mut summary = match result.rows.len() {
        1 => "1 row".to_string(),
        n => format!("{} rows", thousands(n)),
    };
    if result.truncated {
        summary.push_str(" (first rows only)");
    }
    if !result.elapsed.is_zero() {
        summary.push_str(&format!(" · {}", took(result.elapsed)));
    }
    if !results.sort.is_empty() {
        let keys: Vec<String> = results
            .sort
            .iter()
            .filter_map(|&(c, order)| {
                let arrow = if order == Order::Asc { "▲" } else { "▼" };
                Some(format!("{} {arrow}", result.columns.get(c)?.name))
            })
            .collect();
        summary.push_str(&format!(" · sorted by {}", keys.join(", ")));
        if result.truncated {
            summary.push_str(" (fetched rows only)");
        }
    }
    summary
}

/// `⠹ running · 2.4 s · 12 000 rows` while the active session runs a
/// statement.
pub(crate) fn running(state: &State) -> Option<String> {
    let job = state.jobs.running(state.sessions.active().id)?;
    let frame = (state.now.as_millis() / 100) as usize % SPINNER.len();
    // An export says how far it has got before how long it has taken.
    if job.kind == qry_core::jobs::JobKind::Export {
        let rows = match job.state {
            JobState::Running { rows } => rows,
            _ => 0,
        };
        let mut text = format!("{} exporting · {} rows", SPINNER[frame], format_rows(rows));
        if let Some(elapsed) = job.elapsed(state.now) {
            text.push_str(&format!(" · {}", took(elapsed)));
        }
        return Some(text);
    }
    let mut text = format!("{} running", SPINNER[frame]);
    if let Some(elapsed) = job.elapsed(state.now) {
        text.push_str(&format!(" · {}", took(elapsed)));
    }
    if let JobState::Running { rows } = job.state
        && rows > 0
    {
        text.push_str(&format!(" · {} rows", format_rows(rows)));
    }
    Some(text)
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
        let mut text = match result.affected {
            Some(1) => "✓ 1 row affected".to_string(),
            Some(n) => format!(
                "✓ {} rows affected",
                thousands(usize::try_from(n).unwrap_or(usize::MAX))
            ),
            None => "✓ Statement executed".to_string(),
        };
        if !result.elapsed.is_zero() {
            text.push_str(&format!(" · {}", took(result.elapsed)));
        }
        frame.render_widget(
            Paragraph::new(text).style(theme.value_ok()).centered(),
            area,
        );
        return;
    }

    // The search line, while there is a search, takes the bottom line.
    let mut area = area;
    if let Some(search) = &results.search {
        let line = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        area.height = area.height.saturating_sub(1);
        let at = (results.data_row(results.row), results.col);
        draw_search(search, at, theme, frame, line);
    }

    // A result cut short says so on its last line: dim at the row limit,
    // coral when an error stopped it.
    let stopped = &session.tabs[session.active_tab.min(session.tabs.len() - 1)].stopped;
    if (result.truncated || stopped.is_some()) && area.height > 2 {
        let line = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        area.height -= 1;
        let (text, style) = match stopped {
            Some(error) => (
                format!(
                    " stopped after {} rows: {error}",
                    thousands(result.rows.len())
                ),
                theme.value_error(),
            ),
            None => (truncated_footer(state, result.rows.len()), theme.dim()),
        };
        frame.render_widget(Paragraph::new(text).style(style), line);
    }

    let total_rows = result.rows.len();
    let typed = result.columns.iter().any(|c| !c.type_name.is_empty());
    let header_rows: u16 = if typed { 2 } else { 1 };
    // The gutter: right-aligned row numbers, a space and a divider.
    let digits = total_rows.max(1).to_string().len();
    let gutter = u16::try_from(digits + 2)
        .unwrap_or(u16::MAX)
        .min(area.width);
    let inner = Rect::new(
        area.x + gutter,
        area.y,
        area.width.saturating_sub(gutter),
        area.height,
    );

    let (row, col) = (results.row, results.col);
    let page_rows = usize::from(inner.height.saturating_sub(header_rows)).max(1);
    let row_offset = scroll_rows(results.row_offset.get(), row, page_rows, total_rows);
    let col_offset = scroll_columns(
        &results.widths,
        results.col_offset.get(),
        col,
        inner.width,
        results.frozen,
    );
    results.page_rows.set(page_rows);
    results.row_offset.set(row_offset);
    results.col_offset.set(col_offset);
    let mut visible =
        grid::visible_columns(&results.widths, col_offset, results.frozen, inner.width);
    // Columns off the right edge are counted at the end of the header, in
    // room taken from the columns.
    let more = |visible: &[(usize, u16)]| {
        visible
            .last()
            .map_or(0, |&(last, _)| result.columns.len() - 1 - last)
    };
    // A sliver of a column (`…`) says nothing; drop it unless selected.
    let sliver = |visible: &mut Vec<(usize, u16)>| {
        if let Some(&(c, w)) = visible.last()
            && visible.len() > 1
            && c != col
            && w < results.widths[c].min(3)
        {
            visible.pop();
        }
    };
    sliver(&mut visible);
    let mut hint = String::new();
    if more(&visible) > 0 {
        let n = more(&visible);
        let len = u16::try_from(grid::width(&overflow_hint(n + 1))).unwrap_or(u16::MAX);
        let fits = len < inner.width;
        if fits {
            visible = grid::visible_columns(
                &results.widths,
                col_offset,
                results.frozen,
                inner.width - len,
            );
        }
        // The selected column always shows, even if the hint must go.
        if fits && visible.iter().any(|&(c, _)| c == col) {
            sliver(&mut visible);
            hint = overflow_hint(more(&visible));
        } else {
            visible =
                grid::visible_columns(&results.widths, col_offset, results.frozen, inner.width);
        }
    }

    let buf = frame.buffer_mut();
    // The header: names (with sort marks) and, under them, the types.
    let header_area = Rect::new(area.x, area.y, area.width, header_rows.min(area.height));
    buf.set_style(header_area, theme.header());
    if gutter > 1 {
        for y in header_area.top()..header_area.bottom() {
            buf.set_string(area.x + gutter - 1, y, "│", theme.column_divider());
        }
    }
    let mut x = inner.x;
    for (i, &(c, w)) in visible.iter().enumerate() {
        if i > 0 {
            divider(buf, theme, results.frozen && i == 1, x, header_area);
            x += COLUMN_SPACING;
        }
        let column = &result.columns[c];
        let name_style = if c == col {
            theme.header_active()
        } else {
            theme.header()
        };
        // Postgres's name for a column without one.
        let name = if column.name.is_empty() {
            "?column?"
        } else {
            column.name.as_str()
        };
        let mut spans = vec![Span::styled(name, name_style)];
        if let Some(at) = results.sort.iter().position(|(s, _)| *s == c) {
            let arrow = match results.sort[at].1 {
                Order::Asc => "▲",
                Order::Desc => "▼",
            };
            let mark = if results.sort.len() > 1 {
                format!(" {arrow}{}", at + 1)
            } else {
                format!(" {arrow}")
            };
            spans.push(Span::styled(mark, theme.sort_indicator()));
        }
        let line = fit(spans, usize::from(w), false);
        buf.set_line(x, area.y, &line, w);
        if typed && area.height > 1 {
            buf.set_stringn(
                x,
                area.y + 1,
                &column.type_name,
                usize::from(w),
                theme.header_type(),
            );
        }
        x += w;
    }
    if !hint.is_empty() {
        let len = u16::try_from(grid::width(&hint)).unwrap_or(u16::MAX);
        buf.set_string(area.right() - len, area.y, hint, theme.overflow_hint());
    }

    // The rows.
    let matches = results.search.as_ref().map(|s| &s.search.matches);
    let end = (row_offset + page_rows).min(total_rows);
    let top = area.y + header_rows;
    if total_rows == 0 && inner.height > header_rows {
        buf.set_string(inner.x, top, "(no rows)", theme.muted());
    }
    for (grid_row, y) in (row_offset..end).zip(top..area.bottom()) {
        let data_row = results.data_row(grid_row);
        let selected = grid_row == row;
        let base = if selected {
            theme.row_selected()
        } else {
            theme.row(grid_row)
        };
        buf.set_style(Rect::new(area.x, y, area.width, 1), base);
        let number = format!("{:>digits$} ", grid_row + 1);
        let number_style = if selected {
            theme.gutter_selected()
        } else {
            theme.gutter()
        };
        buf.set_stringn(area.x, y, number, usize::from(gutter), number_style);
        if gutter > 1 {
            buf.set_string(area.x + gutter - 1, y, "│", theme.column_divider());
        }

        let mut x = inner.x;
        for (i, &(c, w)) in visible.iter().enumerate() {
            if i > 0 {
                divider(
                    buf,
                    theme,
                    results.frozen && i == 1,
                    x,
                    Rect::new(x, y, 1, 1),
                );
                x += COLUMN_SPACING;
            }
            let cell = result.rows[data_row].get(c).and_then(Option::as_deref);
            let mut style = base;
            if matches.is_some_and(|m| m.binary_search(&(data_row, c)).is_ok()) {
                style = style.patch(theme.match_highlight());
            }
            if selected && c == col && focused {
                style = theme.cursor_cell();
            }
            buf.set_style(Rect::new(x, y, w, 1), style);
            let text = grid::display_text(cell);
            let spans = if cell.is_none() {
                vec![Span::styled(
                    text.into_owned(),
                    style.patch(theme.null_value()),
                )]
            } else {
                glyph_spans(&text, style, style.patch(theme.dim()))
            };
            let right = result.columns[c].kind.is_numeric();
            buf.set_line(x, y, &fit(spans, usize::from(w), right), w);
            x += w;
        }
    }
}

/// ` … more rows — showing the first 1 000 · L more · A all`: `L` only
/// while the result's stream is kept, each key as the keymap has it.
fn truncated_footer(state: &State, rows: usize) -> String {
    let keymap = &state.config.keymap;
    let mut text = format!(" … more rows — showing the first {}", thousands(rows));
    let more = keymap.chords_for(KeyContext::Results, &Action::FetchMore);
    if state.sessions.active().can_load_more() && !more.is_empty() {
        text.push_str(&format!(" · {} more", display_chord(&more)));
    }
    let all = keymap.chords_for(KeyContext::Results, &Action::FetchAll);
    if !all.is_empty() {
        text.push_str(&format!(" · {} all", display_chord(&all)));
    }
    text
}

/// ` → 3 more cols `, at the end of the header.
fn overflow_hint(more: usize) -> String {
    format!(" → {more} more col{} ", if more == 1 { "" } else { "s" })
}

/// The divider before a column: the frozen one after the first column.
fn divider(buf: &mut Buffer, theme: &Theme, frozen: bool, x: u16, area: Rect) {
    let style = if frozen {
        theme.frozen_divider()
    } else {
        theme.column_divider()
    };
    for y in area.top()..area.bottom() {
        buf.set_string(x, y, if frozen { "┃" } else { "│" }, style);
    }
}

/// `text` in runs, the control glyphs in `glyph` and the rest in `plain`.
fn glyph_spans(text: &str, plain: Style, glyph: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut in_glyph = false;
    for c in text.chars() {
        let is_glyph = grid::is_control_glyph(c);
        if is_glyph != in_glyph && !run.is_empty() {
            let style = if in_glyph { glyph } else { plain };
            spans.push(Span::styled(std::mem::take(&mut run), style));
        }
        in_glyph = is_glyph;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if in_glyph { glyph } else { plain }));
    }
    spans
}

/// `spans` cut to `width` cells, ending in `…` when cut, and padded on the
/// left when `right` (numbers line up on the right).
fn fit(spans: Vec<Span<'_>>, width: usize, right: bool) -> Line<'_> {
    let total: usize = spans.iter().map(|s| grid::width(&s.content)).sum();
    if total <= width {
        let mut line = Line::from(spans);
        if right {
            line.spans.insert(0, Span::raw(" ".repeat(width - total)));
        }
        return line;
    }
    let mut out = Vec::new();
    let mut used = 0;
    let room = width.saturating_sub(1);
    let mut last = Style::default();
    'spans: for span in spans {
        let mut kept = String::new();
        for c in span.content.chars() {
            let w = grid::width(c.encode_utf8(&mut [0; 4]));
            if used + w > room {
                out.push(Span::styled(kept, span.style));
                last = span.style;
                break 'spans;
            }
            used += w;
            kept.push(c);
        }
        last = span.style;
        out.push(Span::styled(kept, span.style));
    }
    if width > 0 {
        out.push(Span::styled("…", last));
    }
    Line::from(out)
}

/// The search line: `/needle`, and `match 3 / 17` when the selected cell
/// `at` is one, else how many there are so far.
fn draw_search(
    search: &GridSearch,
    at: (usize, usize),
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let matches = &search.search.matches;
    let found = match (matches.binary_search(&at), matches.len()) {
        (Ok(i), n) => format!("match {} / {}", thousands(i + 1), thousands(n)),
        (_, 0) if search.input.is_empty() => String::new(),
        (_, 0) => "no matches".to_string(),
        (_, 1) => "1 match".to_string(),
        (_, n) => format!("{} matches", thousands(n)),
    };
    let mut spans = vec![
        Span::styled("/", theme.hint_key()),
        Span::styled(search.input.clone(), theme.base()),
    ];
    if search.typing {
        spans.push(Span::styled(" ", theme.editor_cursor()));
    }
    spans.push(Span::styled(format!("  {found}"), theme.muted()));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.status_line()),
        area,
    );
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
/// column fits entirely within `available`. With the first column frozen,
/// it always shows and the rest scroll in the room left beside it.
fn scroll_columns(
    widths: &[u16],
    offset: usize,
    selected: usize,
    available: u16,
    frozen: bool,
) -> usize {
    if frozen && !widths.is_empty() {
        if selected == 0 {
            return offset.max(1);
        }
        let room = available.saturating_sub(widths[0] + COLUMN_SPACING);
        return 1 + scroll_columns(&widths[1..], offset.max(1) - 1, selected - 1, room, false);
    }
    let mut offset = offset.min(selected);
    while offset < selected && total_width(&widths[offset..=selected]) > usize::from(available) {
        offset += 1;
    }
    offset
}

#[cfg(test)]
#[path = "results_tests.rs"]
pub(crate) mod results_tests;
