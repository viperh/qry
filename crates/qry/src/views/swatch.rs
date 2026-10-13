//! The theme swatch: every style helper, drawn on a sample of its own
//! name. Test-only; its styled snapshots put every palette or helper
//! change in front of the reviewer, in each colour mode.

use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};

use crate::{
    connections::Driver,
    theme::{JobKind, JobState, Pill, SessionStatus, Theme, ToastLevel},
};

/// A sample in one style.
fn one(text: &'static str, style: Style) -> Line<'static> {
    Line::from(Span::styled(text, style))
}

/// Several samples of one helper, a space apart.
fn many(samples: Vec<(&'static str, Style)>) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (text, style)) in samples.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// One line per helper, in the order `theme.rs` defines them. A helper
/// that takes an argument gets one sample per value that matters.
fn samples(t: &Theme) -> Vec<Line<'static>> {
    let dot = |status: SessionStatus| t.session_dot(&status);
    let badge = |driver| t.driver_badge(driver);
    let state = |kind, state: JobState| t.job_state(kind, &state);
    vec![
        one("base", t.base()),
        one("surface", t.surface()),
        one("muted", t.muted()),
        one("dim", t.dim()),
        one("border", t.border()),
        one("border_strong", t.border_strong()),
        one("focused_border", t.focused_border()),
        one("spinner", t.spinner()),
        one("value_ok", t.value_ok()),
        one("value_error", t.value_error()),
        one("app_name", t.app_name()),
        one("tab", t.tab()),
        one("tab_active", t.tab_active()),
        one("tab_running", t.tab_running()),
        many(vec![
            ("NORMAL", t.mode_pill(Pill::Normal)),
            ("EDIT", t.mode_pill(Pill::Edit)),
            ("COMMAND", t.mode_pill(Pill::Command)),
        ]),
        one("pane_title", t.pane_title()),
        one("pane_title_focused", t.pane_title_focused()),
        one("divider", t.divider()),
        one("divider_focused", t.divider_focused()),
        one("tree_connection", t.tree_connection()),
        one("tree_conn_selected", t.tree_connection_selected()),
        one("tree_object", t.tree_object()),
        one("tree_kind", t.tree_kind()),
        {
            let (open, closed, failed) = (
                dot(SessionStatus::Open {
                    label: String::new(),
                }),
                dot(SessionStatus::Closed),
                dot(SessionStatus::Failed(String::new())),
            );
            many(vec![
                ("session", t.base()),
                (open.1, open.0),
                (closed.1, closed.0),
                (failed.1, failed.0),
            ])
        },
        many(
            [
                Driver::Postgres,
                Driver::Mysql,
                Driver::Mariadb,
                Driver::Oracle,
                Driver::Sqlite,
            ]
            .map(|d| {
                let (style, label) = badge(d);
                (label, style)
            })
            .to_vec(),
        ),
        one("header", t.header()),
        one("header_type", t.header_type()),
        one("header_active", t.header_active()),
        one("sort_indicator", t.sort_indicator()),
        one("overflow_hint", t.overflow_hint()),
        one("gutter", t.gutter()),
        one("gutter_selected", t.gutter_selected()),
        one("column_divider", t.column_divider()),
        one("frozen_divider", t.frozen_divider()),
        one("row(0)", t.row(0)),
        one("row(1)", t.row(1)),
        one("row_selected", t.row_selected()),
        one("cursor_cell", t.cursor_cell()),
        one("null_value", t.null_value()),
        one("match_highlight", t.match_highlight()),
        one("inspector_label", t.inspector_label()),
        one("inspector_focused", t.inspector_label_focused()),
        one("record_field", t.record_field()),
        one("record_value", t.record_value()),
        one("value_changed", t.value_changed()),
        one("status_line", t.status_line()),
        one("status_session", t.status_session()),
        one("status_jobs", t.status_jobs()),
        one("status_warning", t.status_warning()),
        one("status_ok", t.status_ok()),
        one("status_dim", t.status_dim()),
        many(vec![("gauge", t.base()), ("████░░░░", t.gauge(t.teal))]),
        one("hint", t.hint()),
        one("hint_key", t.hint_key()),
        many(vec![
            ("info", t.toast(ToastLevel::Info)),
            ("ok", t.toast(ToastLevel::Success)),
            ("warn", t.toast(ToastLevel::Warning)),
            ("error", t.toast(ToastLevel::Error)),
        ]),
        one("sql_keyword", t.sql_keyword()),
        one("sql_operator", t.sql_operator()),
        one("sql_string", t.sql_string()),
        one("sql_number", t.sql_number()),
        one("sql_comment", t.sql_comment()),
        one("sql_identifier", t.sql_identifier()),
        one("sql_punct", t.sql_punct()),
        one("sql_quoted_ident", t.sql_quoted_ident()),
        one("sql_error", t.sql_error()),
        one("editor_cursor_line", t.editor_cursor_line()),
        one("editor_cursor", t.editor_cursor()),
        one("editor_selection", t.editor_selection()),
        one("line_number", t.line_number()),
        one("line_number_current", t.line_number_current()),
        one("dialog", t.dialog()),
        one("dialog_border", t.dialog_border(t.teal)),
        one("dialog_title", t.dialog_title(t.teal)),
        one("dialog_heading", t.dialog_heading(t.teal)),
        one("field", t.field()),
        one("field_focused", t.field_focused()),
        one("field_placeholder", t.field_placeholder()),
        one("field_border", t.field_border()),
        one("field_border_focus", t.field_border_focused()),
        one("button", t.button()),
        one("button_primary", t.button_primary()),
        one("button_danger", t.button_danger()),
        one("palette_border", t.palette_border()),
        one("palette_selected", t.palette_item_selected()),
        one("palette_match", t.palette_match()),
        one("backdrop", t.backdrop()),
        many(vec![
            ("query", t.job(JobKind::Query)),
            ("fetch", t.job(JobKind::FetchMore)),
            ("exp", t.job(JobKind::Export)),
            ("sch", t.job(JobKind::Schema)),
        ]),
        {
            let states = [
                state(JobKind::Query, JobState::Running { rows: 0 }),
                state(JobKind::Query, JobState::Done { rows: 0 }),
                state(JobKind::Query, JobState::Failed(String::new())),
                state(JobKind::Query, JobState::Cancelled),
                state(JobKind::Query, JobState::Queued),
            ];
            let mut samples = vec![("jobs", t.base())];
            samples.extend(states.map(|(style, glyph)| (glyph, style)));
            many(samples)
        },
    ]
}

/// Draws the samples in columns, top to bottom then left to right, on the
/// app background.
pub(super) fn draw(theme: &Theme, frame: &mut Frame, area: Rect) {
    frame.render_widget(Block::new().style(theme.base()), area);
    let lines = samples(theme);
    let rows = usize::from(area.height.max(1));
    let columns = lines.len().div_ceil(rows);
    let width = area.width / u16::try_from(columns.max(1)).unwrap_or(u16::MAX);
    for (i, line) in lines.into_iter().enumerate() {
        let (column, row) = (i / rows, i % rows);
        let cell = Rect {
            x: area.x + width * u16::try_from(column).unwrap_or(u16::MAX),
            y: area.y + u16::try_from(row).unwrap_or(u16::MAX),
            // One blank column between columns.
            width: width.saturating_sub(1),
            height: 1,
        }
        .intersection(area);
        frame.render_widget(Paragraph::new(line), cell);
    }
}

#[cfg(test)]
#[path = "swatch_tests.rs"]
mod swatch_tests;
