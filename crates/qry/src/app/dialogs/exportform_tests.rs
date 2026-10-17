//! Unit tests for `exportform.rs`, and the export flow through the app
//! harness: the dialog, its confirms, the job and the answers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::KeyCode;
use qry_core::{
    QueryResult,
    jobs::{JobKind, JobState},
    testing::typed_result,
};

use super::*;
use crate::{
    app::{
        dialogs::{
            DialogState,
            form_tests::{key, press, render, type_str},
        },
        effects::Effect,
        event::DbAnswer,
        state::Focus,
    },
    config::Config,
    db::{DbCommand, ExportJob, ExportSource},
    testing::{AppHarness, TEST_HOME, buffer_lines, styled},
};

/// Moves the focus to the row labelled `label`.
fn focus(form: &mut ExportForm, label: &str) {
    let at = form.labels().iter().position(|l| *l == label).unwrap();
    form.set_focus(at);
}

/// Cycles the row labelled `label` with the Right key until `option` is
/// chosen.
fn pick(form: &mut ExportForm, label: &str, options: &[&str], option: &str) {
    focus(form, label);
    let at = form.focus();
    let target = options.iter().position(|o| *o == option).unwrap();
    while form.selected(at) != target {
        press(form, key(KeyCode::Right));
    }
}

fn choice(form: &ExportForm) -> ExportChoice {
    match form.submit() {
        Ok(Submission::Export(choice)) => *choice,
        other => panic!("expected an export, got {other:?}"),
    }
}

fn subject() -> ExportSubject {
    ExportSubject {
        tab: 2,
        table: Some("orders".into()),
        rows: 7,
        truncated: false,
        read_only: true,
        dialect: Some(Dialect::Postgres),
    }
}

fn form() -> ExportForm {
    ExportForm::new(subject(), &ExportMemory::default())
}

fn probe(form: &ExportForm, exists: bool) -> ExportProbe {
    ExportProbe {
        input: form.path().to_string(),
        etype: form.etype(),
        resolved: format!("/home/ann/{}", form.etype().with_extension(form.path())),
        exists,
        is_dir: false,
    }
}

#[test]
fn export_type_labels_line_up_with_core() {
    let core: Vec<String> = ExportType::ALL.iter().map(ToString::to_string).collect();
    assert_eq!(core, FORMATS);
}

#[test]
fn a_path_is_required() {
    let mut form = form();
    assert_eq!(form.submit().unwrap_err(), "Path is required");
    type_str(&mut form, "   ");
    assert_eq!(form.submit().unwrap_err(), "Path is required");
}

#[test]
fn each_separator_choice_has_its_character() {
    for (option, expected) in [
        (", comma", ','),
        ("; semicolon", ';'),
        ("| pipe", '|'),
        ("tab", '\t'),
    ] {
        let mut form = form();
        type_str(&mut form, "rows.csv");
        pick(&mut form, "Separator", &SEPARATORS, option);
        assert_eq!(choice(&form).options.separator, expected, "{option}");
        assert!(!form.labels().contains(&"Custom"), "{option} added a row");
    }
}

#[test]
fn other_reveals_a_custom_field_that_takes_one_character() {
    let mut form = form();
    type_str(&mut form, "rows.txt");
    pick(&mut form, "Separator", &SEPARATORS, "other");
    assert!(form.labels().contains(&"Custom"));

    assert_eq!(form.submit().unwrap_err(), "Custom separator is required");
    focus(&mut form, "Custom");
    type_str(&mut form, "::");
    assert_eq!(
        form.submit().unwrap_err(),
        "A separator is a single character"
    );
    press(&mut form, key(KeyCode::Backspace));
    assert_eq!(choice(&form).options.separator, ':');

    // Choosing another separator takes the row away, and the focus stays
    // on the separator.
    pick(&mut form, "Separator", &SEPARATORS, "| pipe");
    assert!(!form.labels().contains(&"Custom"));
    assert_eq!(form.labels()[form.focus()], "Separator");
}

