//! Unit tests for the reducer, through [`AppHarness`]: keys go through the
//! keymap as they do in the app, and the I/O the runtime would carry out
//! shows up as effects.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{state::Focus, *};
use crate::{
    connections::{Driver, StoredConnection},
    db::DbCommand,
    import::{Format, Imported},
    testing::AppHarness,
};

/// The main screen with the built-in config, as the runtime sets it up.
fn harness() -> AppHarness {
    AppHarness::new(Config::embedded())
}

/// Opens the results' Export modal: ctrl-s with the results focused.
fn open_export(h: &mut AppHarness) -> &mut AppHarness {
    h.state_mut().focus = Focus::Results;
    h.keys("ctrl-s")
}

#[test]
fn startup_opens_scratch_and_loads_connections() {
    let mut h = harness();
    h.send(UiEvent::Started);
    let effects = h.take_effects();
    let [Effect::LoadConnections, Effect::OpenSession(id, scratch)] = effects.as_slice() else {
        panic!("expected a load and the scratch session, got {effects:?}");
    };
    assert_eq!(*id, SessionId::FIRST);
    assert_eq!(scratch.path.as_deref(), Some(":memory:"));
    assert_eq!(scratch.driver, Driver::Sqlite);
    assert!(scratch.ephemeral);
    assert!(h.render(80, 24).contains("scratch"));

    // Once it opens, it stays in the tree but is never written.
    h.db(DbAnswer::Connected(":memory:".into()));
    assert!(
        !h.take_effects()
            .iter()
            .any(|e| matches!(e, Effect::SaveConnections(_))),
        "the scratch database is never saved"
    );
    let mut saved = StoredConnection::new("app", Driver::Sqlite);
    saved.path = Some("app.db".into());
    h.file(FileAnswer::ConnectionsLoaded(vec![saved]));
    let text = h.render(80, 24);
    assert!(text.contains("scratch") && text.contains("app"), "{text}");
}

#[test]
fn each_mode_opens_its_own_modal() {
    let mut h = harness();
    assert!(h.state().dialog().is_none());

    h.keys("ctrl-n");
    assert!(h.render(80, 50).contains("New Connection"));

    h.keys("esc");
    open_export(&mut h);
    let text = h.render(80, 50);
    assert!(
        text.contains("Export") && !text.contains("New Connection"),
        "{text}"
    );

    h.keys("esc");
    assert!(h.state().dialog().is_none());
    assert!(!h.render(80, 50).contains("Export"));
}

#[test]
fn keys_reach_the_open_modal_and_it_reopens_empty() {
    let mut h = harness();
    open_export(&mut h).type_str("rows.csv");
    assert!(h.render(80, 50).contains("rows.csv"));

    // Reopening builds a fresh form.
    h.keys("esc");
    open_export(&mut h);
    assert!(!h.render(80, 50).contains("rows.csv"));
}

#[test]
fn a_failed_export_keeps_the_modal_and_the_path() {
    let mut h = harness();
    open_export(&mut h)
        .type_str("no/such/dir/rows.csv")
        .keys("enter");

    let effects = h.take_effects();
    let [Effect::Db(_, DbCommand::Export(config))] = effects.as_slice() else {
        panic!("expected an export, got {effects:?}");
    };
    assert_eq!(config.path, "no/such/dir/rows.csv");
    assert_eq!(h.dialog_title(), Some("Export"), "still open");
    assert!(h.render(80, 50).contains("Exporting"));

    h.db(DbAnswer::ExportFailed(
        "could not write no/such/dir/rows.csv".into(),
    ));
    let text = h.render(80, 50);
    assert!(text.contains("could not write"), "{text}");
    assert!(text.contains("no/such/dir/rows.csv"), "{text}");
}

