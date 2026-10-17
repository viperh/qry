//! Exporting a result to a file (T307).
//!
//! The dialog writes the rows in hand and waits for the answer, as the
//! other dialogs do. Every row of the statement is a job instead: the
//! dialog closes as it is queued, the statement runs again in its session
//! and streams to the file, and the outcome is a toast. Two confirms may
//! come first: running again a statement that is not a plain read, and
//! replacing a file that is there. Cancelling either goes back to the
//! dialog.

use std::sync::Arc;

use qry_core::{
    ExportConfig, ExportOptions,
    jobs::{JobId, JobState, format_rows},
    sql::is_read_only,
};

use super::{dialogs, notices::notify, run};
use crate::{
    app::{
        dialogs::{
            DialogState,
            confirm::Confirm,
            exportform::{ExportChoice, ExportForm, ExportSubject, Scope},
            form::Form,
        },
        effects::Effect,
        event::{DbAnswer, ExportProbe},
        state::{Awaiting, Queued, SessionId, State},
    },
    db::{DbCommand, ExportJob, ExportSource},
    notice::Notice,
};

/// What the active tab holds to export, if it holds rows.
fn subject(state: &State) -> Option<ExportSubject> {
    let session = state.sessions.active();
    let tab = &session.tabs[session.active_tab.min(session.tabs.len() - 1)];
    let result = tab.results.result.as_ref()?;
    if result.columns.is_empty() {
        return None;
    }
    let dialect = session.dialect();
    Some(ExportSubject {
        tab: tab.number,
        table: tab.table.clone(),
        rows: result.rows.len(),
        truncated: result.truncated,
        read_only: is_read_only(dialect, &tab.sql),
        dialect: Some(dialect),
    })
}

/// Whether the active tab has rows to export.
pub(super) fn available(state: &State) -> bool {
    subject(state).is_some()
}

/// Opens the dialog on the active tab's result, with what the session
/// remembers; without a result, says there is nothing to export.
pub(super) fn open(state: &mut State, fx: &mut Vec<Effect>) {
    let Some(subject) = subject(state) else {
        return notify(state, Notice::info("nothing to export"), fx);
    };
    let memory = state.sessions.active().export.clone();
    state.open_dialog(DialogState::Export(ExportForm::new(subject, &memory)));
    state.awaiting = None;
    probe(state, fx);
}

fn form_mut(state: &mut State) -> Option<&mut ExportForm> {
    match state.dialog_mut() {
        Some(DialogState::Export(form)) => Some(form),
        _ => None,
    }
}

/// Asks the runtime about the file the open dialog names, once per path
/// and format. Runs after every event.
pub(super) fn probe(state: &mut State, fx: &mut Vec<Effect>) {
    let Some(form) = form_mut(state) else {
        return;
    };
    if let Some((input, etype)) = form.probe_wanted() {
        form.probe_asked(input.clone(), etype);
        fx.push(Effect::ProbeExportPath { input, etype });
    }
}

/// What the runtime found there.
pub(super) fn probed(state: &mut State, probe: ExportProbe) {
    if let Some(form) = form_mut(state) {
        form.probed(probe);
    }
}

/// The dialog's Enter: the confirms it needs first, then the export.
pub(super) fn submit(state: &mut State, choice: ExportChoice, fx: &mut Vec<Effect>) {
    let Some(form) = form_mut(state) else {
        return;
    };
    let form = form.clone();
    if choice.scope == Scope::All && !form.subject().read_only && !form.rerun_confirmed {
        let confirm = Confirm::rerun_export(form);
        state.open_dialog(DialogState::Confirm(confirm));
        return;
    }
    if let Some(probe) = form.current_probe()
        && probe.exists
        && !probe.is_dir
        && !form.overwrite
    {
        let name = file_name(&probe.resolved);
        let confirm = Confirm::overwrite_export(&name, form);
        state.open_dialog(DialogState::Confirm(confirm));
        return;
    }
    let id = state.sessions.active().id;
    state.sessions.active_mut().export = form.memory();
    let session = state.sessions.active();
    let tab = &session.tabs[session.active_tab.min(session.tabs.len() - 1)];
    let (Some(result), sql) = (tab.results.result.clone(), tab.sql.clone()) else {
        if let Some(form) = form_mut(state) {
            form.set_error(Some("There is no result to export".into()));
        }
        return;
    };
    let source = match choice.scope {
        Scope::Fetched => ExportSource::Fetched(result),
        Scope::All => ExportSource::Rerun(sql.clone()),
    };
    let job = ExportJob {
        job: None,
        source,
        path: choice.path,
        etype: choice.etype,
        options: choice.options,
        overwrite: form.overwrite,
    };
    match choice.scope {
        // The rows in hand: the dialog waits for them to be written.
        Scope::Fetched => {
            fx.push(Effect::Db(id, DbCommand::Export(Box::new(job))));
            if let Some(form) = form_mut(state) {
                form.set_pending(Some("Exporting…"));
            }
            dialogs::start_spinner(state, fx);
            state.awaiting = Some(Awaiting::Export(id));
        }
        // Every row: a job of its own, and the dialog goes.
        Scope::All => {
            state.close_dialog();
            state.awaiting = None;
            let mut queued = Queued::new(sql, None);
            queued.export = Some(Box::new(job));
            run::enqueue_on(state, id, vec![queued], fx);
        }
    }
}