#[test]
fn fields_change_with_format() {
    let expected: [(&str, &[&str]); 8] = [
        (
            "CSV",
            &[
                "Path",
                "Format",
                "Rows",
                "Separator",
                "Line ends",
                "Header",
                "NULL as",
                "Encoding",
            ],
        ),
        (
            "TSV",
            &["Path", "Format", "Rows", "Header", "NULL as", "Encoding"],
        ),
        ("JSON", &["Path", "Format", "Rows", "Encoding"]),
        ("JSON lines", &["Path", "Format", "Rows", "Encoding"]),
        ("Excel", &["Path", "Format", "Rows", "Header", "NULL as"]),
        (
            "Markdown",
            &["Path", "Format", "Rows", "Header", "NULL as", "Encoding"],
        ),
        (
            "SQL INSERT",
            &["Path", "Format", "Rows", "Header", "Table", "Encoding"],
        ),
        (
            "Text",
            &[
                "Path",
                "Format",
                "Rows",
                "Separator",
                "Header",
                "NULL as",
                "Encoding",
            ],
        ),
    ];
    let mut form = form();
    type_str(&mut form, "rows");
    for (format, labels) in expected {
        pick(&mut form, "Format", &FORMATS, format);
        assert_eq!(form.labels(), labels, "{format}");
        let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
        assert_eq!(
            shown.contains("Separator"),
            labels.contains(&"Separator"),
            "{format}: {shown}"
        );
    }
    // The table defaults to the one the result read.
    pick(&mut form, "Format", &FORMATS, "SQL INSERT");
    assert_eq!(choice(&form).options.table.as_deref(), Some("orders"));
}

#[test]
fn options_follow_the_rows() {
    let mut form = form();
    type_str(&mut form, "rows");
    pick(&mut form, "Line ends", &LINE_ENDS, "CRLF");
    pick(&mut form, "Header", &YES_NO, "no");
    pick(&mut form, "Encoding", &ENCODINGS, "UTF-8 with BOM");
    focus(&mut form, "NULL as");
    type_str(&mut form, "NULL");
    let options = choice(&form).options;
    assert!(options.crlf && !options.header && options.bom);
    assert_eq!(options.null_as, "NULL");
    assert_eq!(options.dialect, Dialect::Postgres);
}

#[test]
fn preview_shows_resolved_path_and_exists_warning() {
    let mut form = form();
    let shown = buffer_lines(&render(&form, 80, 24)).join("\n");
    assert!(shown.contains("→ type where the file goes"), "{shown}");

    type_str(&mut form, "~/exports/orders");
    assert_eq!(
        form.probe_wanted(),
        Some(("~/exports/orders".to_string(), ExportType::Csv))
    );
    form.probe_asked("~/exports/orders".into(), ExportType::Csv);
    assert_eq!(form.probe_wanted(), None, "asked once");
    let p = probe(&form, true);
    form.probed(p);
    let shown = buffer_lines(&render(&form, 100, 30)).join("\n");
    assert!(
        shown.contains("→ /home/ann/~/exports/orders.csv · exists, will ask before overwriting"),
        "{shown}"
    );

    // A new format is a new file, looked up again.
    pick(&mut form, "Format", &FORMATS, "JSON");
    assert!(form.current_probe().is_none());
    assert!(form.probe_wanted().is_some());
    let shown = buffer_lines(&render(&form, 100, 30)).join("\n");
    assert!(!shown.contains("exists"), "{shown}");

    // A late answer about another path is not kept.
    let mut stale = probe(&form, true);
    stale.input = "old".into();
    form.probed(stale);
    assert!(form.current_probe().is_none());
}

#[test]
fn the_separator_survives_a_trip_through_excel() {
    let mut form = form();
    type_str(&mut form, "rows.txt");
    pick(&mut form, "Separator", &SEPARATORS, "; semicolon");
    pick(&mut form, "Format", &FORMATS, "Excel");
    pick(&mut form, "Format", &FORMATS, "CSV");
    focus(&mut form, "Separator");
    assert_eq!(SEPARATORS[form.selected(form.focus())], "; semicolon");
    assert_eq!(choice(&form).options.separator, ';');
}

