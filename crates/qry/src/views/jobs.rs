//! The jobs drawer, after tachy's: a purple title with how many run, then
//! the newest jobs, each with its session, statement, state and time.

use qry_core::jobs::{JobState, format_elapsed, format_rows};
use ratatui::{prelude::*, widgets::Paragraph};

use crate::{
    app::state::State,
    keymap::{KeyContext, display_chord},
    theme::Theme,
};

/// Width of the session's name, the state's word and the time.
const SESSION: usize = 12;
const STATE: usize = 10;
const TIME: usize = 9;
/// Room kept for the row count or the error.
const DETAIL: usize = 30;

pub(crate) fn draw(state: &State, theme: &Theme, frame: &mut Frame, area: Rect) {
    let Some(drawer) = state.drawer else {
        return;
    };
    let running = state.jobs.running_count();
    let keys = keys(state);
    let title = match running {
        0 => " JOBS".to_string(),
        n => format!(" JOBS · {n} running"),
    };
    let room = usize::from(area.width);
    let gap = room.saturating_sub(title.chars().count() + keys.chars().count() + 1);
    let mut lines = vec![Line::from(vec![
        Span::styled(title, theme.status_jobs()),
        Span::styled(" ".repeat(gap), theme.status_line()),
        Span::styled(keys, theme.status_dim()),
        Span::styled(" ", theme.status_line()),
    ])];

    let rows = usize::from(area.height.saturating_sub(1));
    if state.jobs.is_empty() {
        lines.push(Line::styled("   no jobs yet", theme.dim()));
    }
    // The selected job stays in view.
    let first = drawer.selected.saturating_sub(rows.saturating_sub(1));
    for (i, job) in state.jobs.iter().rev().enumerate().skip(first).take(rows) {
        let (glyph_style, glyph) = theme.job_state(job.kind, &job.state);
        let session = state
            .sessions
            .get(job.session)
            .map_or("closed", |s| s.title.as_str());
        let (word, detail) = match &job.state {
            JobState::Queued => ("queued", String::new()),
            JobState::Running { rows } => ("running", rows_text(*rows)),
            JobState::Done { rows } => ("done", rows_text(*rows)),
            JobState::Failed(message) => ("failed", message.clone()),
            JobState::Cancelled => ("cancelled", String::new()),
        };
        let time = job
            .elapsed(state.now)
            .map(format_elapsed)
            .unwrap_or_default();
        // One line of the statement, as wide as leaves room for a detail.
        let fixed = 3 + SESSION + 2 + 2 + STATE + TIME;
        let sql_room = room.saturating_sub(fixed + DETAIL).clamp(10, 60);
        let sql: String = job.sql.split_whitespace().collect::<Vec<_>>().join(" ");
        let sql = cut(&sql, sql_room);
        let detail = cut(&detail, room.saturating_sub(fixed + sql_room));
        let base = if i == drawer.selected {
            theme.row_selected()
        } else {
            theme.base()
        };
        lines.push(
            Line::from(vec![
                Span::styled(format!(" {glyph} "), glyph_style),
                Span::raw(format!("{:<SESSION$}  ", cut(session, SESSION))),
                Span::raw(format!("{sql:<sql_room$}  ")),
                Span::styled(format!("{word:<STATE$}"), glyph_style),
                Span::raw(format!("{time:<TIME$}")),
                Span::styled(detail, theme.muted()),
            ])
            .style(base),
        );
    }
    frame.render_widget(Paragraph::new(lines).style(theme.base()), area);
}

/// `38 000 rows`, or nothing for none.
fn rows_text(rows: u64) -> String {
    match rows {
        0 => String::new(),
        1 => "1 row".into(),
        n => format!("{} rows", format_rows(n)),
    }
}

/// `text` cut to `width` characters, ending in `…` when cut.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// `↑↓ select  c cancel  Enter show  J close`, from the live keymap.
fn keys(state: &State) -> String {
    use crate::action::Action;
    let keymap = &state.config.keymap;
    let key = |action| {
        let chords = keymap.chords_for(KeyContext::Jobs, &action);
        (!chords.is_empty()).then(|| display_chord(&chords))
    };
    [
        (Action::Down, "select"),
        (Action::CancelQuery, "cancel"),
        (Action::Open, "show"),
        (Action::ToggleJobs, "close"),
    ]
    .into_iter()
    .filter_map(|(action, label)| Some(format!("{} {label}", key(action)?)))
    .collect::<Vec<_>>()
    .join("  ")
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod jobs_tests;