/// An export confirm said yes: back to the dialog, which now may, and
/// submit it again.
pub(super) fn confirmed(
    state: &mut State,
    mut form: ExportForm,
    overwrite: bool,
    fx: &mut Vec<Effect>,
) {
    if overwrite {
        form.overwrite = true;
    } else {
        form.rerun_confirmed = true;
    }
    state.open_dialog(DialogState::Export(form));
    state.awaiting = None;
    dialogs::submit(state, fx);
}

/// `orders.csv`, from the whole path.
fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The palette's `export csv <path>`: the active tab's rows, at once, with
/// no dialog. A file that is there is left alone.
pub(super) fn quick(state: &mut State, config: ExportConfig, fx: &mut Vec<Effect>) {
    let session = state.sessions.active();
    let tab = &session.tabs[session.active_tab.min(session.tabs.len() - 1)];
    let Some(result) = tab.results.result.clone().filter(|r| !r.columns.is_empty()) else {
        return notify(state, Notice::info("nothing to export"), fx);
    };
    let options = ExportOptions {
        dialect: session.dialect(),
        table: tab.table.clone(),
        ..ExportOptions::from_config(&config)
    };
    let job = ExportJob {
        job: None,
        source: ExportSource::Fetched(Arc::clone(&result)),
        path: config.path,
        etype: config.etype,
        options,
        overwrite: false,
    };
    fx.push(Effect::Db(session.id, DbCommand::Export(Box::new(job))));
}

/// `1 row`, `1 000 rows`.
fn rows_text(rows: u64) -> String {
    match rows {
        1 => "1 row".into(),
        n => format!("{} rows", format_rows(n)),
    }
}

/// The toast for a written file, a warning if something was changed to
/// fit.
fn written_notice(rows: u64, path: &str, warning: Option<String>) -> Notice {
    let text = format!("exported {} to {path}", rows_text(rows));
    match warning {
        Some(warning) => Notice::warning(format!("{text} · {warning}")),
        None => Notice::success(text),
    }
}

/// An export's answer from session `id`.
pub(super) fn answer(state: &mut State, id: SessionId, answer: DbAnswer, fx: &mut Vec<Effect>) {
    match answer {
        DbAnswer::ExportProgress { job, rows } => state.jobs.progress(job, rows),
        DbAnswer::Exported {
            job: Some(job),
            rows,
            path,
            warning,
        } => {
            if job_ended(state, id, job, Ok(rows), fx) {
                notify(state, written_notice(rows, &path, warning), fx);
            }
        }
        DbAnswer::ExportFailed {
            job: Some(job),
            rows,
            path,
            message,
        } => {
            let failed = format!(
                "export to {path} failed after {}: {message}",
                rows_text(rows)
            );
            if job_ended(state, id, job, Err(message), fx) {
                notify(state, Notice::error(failed), fx);
            }
        }
        DbAnswer::ExportExists {
            job: Some(job),
            path,
        } => {
            let message = format!("{path} exists; export again to replace it");
            if job_ended(state, id, job, Err(message.clone()), fx) {
                notify(state, Notice::error(message), fx);
            }
        }
        // The rows in hand: the dialog that waits closes, or keeps its
        // fields and says why. The toast says so either way.
        DbAnswer::Exported {
            job: None,
            rows,
            path,
            warning,
        } => {
            dialogs::done(state, Awaiting::Export(id));
            notify(state, written_notice(rows, &path, warning), fx);
        }
        DbAnswer::ExportFailed {
            job: None,
            rows,
            path,
            message,
        } => {
            dialogs::failed(state, Awaiting::Export(id), &message);
            let text = format!("could not export {} to {path}: {message}", rows_text(rows));
            notify(state, Notice::error(text), fx);
        }
        // The file appeared after the dialog looked: ask now.
        DbAnswer::ExportExists { job: None, path } => {
            let waiting = state.awaiting == Some(Awaiting::Export(id));
            match form_mut(state) {
                Some(form) if waiting => {
                    form.set_pending(None);
                    let form = form.clone();
                    state.awaiting = None;
                    let confirm = Confirm::overwrite_export(&file_name(&path), form);
                    state.open_dialog(DialogState::Confirm(confirm));
                }
                _ => notify(
                    state,
                    Notice::warning(format!(
                        "{path} exists; export from the dialog to replace it"
                    )),
                    fx,
                ),
            }
        }
        _ => {}
    }
}

/// Session `id`'s export job `job` ended: done with `rows`, or failed.
/// Returns whether to say so: not for a cancelled job, nor one the
/// session is not running.
fn job_ended(
    state: &mut State,
    id: SessionId,
    job: JobId,
    outcome: Result<u64, String>,
    fx: &mut Vec<Effect>,
) -> bool {
    if !run::export_ended(state, id, job, outcome.is_ok()) {
        return false;
    }
    let cancelled = state.jobs.get(job).map(|j| &j.state) == Some(&JobState::Cancelled);
    let now = state.now;
    match outcome {
        _ if cancelled => {}
        Ok(rows) => state.jobs.finish(job, rows, now),
        Err(message) => state.jobs.fail(job, message, now),
    }
    // Ended first, so the next statement may start.
    run::pump(state, id, fx);
    !cancelled
}