#[test]
fn a_windows_path_can_be_typed() {
    let mut form = form();
    type_str(&mut form, r"C:\data\rows.csv");
    assert_eq!(form.path(), r"C:\data\rows.csv");
    assert_eq!(form.memory().dir.as_deref(), Some(r"C:\data\"));
}

// --- through the app ------------------------------------------------------

/// Seven orders, read by `sql`, in result tab 1 of the scratch session,
/// with the results focused.
fn harness_with(result: QueryResult, sql: &str) -> AppHarness {
    let mut h = AppHarness::new(Config::embedded());
    h.db(DbAnswer::QueryDone(Arc::new(result)));
    let tab = h.state_mut().sessions.active_mut().tabs.last_mut().unwrap();
    tab.sql = sql.into();
    tab.table = Some("orders".into());
    h.state_mut().focus = Focus::Results;
    h.take_effects();
    h
}

fn orders() -> QueryResult {
    let rows: Vec<Vec<Option<String>>> = (1..=7)
        .map(|i| {
            vec![
                Some(i.to_string()),
                Some(format!("customer {i}")),
                Some(format!("{i}0.50")),
            ]
        })
        .collect();
    let mut result = typed_result(
        &[("id", "int4"), ("customer", "text"), ("total", "numeric")],
        &[],
    );
    result.rows = rows;
    result
}

fn harness() -> AppHarness {
    harness_with(orders(), "SELECT * FROM orders")
}

/// The one export command among `effects`.
fn export_job(effects: &[Effect]) -> ExportJob {
    let jobs: Vec<&ExportJob> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Db(_, DbCommand::Export(job)) => Some(&**job),
            _ => None,
        })
        .collect();
    let [job] = jobs.as_slice() else {
        panic!("expected one export, got {effects:?}");
    };
    (*job).clone()
}

/// The newest notice's text.
fn toast(h: &mut AppHarness) -> String {
    h.state()
        .messages
        .newest_first()
        .next()
        .map(|(_, n)| n.text.clone())
        .unwrap_or_default()
}

#[test]
fn nothing_to_export_without_a_result() {
    let mut h = AppHarness::new(Config::embedded());
    h.state_mut().focus = Focus::Results;
    h.keys("ctrl-e");
    assert!(h.state().dialog().is_none());
    assert_eq!(toast(&mut h), "nothing to export");
    // And the palette leaves the command out.
    h.keys(":");
    h.type_str("Export result");
    let shown = h.render(100, 30);
    assert!(!shown.contains("Export result…"), "{shown}");

    let mut h = harness();
    h.keys(":");
    h.type_str("Export result");
    assert!(h.render(100, 30).contains("Export result…"));
    h.keys("enter");
    assert_eq!(h.dialog_title(), Some("Export"));
}

#[test]
fn a_failed_export_keeps_the_modal_and_the_path() {
    let mut h = harness();
    h.keys("ctrl-e")
        .type_str("no/such/dir/rows.csv")
        .keys("enter");

    let job = export_job(&h.take_effects());
    assert_eq!(job.path, "no/such/dir/rows.csv");
    assert_eq!(job.job, None);
    assert!(matches!(&job.source, ExportSource::Fetched(r) if r.rows.len() == 7));
    assert_eq!(h.dialog_title(), Some("Export"), "still open");
    assert!(h.render(80, 24).contains("Exporting"));

    h.db(DbAnswer::ExportFailed {
        job: None,
        rows: 7,
        path: "no/such/dir/rows.csv".into(),
        message: "could not write no/such/dir/rows.csv: the folder no/such/dir does not exist"
            .into(),
    });
    let text = h.render(100, 30);
    assert!(text.contains("could not write"), "{text}");
    assert!(text.contains("no/such/dir/rows.csv"), "{text}");
    assert_eq!(
        toast(&mut h),
        "could not export 7 rows to no/such/dir/rows.csv: could not write no/such/dir/rows.csv: \
         the folder no/such/dir does not exist"
    );
}