#[test]
fn import_waits_and_keeps_path_on_failure() {
    let mut h = harness();
    // Ctrl-o opens it from the main screen.
    h.keys("ctrl-o");
    assert_eq!(h.dialog_title(), Some("Import Connections"));
    h.type_str("~/nope.json").keys("enter");

    let effects = h.take_effects();
    let [Effect::Import { path, .. }] = effects.as_slice() else {
        panic!("expected an import, got {effects:?}");
    };
    assert_eq!(path, "~/nope.json");
    assert!(h.render(80, 50).contains("Importing"));

    h.file(FileAnswer::ImportFailed("cannot read nope.json".into()));
    let text = h.render(80, 50);
    assert!(
        text.contains("cannot read nope.json") && text.contains("~/nope.json"),
        "{text}"
    );

    // A second try that works closes it.
    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Import { .. }]
    ));
    h.file(FileAnswer::Imported(Box::new(Imported {
        format: Format::Qry,
        connections: Vec::new(),
        skipped: Vec::new(),
        warnings: Vec::new(),
    })));
    assert_eq!(h.dialog_title(), None);
}

#[test]
fn exporting_connections_waits_for_the_file() {
    let mut h = harness();
    let mut saved = StoredConnection::new("app", Driver::Sqlite);
    saved.path = Some("app.db".into());
    h.file(FileAnswer::ConnectionsLoaded(vec![saved]));
    h.keys("ctrl-s");
    assert_eq!(h.dialog_title(), Some("Export Connections"));
    assert!(h.render(80, 50).contains("Export Connections"));
    h.type_str("/root/b.json").keys("enter");

    // The tree holds the list, and the runtime writes it.
    let effects = h.take_effects();
    let [Effect::ExportConnections { path, connections }] = effects.as_slice() else {
        panic!("expected a write, got {effects:?}");
    };
    assert_eq!(path, "/root/b.json");
    assert_eq!(connections.len(), 1);
    assert!(h.render(80, 50).contains("Exporting"));

    h.file(FileAnswer::ConnectionsExportFailed(
        "cannot write /root/b.json".into(),
    ));
    assert!(h.render(80, 50).contains("cannot write /root/b.json"));

    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::ExportConnections { .. }]
    ));
    h.file(FileAnswer::ConnectionsExported {
        path: "/tmp/b.json".into(),
        count: 1,
    });
    assert_eq!(h.dialog_title(), None);
    // Its toast shows once the failure's has had its 8 s.
    h.advance(8000);
    assert!(
        h.render(80, 24)
            .contains("Exported 1 connection to /tmp/b.json")
    );
}

#[test]
fn exporting_no_saved_connections_is_refused_in_the_modal() {
    let mut h = harness();
    h.keys("ctrl-s").type_str("/tmp/b.json").keys("enter");
    assert!(h.take_effects().is_empty());
    let text = h.render(80, 50);
    assert!(
        text.contains("There are no saved connections to export"),
        "{text}"
    );
    assert_eq!(h.dialog_title(), Some("Export Connections"));
}

#[test]
fn a_written_export_closes_the_modal() {
    let mut h = harness();
    open_export(&mut h).type_str("rows.csv").keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::Db(_, DbCommand::Export(_))]
    ));

    h.db(DbAnswer::Exported("rows.csv".into()));
    assert!(h.state().dialog().is_none());
    assert_eq!(h.dialog_title(), None);
}

#[test]
fn late_answer_for_closed_modal_is_ignored() {
    let mut h = harness();
    open_export(&mut h).type_str("rows.csv").keys("enter esc");
    assert!(h.state().dialog().is_none());

    let before = h.state().clone();
    h.db(DbAnswer::Exported("rows.csv".into()));
    h.db(DbAnswer::ExportFailed("too late".into()));
    let mut after = h.state().clone();
    after.dirty = before.dirty;
    assert_eq!(after, before, "a late answer changed the state");
    assert!(h.state().overlays.is_empty());
}

#[test]
fn answer_for_other_modal_is_ignored() {
    let mut h = harness();
    // An export is waiting, and a connection's answer arrives late.
    open_export(&mut h).type_str("rows.csv").keys("enter");
    h.db(DbAnswer::Connected(":memory:".into()));
    h.db(DbAnswer::ConnectFailed("nope".into()));

    let text = h.render(80, 50);
    let dialog = h.state().dialog();
    assert!(
        dialog.is_some(),
        "the export modal was closed by a connection"
    );
    assert_eq!(dialog.unwrap().form().error(), None);
    assert!(text.contains("Exporting"), "{text}");
}

#[test]
fn a_refused_modal_stays_open_with_its_error() {
    let mut h = harness();
    open_export(&mut h).keys("enter");
    assert!(h.take_effects().is_empty());
    assert!(h.render(80, 50).contains("Path is required"));
}

/// Fills in enough of the New Connection form to be submittable.
fn fill_connection(h: &mut AppHarness) {
    for (field, value) in [(2, "db.local"), (4, "alice"), (6, "app")] {
        let dialog = h.state_mut().dialog_mut().unwrap();
        dialog.form_mut().set_focus(field);
        h.type_str(value);
    }
}

#[test]
fn connect_modal_waits_for_answer() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    let focus = h.state().focus;

    // Submitting opens a session on the connection but leaves the modal
    // waiting.
    h.keys("enter");
    let effects = h.take_effects();
    let [Effect::OpenSession(_, record)] = effects.as_slice() else {
        panic!("expected a session and nothing else, got {effects:?}");
    };
    assert_eq!(record.host.as_deref(), Some("db.local"));
    assert_eq!(h.dialog_title(), Some("New Connection"), "still open");
    assert_eq!(h.state().focus, focus);
    assert!(h.render(80, 50).contains("Connecting"));
}

#[test]
fn second_submit_while_awaiting_is_ignored() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    h.keys("enter");
    assert_eq!(h.take_effects().len(), 1);

    // A second Enter while waiting does not try again.
    h.keys("enter");
    assert!(h.take_effects().is_empty());
}

#[test]
fn connect_failure_keeps_fields_and_shows_error() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);
    h.keys("enter");
    h.take_effects();

    h.db(DbAnswer::ConnectFailed(
        "password authentication failed".into(),
    ));
    let text = h.render(80, 50);
    assert!(text.contains("password authentication failed"), "{text}");
    assert!(
        text.contains("db.local") && text.contains("alice"),
        "{text}"
    );
    assert!(!text.contains("Connecting"), "{text}");

    // Its session went with it, so correcting and submitting again opens
    // a new one.
    h.keys("x enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::CloseSession(_), Effect::OpenSession(..)]
    ));
}

#[test]
fn a_working_connection_closes_the_modal() {
    let mut h = harness();
    h.keys("ctrl-n");
    fill_connection(&mut h);

    h.keys("enter");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::OpenSession(..)]
    ));

    h.db(DbAnswer::Connected("alice@db.local:5432/app".into()));
    assert!(h.state().dialog().is_none());
    assert_eq!(h.dialog_title(), None);
    assert!(!h.render(80, 50).contains("New Connection"));
}

#[test]
fn a_connect_answer_without_a_waiting_modal_is_ignored() {
    let mut h = harness();
    // The in-memory connection at startup answers with nobody waiting.
    h.db(DbAnswer::Connected(":memory:".into()));
    h.db(DbAnswer::ConnectFailed("nope".into()));
    assert!(h.state().dialog().is_none());

    // An export modal is not touched by a connection's answer either.
    open_export(&mut h);
    h.db(DbAnswer::ConnectFailed("nope".into()));
    assert_eq!(h.dialog_title(), Some("Export"));
    assert_eq!(h.state().dialog().unwrap().form().error(), None);
}