#[test]
fn a_written_export_closes_the_modal() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv").keys("enter");
    export_job(&h.take_effects());

    h.db(DbAnswer::Exported {
        job: None,
        rows: 1000,
        path: "~/exports/orders.csv".into(),
        warning: None,
    });
    assert!(h.state().dialog().is_none());
    assert_eq!(
        toast(&mut h),
        "exported 1\u{2009}000 rows to ~/exports/orders.csv"
    );
}

#[test]
fn an_answer_meant_for_another_modal_is_ignored() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv").keys("enter");
    h.db(DbAnswer::Connected(":memory:".into()));
    h.db(DbAnswer::ConnectFailed("nope".into()));

    let dialog = h.state().dialog();
    assert!(
        dialog.is_some(),
        "the export modal was closed by a connection"
    );
    assert_eq!(dialog.unwrap().form().error(), None);
    assert!(h.render(80, 24).contains("Exporting"));
}

#[test]
fn a_late_answer_does_not_reopen_the_modal() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("rows.csv").keys("enter esc");
    assert!(h.state().dialog().is_none());
    h.db(DbAnswer::ExportFailed {
        job: None,
        rows: 7,
        path: "rows.csv".into(),
        message: "too late".into(),
    });
    assert!(h.state().overlays.is_empty());
}

#[test]
fn truncated_result_labels_fetched_scope() {
    let mut result = orders();
    result.truncated = true;
    let mut h = harness_with(result, "SELECT * FROM orders");
    h.keys("ctrl-e");
    let text = h.render(80, 24);
    assert!(
        text.contains("result 1 orders · 7 rows (7 of more)"),
        "{text}"
    );
    // In amber, so it is not missed.
    let buffer = h.render_buffer(80, 24);
    let row = (0..24u16)
        .find(|&y| buffer_lines(&buffer)[usize::from(y)].contains("of more"))
        .unwrap();
    let x = buffer_lines(&buffer)[usize::from(row)]
        .chars()
        .position(|c| c == '(')
        .unwrap();
    assert_eq!(buffer[(u16::try_from(x).unwrap(), row)].fg, h.theme().amber);

    let mut h = harness();
    h.keys("ctrl-e");
    assert!(h.render(80, 24).contains("7 rows (all fetched)"));
}

#[test]
fn all_rows_scope_emits_export_job_and_closes() {
    let mut h = harness();
    h.keys("ctrl-e")
        .type_str("~/all")
        .keys("tab tab right enter");

    assert!(h.state().dialog().is_none(), "the dialog closes at once");
    let job = export_job(&h.take_effects());
    assert_eq!(
        job.source,
        ExportSource::Rerun("SELECT * FROM orders".into())
    );
    let id = job.job.expect("a job of its own");
    let entry = h.state().jobs.get(id).unwrap();
    assert_eq!(entry.kind, JobKind::Export);
    assert_eq!(entry.state, JobState::Running { rows: 0 });

    // Progress shows in the status line, and the end is a toast.
    h.db(DbAnswer::ExportProgress {
        job: id,
        rows: 120_000,
    });
    // Started on the first tick, timed in tick steps.
    h.advance(3100);
    let text = h.render(120, 40);
    assert!(
        text.contains("exporting · 120\u{2009}000 rows · 2.8 s"),
        "{text}"
    );
    h.db(DbAnswer::Exported {
        job: Some(id),
        rows: 200_000,
        path: "~/all.csv".into(),
        warning: None,
    });
    assert_eq!(
        h.state().jobs.get(id).unwrap().state,
        JobState::Done { rows: 200_000 }
    );
    assert_eq!(toast(&mut h), "exported 200\u{2009}000 rows to ~/all.csv");
    assert!(h.state().sessions.active().in_flight.is_none());
}

#[test]
fn an_export_job_is_cancelled_like_a_query() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("all").keys("tab tab right enter");
    let id = export_job(&h.take_effects()).job.unwrap();
    h.keys("ctrl-c");
    assert!(
        h.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::CancelQuery(_)))
    );
    assert_eq!(toast(&mut h), "export cancelled");
    // The failure the cancel causes says nothing more.
    h.db(DbAnswer::ExportFailed {
        job: Some(id),
        rows: 3000,
        path: "all.csv".into(),
        message: "interrupted".into(),
    });
    assert_eq!(h.state().jobs.get(id).unwrap().state, JobState::Cancelled);
    assert_eq!(toast(&mut h), "export cancelled");
    assert!(h.state().sessions.active().in_flight.is_none());
}

#[test]
fn closing_the_session_cancels_its_export() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("all").keys("tab tab right enter");
    let id = export_job(&h.take_effects()).job.unwrap();
    let session = h.active_session();
    // It asks first, as for a running query; the export is stopped before
    // the worker goes, so it leaves no file.
    h.keys("ctrl-w right enter");
    let effects = h.take_effects();
    let at = |wanted: &Effect| effects.iter().position(|e| e == wanted);
    let cancel = at(&Effect::CancelQuery(session)).expect("cancelled");
    let close = at(&Effect::CloseSession(session)).expect("closed");
    assert!(cancel < close, "{effects:?}");
    assert_eq!(h.state().jobs.get(id).unwrap().state, JobState::Cancelled);
}

#[test]
fn a_failed_export_job_says_how_far_it_got() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("all").keys("tab tab right enter");
    let id = export_job(&h.take_effects()).job.unwrap();
    h.db(DbAnswer::ExportFailed {
        job: Some(id),
        rows: 3000,
        path: "~/all.csv".into(),
        message: "disk full".into(),
    });
    assert!(matches!(
        h.state().jobs.get(id).unwrap().state,
        JobState::Failed(_)
    ));
    assert_eq!(
        toast(&mut h),
        "export to ~/all.csv failed after 3\u{2009}000 rows: disk full"
    );
}

#[test]
fn non_select_rerun_requires_confirm() {
    let mut h = harness_with(orders(), "UPDATE orders SET total = 0 RETURNING *");
    h.keys("ctrl-e").type_str("all").keys("tab tab right");
    let text = h.render(80, 24);
    assert!(text.contains("re-running may change data"), "{text}");

    h.keys("enter");
    assert!(
        !h.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::Db(_, DbCommand::Export(_))))
    );
    assert_eq!(h.dialog_title(), Some("Run Again"));
    // Cancelled, it goes back to the dialog as it was.
    h.keys("esc");
    assert_eq!(h.dialog_title(), Some("Export"));
    assert!(h.render(80, 24).contains("all"));

    // Confirmed, the export goes ahead.
    h.keys("enter y");
    assert!(h.state().dialog().is_none());
    let job = export_job(&h.take_effects());
    assert!(matches!(job.source, ExportSource::Rerun(_)));
}

#[test]
fn existing_file_opens_overwrite_confirm() {
    let mut h = harness();
    h.with_existing_file(format!("{TEST_HOME}/orders.csv"));
    h.keys("ctrl-e").type_str("~/orders");
    let text = h.render(100, 30);
    assert!(
        text.contains("→ /home/test/orders.csv · exists, will ask before overwriting"),
        "{text}"
    );
    h.keys("enter");
    assert_eq!(h.dialog_title(), Some("File Exists"));
    assert!(h.render(80, 24).contains("orders.csv exists — overwrite?"));
    let effects = h.take_effects();
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Db(_, DbCommand::Export(_)))),
        "{effects:?}"
    );

    // Overwrite: the dialog comes back waiting, and the worker may replace
    // the file.
    h.keys("y");
    assert_eq!(h.dialog_title(), Some("Export"));
    assert!(h.render(80, 24).contains("Exporting"));
    let job = export_job(&h.take_effects());
    assert!(job.overwrite);

    // A file the worker finds there anyway asks too.
    let mut h = harness();
    h.keys("ctrl-e").type_str("late").keys("enter");
    assert!(!export_job(&h.take_effects()).overwrite);
    h.db(DbAnswer::ExportExists {
        job: None,
        path: "~/late.csv".into(),
    });
    assert_eq!(h.dialog_title(), Some("File Exists"));
    h.keys("esc");
    assert_eq!(h.dialog_title(), Some("Export"));
    assert!(h.state().dialog().unwrap().form().pending().is_none());
}