#[test]
fn need_password_opens_prompt_and_submit_awaits_connect() {
    let mut h = harness();
    h.db(DbAnswer::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    // The prompt is a dialog, so its keys are the dialogs'.
    assert_eq!(h.dialog_title(), Some("Password"));
    let text = h.render(80, 30);
    assert!(
        text.contains("Password") && text.contains("for prod"),
        "{text}"
    );

    // Submitting leaves the password for the worker and waits for it, as a
    // connection does.
    h.type_str("x").keys("enter");
    let effects = h.take_effects();
    let [
        Effect::StashPassword {
            connection,
            password,
        },
        Effect::Db(_, DbCommand::PasswordEntered),
    ] = effects.as_slice()
    else {
        panic!("expected the stash and then the worker, got {effects:?}");
    };
    assert_eq!(connection, "c7f3a1e2", "keyed by the connection that asked");
    assert_eq!(password.as_str(), "x");
    assert!(h.render(80, 30).contains("Connecting"));

    h.db(DbAnswer::Connected(":memory:".into()));
    assert!(h.state().dialog().is_none());
}

#[test]
fn password_text_never_in_effects() {
    const CANARY: &str = "CANARY-PW-x1";
    let mut h = harness();
    h.db(DbAnswer::NeedPassword {
        id: "c7f3a1e2".into(),
        name: "prod".into(),
    });
    h.type_str(CANARY).keys("enter");
    let effects = h.take_effects();
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::StashPassword { .. }))
    );
    for effect in &effects {
        let shown = format!("{effect:?}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
    let state = format!("{:?}", h.state());
    assert!(!state.contains(CANARY), "the state printed it: {state}");

    // The same in the New Connection form's password field.
    h.keys("esc ctrl-n");
    fill_connection(&mut h);
    h.state_mut().dialog_mut().unwrap().form_mut().set_focus(5);
    h.type_str(CANARY).keys("enter");
    let effects = h.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::StashPassword { .. }, Effect::OpenSession(..)]
    ));
    for effect in &effects {
        let shown = format!("{effect:?}");
        assert!(!shown.contains(CANARY), "{shown}");
    }
}

#[test]
fn help_opens_scrolls_and_closes_with_configured_keys() {
    let mut h = harness();
    h.keys("f1");
    assert!(h.state().help().is_some());
    assert!(h.render(80, 24).contains("Help — qry"));

    // While help is open, keys go to it, not to the focused pane.
    h.state_mut().focus = Focus::Results;
    h.keys("pagedown");
    assert!(h.state().help().unwrap().scroll.get() > 0);

    h.keys("esc");
    assert!(h.state().help().is_none());

    h.keys("f1");
    assert!(h.render(80, 24).contains("Help — qry"));
    h.keys("f1");
    assert!(h.state().help().is_none());
}

#[test]
fn a_notice_answer_becomes_a_toast() {
    let mut h = harness();
    assert!(h.state().toasts.visible().is_none());
    h.db(DbAnswer::Notice(crate::notice::Notice::error("e")));
    assert_eq!(h.state().toasts.visible().unwrap().0.notice.text, "e");
    assert!(h.render(80, 24).contains("✗ e"));
}

#[test]
fn a_stopped_worker_is_reported_in_the_status_bar() {
    let mut h = harness();
    h.db(DbAnswer::WorkerStopped);
    assert!(h.render(80, 24).contains("database worker has stopped"));
}

#[test]
fn keys_reach_the_focused_pane_when_nothing_is_on_top() {
    let mut h = harness();
    h.keys("tab").type_str("select 1").keys("f8");
    assert_eq!(
        h.take_effects(),
        [Effect::Db(
            SessionId::FIRST,
            DbCommand::Query("select 1".into())
        )]
    );
}

#[test]
fn query_done_is_never_printed_in_full() {
    let mut h = harness();
    let answer = DbAnswer::QueryDone(std::sync::Arc::new(crate::testing::grid(2, 2)));
    let shown = format!("{answer:?}");
    assert!(!shown.contains("r1c1"), "{shown}");
    h.db(answer);
    assert!(h.render(80, 24).contains("r1c1"), "but it is shown");
}