#[test]
fn remembers_last_dir_and_format_per_session() {
    let mut h = harness();
    h.keys("ctrl-e")
        .type_str("~/exports/orders")
        .keys("tab right right right enter");
    assert_eq!(export_job(&h.take_effects()).etype, ExportType::JsonLines);
    h.db(DbAnswer::Exported {
        job: None,
        rows: 7,
        path: "~/exports/orders.jsonl".into(),
        warning: None,
    });

    h.keys("ctrl-e");
    let Some(DialogState::Export(form)) = h.state().dialog() else {
        panic!("no export dialog");
    };
    assert_eq!(form.path(), "~/exports/");
    assert_eq!(form.etype(), ExportType::JsonLines);
    // The cursor is at the end, ready for the name.
    h.type_str("again").keys("enter");
    assert_eq!(export_job(&h.take_effects()).path, "~/exports/again");
}

#[test]
fn excel_warning_reaches_the_toast() {
    let mut h = harness();
    h.keys("ctrl-e").type_str("book").keys("enter");
    h.db(DbAnswer::Exported {
        job: None,
        rows: 7,
        path: "book.xlsx".into(),
        warning: Some("characters Excel cannot hold were removed".into()),
    });
    let (_, last) = h.state().messages.newest_first().next().unwrap().clone();
    assert_eq!(last.level, crate::notice::Level::Warning);
    assert_eq!(
        last.text,
        "exported 7 rows to book.xlsx · characters Excel cannot hold were removed"
    );
}

// --- snapshots ------------------------------------------------------------

/// The dialog on `orders`, with `keys` pressed after the path.
fn dialog_after(result: QueryResult, keys: &str) -> AppHarness {
    let mut h = harness_with(result, "SELECT * FROM orders");
    h.with_existing_file(format!("{TEST_HOME}/exports/orders.csv"));
    h.keys("ctrl-e").type_str("~/exports/orders");
    if !keys.is_empty() {
        h.keys(keys);
    }
    h
}

fn snapshot(h: &mut AppHarness, name: &str, w: u16, height: u16) {
    insta::assert_snapshot!(name, h.render(w, height));
    insta::assert_snapshot!(
        format!("{name}_styled"),
        styled(&h.render_buffer(w, height))
    );
}

#[test]
fn export_dialog_snapshots() {
    let mut h = dialog_after(orders(), "");
    snapshot(&mut h, "export_dialog_csv_80x24", 80, 24);

    let mut h = dialog_after(orders(), "tab right right right right");
    snapshot(&mut h, "export_dialog_excel_80x24", 80, 24);

    let mut h = dialog_after(orders(), "tab left left");
    snapshot(&mut h, "export_dialog_sql_insert_80x24", 80, 24);

    let mut truncated = orders();
    truncated.truncated = true;
    let mut h = dialog_after(truncated, "tab tab right");
    snapshot(&mut h, "export_dialog_truncated_warning_80x24", 80, 24);

    let mut h = dialog_after(orders(), "enter");
    snapshot(&mut h, "export_overwrite_confirm_80x24", 80, 24);
}

#[test]
fn export_job_in_drawer_snapshot() {
    let mut h = harness();
    h.keys("ctrl-e")
        .type_str("~/exports/all")
        .keys("tab tab right enter");
    let id = export_job(&h.take_effects()).job.unwrap();
    h.db(DbAnswer::ExportProgress {
        job: id,
        rows: 120_000,
    });
    h.advance(3000);
    h.keys("J");
    snapshot(&mut h, "export_job_in_drawer_120x40", 120, 40);
}