#[test]
fn quitting_is_an_effect_for_the_runtime() {
    let mut h = harness();
    h.keys("ctrl-q");
    assert_eq!(h.take_effects(), [Effect::Quit]);
}

/// The harness with a 5x3 result in the grid, its cursor on row 3.
fn results_on_row_3() -> AppHarness {
    let mut h = harness();
    h.db(DbAnswer::QueryDone(std::sync::Arc::new(
        crate::testing::grid(5, 3),
    )));
    h.state_mut().focus = Focus::Results;
    h.keys("j j j");
    assert_eq!(h.state().results().row, 3);
    h
}

#[test]
fn sequence_times_out_after_one_second() {
    let mut h = results_on_row_3();
    h.keys("g");
    let effects = h.take_effects();
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::ScheduleTimer { after, .. }] if *after == std::time::Duration::from_secs(1)
        ),
        "{effects:?}"
    );
    h.advance(1001);
    assert!(h.state().pending_keys.is_empty());
    h.keys("g");
    assert_eq!(h.state().results().row, 3, "a lone g after the timeout");

    // Within the second, g g goes to the top.
    let mut h = results_on_row_3();
    h.keys("g");
    h.advance(999);
    h.keys("g");
    assert_eq!(h.state().results().row, 0);
    assert!(h.state().pending_keys.is_empty());
}

#[test]
fn ticks_leave_a_half_typed_sequence_alone() {
    let mut h = results_on_row_3();
    h.keys("g");
    // Ticks no longer clear it; only the timer does.
    h.advance(750);
    assert_eq!(h.ticks(), 3);
    assert_eq!(h.state().pending_keys.len(), 1);
    h.keys("g");
    assert_eq!(h.state().results().row, 0);
}

#[test]
fn a_broken_sequence_hands_on_the_key() {
    let mut h = results_on_row_3();
    // `g` waits, `k` is no second `g`: it moves up as usual.
    h.keys("g k");
    assert_eq!(h.state().results().row, 2);
    assert!(h.state().pending_keys.is_empty());
}

#[test]
fn a_prefix_binding_runs_once_the_sequence_times_out() {
    let config = Config::with_user_file(
        "u.json",
        r#"{"keybindings": {"Results": {"<g>": "Bottom"}}}"#,
    )
    .unwrap();
    let mut h = AppHarness::new(config);
    h.db(DbAnswer::QueryDone(std::sync::Arc::new(
        crate::testing::grid(5, 3),
    )));
    h.state_mut().focus = Focus::Results;
    h.keys("g");
    assert_eq!(h.state().results().row, 0, "waits for a second g");
    h.advance(1000);
    assert_eq!(h.state().results().row, 4);

    // A stale timer does nothing: the second wait is still on.
    h.keys("k k g");
    h.advance(500);
    h.keys("g");
    assert_eq!(h.state().results().row, 0, "g g");
    h.keys("j g");
    h.advance(600);
    assert_eq!(h.state().results().row, 1, "only 600 ms into the new wait");
    h.advance(400);
    assert_eq!(h.state().results().row, 4);
}

#[test]
fn question_mark_types_in_editor_but_opens_help_in_results() {
    let mut h = harness();
    h.keys("tab").type_str("a?b:c");
    assert_eq!(h.state().editor().query(), "a?b:c");
    assert!(h.state().help().is_none());

    h.keys("tab ?");
    assert!(h.state().help().is_some());
    h.keys("?");
    assert!(h.state().help().is_none());
}

#[test]
fn pill_follows_focus() {
    use crate::theme::Pill;
    let mut h = harness();
    h.keys("tab");
    assert_eq!(h.state().pill(), Pill::Edit);
    h.keys("tab");
    assert_eq!(h.state().pill(), Pill::Normal);
    h.keys("tab");
    assert_eq!(h.state().pill(), Pill::Normal, "the tree");

    // The help and the dialogs keep the pill of what is underneath.
    h.keys("tab f1");
    assert!(h.state().help().is_some());
    assert_eq!(h.state().pill(), Pill::Edit);
    h.keys("esc ctrl-n");
    assert_eq!(h.state().pill(), Pill::Edit);
    h.keys("f1");
    assert_eq!(h.state().pill(), Pill::Edit);
    // There is no palette to open until T206; its context gives COMMAND.
    assert_eq!(
        crate::keymap::KeyContext::Palette.pill(),
        Some(Pill::Command)
    );
}

#[test]
fn overlays_take_the_keys_alone() {
    let mut h = harness();
    h.keys("f1 ctrl-n");
    assert!(h.state().dialog().is_none(), "ctrl-n does nothing in help");
    h.keys("esc");

    // In a dialog, ctrl-q still quits, F1 opens the help over it, and
    // ctrl-p goes to the text field, which ignores it.
    h.keys("ctrl-n");
    h.type_str("ab").keys("ctrl-p");
    assert!(h.state().dialog().is_some());
    assert!(h.render(80, 50).contains("ab"));
    h.keys("f1");
    assert!(h.state().help().is_some() && h.state().dialog().is_some());
    h.keys("esc");
    h.take_effects();
    h.keys("ctrl-q");
    assert_eq!(h.take_effects(), [Effect::Quit]);
}

#[test]
fn space_types_in_a_text_field_and_picks_in_a_choice() {
    let mut h = harness();
    h.keys("ctrl-n");
    let dialog = h.state_mut().dialog_mut().unwrap();
    // The name field, then the driver choice.
    dialog.form_mut().set_focus(0);
    h.type_str("a b");
    assert!(h.render(80, 50).contains("a b"));
    let form = h.state().dialog().unwrap().form();
    let choice = (0..form.labels().len())
        .find(|i| {
            !matches!(
                form.fields()[*i],
                crate::app::dialogs::form::Field::Text(_)
                    | crate::app::dialogs::form::Field::Secret(_)
            )
        })
        .unwrap();
    let before = form.selected(choice);
    h.state_mut()
        .dialog_mut()
        .unwrap()
        .form_mut()
        .set_focus(choice);
    h.keys("space");
    assert_ne!(h.state().dialog().unwrap().form().selected(choice), before);
}

#[test]
fn ctrl_digit_switches_session() {
    let keymap = &Config::embedded().keymap;
    let contexts = [
        crate::keymap::KeyContext::Tree,
        crate::keymap::KeyContext::Global,
    ];
    assert_eq!(
        keymap.resolve(&contexts, &crate::testing::parse_keys("ctrl-3")),
        crate::keymap::Lookup::Match(crate::action::Action::SwitchSession(3))
    );

    // Two more sessions, on the scratch connection: ctrl-3 is the last.
    let mut h = harness();
    h.send(UiEvent::Started);
    h.keys("shift-enter shift-enter ctrl-1");
    assert_eq!(h.active_session(), SessionId::FIRST);
    h.keys("ctrl-3");
    assert_eq!(h.active_session(), h.sessions()[2]);
}

#[test]
fn tree_keys_follow_vim() {
    let mut h = harness();
    let mut a = StoredConnection::new("a", Driver::Sqlite);
    a.path = Some("a.db".into());
    let mut b = StoredConnection::new("b", Driver::Sqlite);
    b.path = Some("b.db".into());
    h.file(FileAnswer::ConnectionsLoaded(vec![a, b]));
    h.keys("j");
    assert_eq!(h.state().tree.selected, 1);
    h.keys("k");
    assert_eq!(h.state().tree.selected, 0);
    h.keys("G");
    assert_eq!(h.state().tree.selected, 1);
    h.keys("g g");
    assert_eq!(h.state().tree.selected, 0);
}
