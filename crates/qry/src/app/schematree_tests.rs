//! The schema browser end to end: keys through the reducer, a fake session
//! answering the loads, the rows and the drawn tree.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use qry_core::{
    schema::{Index, SchemaPath},
    testing::FakeDatabase,
};

use super::*;
use crate::{
    app::{DbAnswer, Effect, UiEvent, dialogs::DialogState, state::Focus},
    config::Config,
    connections::{Driver, Secret, StoredConnection},
    db::DbCommand,
    testing::{AppHarness, FakeSchema, Reply, buffer_lines, render_with, result, styled},
    views,
};

/// The scratch database's schema, as the fake session reports it.
fn schema() -> FakeSchema {
    FakeSchema::new()
        .catalog("main", true, false)
        .catalog("temp", false, true)
        .table("main", "users")
        .table("main", "orders")
        .view("main", "big_orders")
        .table("temp", "scratchpad")
        .column("main", "orders", FakeSchema::pk("id", "int8"))
        .column(
            "main",
            "orders",
            FakeSchema::fk("user_id", "int8", "users", "id"),
        )
        .column("main", "orders", FakeSchema::col("total", "numeric"))
        .column(
            "main",
            "orders",
            qry_core::schema::Column {
                nullable: false,
                ..FakeSchema::col("created_at", "timestamptz")
            },
        )
        .index(
            "main",
            "orders",
            Index {
                name: "orders_pkey".into(),
                columns: vec!["id".into()],
                unique: true,
            },
        )
        .index(
            "main",
            "orders",
            Index {
                name: "orders_user".into(),
                columns: vec!["user_id".into()],
                unique: false,
            },
        )
        .column("main", "users", FakeSchema::pk("id", "int8"))
}

/// The app started on scratch, connected by a fake with `schema`, the tree
/// focused.
fn harness_with(schema: FakeSchema, db: FakeDatabase) -> AppHarness {
    let mut h = AppHarness::new(Config::embedded()).with_database(db.schema(schema));
    h.send(UiEvent::Started);
    h.state_mut().focus = Focus::Tree;
    h.take_effects();
    h
}

fn harness() -> AppHarness {
    harness_with(schema(), FakeDatabase::default())
}

/// The names (or labels) of the rows, indented by depth.
fn rows(h: &AppHarness) -> Vec<String> {
    let state = h.state();
    state
        .tree
        .rows
        .iter()
        .map(|row| {
            let label = match row.node {
                TreeNode::Folder { folder, .. } => folder.label().to_string(),
                TreeNode::Section { section, .. } => section.label().to_string(),
                TreeNode::Error { .. } => "!".into(),
                TreeNode::Note { text, .. } => text.into(),
                node => name(state, node).unwrap_or("?").to_string(),
            };
            format!("{}{label}", "  ".repeat(usize::from(row.depth)))
        })
        .collect()
}

fn selected(h: &AppHarness) -> String {
    rows(h)[h.state().tree.selected].trim().to_string()
}

/// Moves the selection to the row named `name`.
fn select(h: &mut AppHarness, name: &str) {
    let at = rows(h).iter().position(|r| r.trim() == name).unwrap();
    h.keys("g g");
    for _ in 0..at {
        h.keys("j");
    }
    assert_eq!(selected(h), name);
}

fn loads(effects: &[Effect]) -> Vec<SchemaPath> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Db(_, DbCommand::LoadSchema { path, .. }) => Some(path.clone()),
            _ => None,
        })
        .collect()
}

/// Draws only the tree.
fn tree_buffer(h: &AppHarness, w: u16, height: u16) -> ratatui::buffer::Buffer {
    let state = h.state();
    render_with(w, height, |frame| {
        views::tree::draw(state, &state.theme, frame, frame.area());
    })
}

fn tree_text(h: &AppHarness, w: u16, height: u16) -> String {
    buffer_lines(&tree_buffer(h, w, height)).join("\n")
}

fn saved(name: &str, secret: Secret) -> StoredConnection {
    let mut record = StoredConnection::with_id(format!("id-{name}"), name, Driver::Postgres);
    record.host = Some("db.local".into());
    record.database = Some("app".into());
    record.secret = secret;
    record
}

#[test]
fn flatten_respects_expanded_set() {
    let mut h = harness();
    assert_eq!(rows(&h), ["scratch"]);

    // The connection opens on its own catalog and its tables.
    h.keys("l");
    assert_eq!(
        rows(&h),
        [
            "scratch",
            "  main",
            "    Tables",
            "      orders",
            "      users",
            "    Views",
        ]
    );

    select(&mut h, "orders");
    h.keys("l");
    assert_eq!(
        rows(&h)[4..],
        [
            "        Columns",
            "        Indexes",
            "      users",
            "    Views"
        ]
    );
    h.keys("j l");
    assert_eq!(
        rows(&h)[5..9],
        [
            "          id",
            "          user_id",
            "          total",
            "          created_at"
        ]
    );

    // Closing Tables hides everything under it, and opening it again
    // shows it as it was.
    select(&mut h, "Tables");
    h.keys("space");
    assert_eq!(rows(&h), ["scratch", "  main", "    Tables", "    Views"]);
    h.keys("space");
    assert!(rows(&h).contains(&"          user_id".to_string()));
}

#[test]
fn collapse_on_child_moves_to_parent() {
    let mut h = harness();
    h.keys("l");
    select(&mut h, "users");
    h.keys("h");
    assert_eq!(selected(&h), "Tables");
    h.keys("h");
    assert_eq!(selected(&h), "Tables", "an open node closes first");
    assert!(!rows(&h).iter().any(|r| r.trim() == "users"));
    h.keys("left");
    assert_eq!(selected(&h), "main");
}

#[test]
fn filter_keeps_ancestors_of_matches() {
    let mut h = harness();
    h.keys("l");
    h.keys("/ o r d");
    assert_eq!(
        rows(&h),
        [
            "scratch",
            "  main",
            "    Tables",
            "      orders",
            "    Views",
            "      big_orders",
        ]
    );
    assert_eq!(selected(&h), "orders", "the first match is selected");
    // Matching ignores case.
    h.keys("backspace backspace backspace").type_str("USE");
    assert_eq!(rows(&h).last().unwrap().trim(), "users");
}

#[test]
fn filter_does_not_trigger_loads() {
    let mut h = harness();
    h.keys("l");
    h.take_effects();
    // Columns of no table are loaded, so `id` matches nothing yet.
    h.keys("/ i d");
    assert!(loads(&h.take_effects()).is_empty());
    assert!(rows(&h).is_empty() || !rows(&h).iter().any(|r| r.trim() == "id"));
    assert!(h.state().tree.filter_partial);
    assert!(tree_text(&h, 40, 10).contains("filter covers loaded objects only"));
}

#[test]
fn filter_esc_clears_and_restores_selection() {
    let mut h = harness();
    h.keys("l");
    select(&mut h, "Views");
    h.keys("/ u s");
    assert_eq!(selected(&h), "users");
    h.keys("esc");
    assert!(h.state().tree.filter.is_none());
    assert_eq!(selected(&h), "Views");
    assert_eq!(rows(&h).len(), 6);

    // Enter keeps it, and a later Esc clears it.
    h.keys("/ u s enter");
    let filter = h.state().tree.filter.as_ref().unwrap();
    assert!(!filter.editing);
    h.keys("j");
    assert_eq!(selected(&h), "users", "keys move again");
    h.keys("esc");
    assert!(h.state().tree.filter.is_none());
}

#[test]
fn system_objects_hidden_until_toggled() {
    let mut h = harness();
    h.keys("l");
    assert!(!rows(&h).iter().any(|r| r.trim() == "temp"));
    h.keys(".");
    assert!(rows(&h).iter().any(|r| r.trim() == "temp"));
    h.keys(".");
    assert!(!rows(&h).iter().any(|r| r.trim() == "temp"));
}

#[test]
fn enter_on_table_emits_insert_and_run_effects() {
    let mut h = harness_with(
        schema(),
        FakeDatabase::default().reply(Reply::Rows(result(&["id"], &[&[Some("1")]]))),
    );
    h.keys("l");
    h.state_mut().editor_mut().textarea.insert_str("select 1");
    select(&mut h, "users");
    h.take_effects();
    h.keys("enter");
    let effects = h.take_effects();
    let session = h.active_session();
    assert!(
        effects.contains(&Effect::Db(
            session,
            DbCommand::Query("SELECT * FROM users".into())
        )),
        "{effects:?}"
    );
    assert_eq!(
        h.state().editor().query(),
        "select 1;\nSELECT * FROM users;"
    );
    assert_eq!(h.state().results().result.as_ref().unwrap().rows.len(), 1);
}

#[test]
fn s_on_table_emits_insert_only() {
    let mut h = harness();
    h.keys("l");
    select(&mut h, "orders");
    h.take_effects();
    h.keys("s");
    assert!(h.take_effects().is_empty());
    assert_eq!(h.state().editor().query(), "SELECT * FROM orders;");
    // A second goes on a line of its own.
    h.keys("s");
    assert_eq!(
        h.state().editor().query(),
        "SELECT * FROM orders;\nSELECT * FROM orders;"
    );
}

#[test]
fn names_that_need_quoting_are_quoted_for_the_dialect() {
    let quoted = FakeSchema::new()
        .catalog("main", true, false)
        .table("main", "Order Items");
    let mut h = harness_with(quoted, FakeDatabase::default());
    h.keys("l");
    select(&mut h, "Order Items");
    h.keys("s");
    assert_eq!(h.state().editor().query(), "SELECT * FROM \"Order Items\";");
    h.keys("y");
    assert!(
        h.take_effects()
            .contains(&Effect::CopyToClipboard("\"Order Items\"".into()))
    );
}

#[test]
fn delete_opens_confirm_and_cancel_is_noop() {
    let mut h = harness();
    h.file(crate::app::FileAnswer::ConnectionsLoaded(vec![saved(
        "prod",
        Secret::Keyring,
    )]));
    select(&mut h, "prod");
    let before = h.state().tree.connections.clone();
    h.take_effects();
    h.keys("D");
    assert!(matches!(h.state().dialog(), Some(DialogState::Confirm(_))));
    h.keys("esc");
    assert!(h.state().dialog().is_none());
    assert_eq!(h.state().tree.connections, before);
    assert!(h.take_effects().is_empty());
    // Enter on the default button cancels too.
    h.keys("D enter");
    assert_eq!(h.state().tree.connections, before);
    assert!(h.take_effects().is_empty());
}

#[test]
fn confirm_delete_forgets_keychain_secret_and_saves() {
    let mut h = harness();
    h.file(crate::app::FileAnswer::ConnectionsLoaded(vec![saved(
        "prod",
        Secret::Keyring,
    )]));
    select(&mut h, "prod");
    h.take_effects();
    h.keys("D right enter");
    assert_eq!(
        h.take_effects(),
        [
            Effect::ForgetSecret("id-prod".into()),
            Effect::SaveConnections(Vec::new())
        ]
    );
    assert_eq!(rows(&h), ["scratch"]);
}

#[test]
fn delete_prompt_secret_sends_no_forget() {
    let mut h = harness();
    h.file(crate::app::FileAnswer::ConnectionsLoaded(vec![saved(
        "prod",
        Secret::Prompt,
    )]));
    select(&mut h, "prod");
    h.take_effects();
    h.keys("delete y");
    assert_eq!(h.take_effects(), [Effect::SaveConnections(Vec::new())]);
}

#[test]
fn ddl_in_a_session_invalidates_its_cache_and_open_nodes_reload() {
    let mut h = harness_with(
        schema(),
        FakeDatabase::default()
            .reply(Reply::Affected(0))
            .reply(Reply::Rows(result(&["x"], &[]))),
    );
    h.keys("l");
    h.take_effects();

    // A query leaves the cache alone.
    h.keys("tab").type_str("select 1");
    h.keys("f5");
    // DDL drops it, and what is open loads again.
    h.keys("ctrl-a backspace")
        .type_str("CREATE TABLE t (x int)");
    let effects = h.take_effects();
    assert!(loads(&effects).is_empty(), "{effects:?}");
    h.keys("f5");
    let effects = h.take_effects();
    assert_eq!(
        loads(&effects),
        [SchemaPath::Catalogs, SchemaPath::objects("main")],
        "{effects:?}"
    );
}

#[test]
fn ddl_order_with_query_first() {
    // The plain query runs first and keeps the cache; then the DDL.
    let mut h = harness_with(
        schema(),
        FakeDatabase::default()
            .reply(Reply::Rows(result(&["x"], &[])))
            .reply(Reply::Affected(0)),
    );
    h.keys("l");
    h.keys("tab").type_str("select 1");
    h.take_effects();
    h.keys("f5");
    assert!(loads(&h.take_effects()).is_empty());
    h.keys("ctrl-a backspace").type_str("drop table users");
    h.keys("f5");
    assert!(!loads(&h.take_effects()).is_empty());
}

#[test]
fn a_failed_catalog_shows_its_error_only_there_and_r_retries_it() {
    let failing = schema().catalog("audit", false, false).fail(
        SchemaPath::objects("audit"),
        "permission denied for schema audit",
    );
    let mut h = harness_with(failing, FakeDatabase::default());
    h.keys("l");
    select(&mut h, "audit");
    h.keys("l");
    let rows = rows(&h);
    assert_eq!(rows[rows.len() - 2..], ["  audit", "    !"]);
    let text = tree_text(&h, 60, 14);
    assert!(
        text.contains("! permission denied for schema audit"),
        "{text}"
    );
    assert!(text.contains("orders"), "the other catalog is fine: {text}");

    h.take_effects();
    h.keys("j r");
    assert_eq!(loads(&h.take_effects()), [SchemaPath::objects("audit")]);
}

#[test]
fn a_failed_reload_keeps_what_was_shown() {
    let mut h = harness();
    h.keys("l");
    let before = rows(&h);
    // A reload of main that fails.
    let id = h.active_session();
    let generation = h
        .state_mut()
        .sessions
        .get_mut(id)
        .unwrap()
        .schema
        .begin(&SchemaPath::objects("main"));
    h.db_for(
        id,
        DbAnswer::Schema {
            path: SchemaPath::objects("main"),
            generation,
            outcome: Err("connection reset".into()),
        },
    );
    let after: Vec<String> = rows(&h).into_iter().filter(|r| r.trim() != "!").collect();
    assert_eq!(after, before);
    assert!(rows(&h).iter().any(|r| r.trim() == "!"));
}

#[test]
fn a_late_answer_from_before_a_reconnect_is_ignored() {
    let mut h = harness();
    h.keys("l");
    let id = h.active_session();
    let stale = h
        .state_mut()
        .sessions
        .get_mut(id)
        .unwrap()
        .schema
        .begin(&SchemaPath::objects("main"));
    // Reconnecting clears the cache.
    h.db_for(id, DbAnswer::Connected(":memory:".into()));
    let rows_now = rows(&h);
    h.db_for(
        id,
        DbAnswer::Schema {
            path: SchemaPath::objects("main"),
            generation: stale,
            outcome: Ok(qry_core::schema::Children::Objects(vec![])),
        },
    );
    assert_eq!(rows(&h), rows_now);
}

#[test]
fn connection_rows_show_badge_and_state() {
    let mut h = harness();
    h.file(crate::app::FileAnswer::ConnectionsLoaded(vec![saved(
        "prod",
        Secret::None,
    )]));
    let text = tree_text(&h, 40, 6);
    assert!(text.contains("▌▸ ● sq  scratch"), "{text}");
    assert!(text.contains(" ▸ ○ pg  prod"), "{text}");
}

#[test]
fn palette_lists_loaded_tables_and_runs_them() {
    let mut h = harness_with(
        schema(),
        FakeDatabase::default().reply(Reply::Rows(result(&["id"], &[&[Some("7")]]))),
    );
    h.keys("l");
    h.take_effects();
    h.keys("ctrl-p").type_str("Table: users");
    h.keys("enter");
    let session = h.active_session();
    assert!(h.take_effects().contains(&Effect::Db(
        session,
        DbCommand::Query("SELECT * FROM users".into())
    )));
}

#[test]
fn moving_through_ten_thousand_tables_draws_only_what_is_in_view() {
    let mut big = FakeSchema::new().catalog("main", true, false);
    for i in 0..10_000 {
        big = big.table("main", &format!("t{i:05}"));
    }
    let mut h = harness_with(big, FakeDatabase::default());
    h.keys("l");
    assert_eq!(h.state().tree.rows.len(), 10_004);
    h.keys("G");
    let text = tree_text(&h, 40, 10);
    assert!(text.contains("Views"), "{text}");
    for _ in 0..50 {
        h.keys("k");
    }
    assert!(tree_text(&h, 40, 10).contains("t09953"));
}

// ---- snapshots ---------------------------------------------------------

fn snapshot(name: &str, h: &AppHarness, w: u16, height: u16) {
    let buffer = tree_buffer(h, w, height);
    insta::assert_snapshot!(name, buffer_lines(&buffer).join("\n"));
    insta::assert_snapshot!(format!("{name}_styled"), styled(&buffer));
}

#[test]
fn schematree_collapsed_40x20() {
    let mut h = harness();
    h.file(crate::app::FileAnswer::ConnectionsLoaded(vec![
        saved("prod-db", Secret::None),
        saved("billing", Secret::None),
    ]));
    snapshot("schematree_collapsed_40x20", &h, 40, 20);
}

#[test]
fn schematree_expanded_columns_40x30() {
    let mut h = harness();
    h.keys("l");
    select(&mut h, "orders");
    h.keys("l j l j j j j j l");
    snapshot("schematree_expanded_columns_40x30", &h, 40, 30);
}

#[test]
fn schematree_loading_spinner_40x10() {
    let slow = schema().delay(SchemaPath::objects("main"), u64::MAX);
    let mut h = harness_with(slow, FakeDatabase::default());
    h.keys("l");
    h.advance(300);
    assert_eq!(h.state().tree.spinner, 3);
    snapshot("schematree_loading_spinner_40x10", &h, 40, 10);
}

#[test]
fn schematree_load_error_40x10() {
    let failing = schema().fail(SchemaPath::objects("main"), "permission denied");
    let mut h = harness_with(failing, FakeDatabase::default());
    h.keys("l");
    snapshot("schematree_load_error_40x10", &h, 40, 10);
}

#[test]
fn schematree_filter_ord_40x20() {
    let mut h = harness();
    h.keys("l / o r d");
    snapshot("schematree_filter_ord_40x20", &h, 40, 20);
}

#[test]
fn schematree_focused_vs_unfocused_40x10() {
    let mut h = harness();
    h.keys("l j");
    let focused = tree_buffer(&h, 40, 10);
    h.state_mut().focus = Focus::Editor;
    let unfocused = tree_buffer(&h, 40, 10);
    insta::assert_snapshot!(
        "schematree_focused_vs_unfocused_40x10",
        format!(
            "{}\n---\n{}",
            buffer_lines(&focused).join("\n"),
            buffer_lines(&unfocused).join("\n")
        )
    );
    insta::assert_snapshot!(
        "schematree_focused_vs_unfocused_40x10_styled",
        format!("{}\n---\n{}", styled(&focused), styled(&unfocused))
    );
}

#[test]
fn schematree_long_names_truncated_24x10() {
    let long = FakeSchema::new()
        .catalog("a_schema_with_a_very_long_name", true, false)
        .table(
            "a_schema_with_a_very_long_name",
            "customer_order_line_items_archive_2024",
        );
    let mut h = harness_with(long, FakeDatabase::default());
    h.keys("l");
    snapshot("schematree_long_names_truncated_24x10", &h, 24, 10);
}

// ---- whole-app flows ---------------------------------------------------

#[test]
fn browser_flow_scratch_120x40() {
    let mut h =
        AppHarness::new(Config::embedded()).with_database(FakeDatabase::default().schema(schema()));
    h.send(UiEvent::Started);
    h.state_mut().focus = Focus::Tree;
    h.keys("enter");
    h.keys("l l");
    assert_eq!(selected(&h), "main");
    insta::assert_snapshot!("browser_flow_scratch_120x40", h.render(120, 40));
}

#[test]
fn filtering_to_a_table_and_pressing_enter_runs_it() {
    let mut h = harness_with(
        schema(),
        FakeDatabase::default().reply(Reply::Rows(result(
            &["id", "email"],
            &[&[Some("1"), Some("a@example.com")]],
        ))),
    );
    h.keys("l");
    h.keys("/ u s e r enter enter");
    assert_eq!(h.state().editor().query(), "SELECT * FROM users;");
    let screen = h.render(120, 40);
    assert!(screen.contains("a@example.com"), "{screen}");
}

#[test]
fn deleting_from_the_tree_asks_and_cancel_keeps_it() {
    let mut h = harness();
    h.file(crate::app::FileAnswer::ConnectionsLoaded(vec![saved(
        "prod",
        Secret::None,
    )]));
    select(&mut h, "prod");
    h.keys("D");
    h.keys("esc");
    assert!(h.state().tree.position("id-prod").is_some());
    // The confirm is destructive, so it starts on Cancel: Right picks Delete.
    h.keys("D right enter");
    assert!(h.state().tree.position("id-prod").is_none());
}

#[test]
fn disconnect_closes_the_selected_connections_session() {
    let mut h = harness();
    let scratch = h.active_session();
    h.take_effects();
    h.keys("x");
    let effects = h.take_effects();
    assert!(
        effects.contains(&Effect::CloseSession(scratch)),
        "{effects:?}"
    );
}

// ---- property ----------------------------------------------------------

fn key() -> impl Strategy<Value = &'static str> {
    proptest::sample::select(vec![
        "up",
        "down",
        "left",
        "right",
        "space",
        "/",
        "esc",
        "o",
        "r",
        "backspace",
        "enter",
        ".",
        "pagedown",
        "G",
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_flatten_never_panics_and_selection_stays_in_bounds(
        tables in 0..12usize,
        views in 0..4usize,
        catalogs in 1..4usize,
        keys in proptest::collection::vec(key(), 0..40),
    ) {
        let mut s = FakeSchema::new();
        for c in 0..catalogs {
            let catalog = format!("c{c}");
            s = s.catalog(&catalog, c == 0, c == 2);
            for t in 0..tables {
                s = s.table(&catalog, &format!("t{t}"))
                    .column(&catalog, &format!("t{t}"), FakeSchema::pk("id", "int"));
            }
            for v in 0..views {
                s = s.view(&catalog, &format!("v{v}"));
            }
        }
        let mut h = harness_with(s, FakeDatabase::default());
        for key in keys {
            // Enter on a table would run a query nobody scripted.
            if key == "enter" && h.state().tree.filter.as_ref().is_none_or(|f| !f.editing) {
                continue;
            }
            h.keys(key);
            let tree = &h.state().tree;
            if tree.rows.is_empty() {
                prop_assert_eq!(tree.selected, 0);
            } else {
                prop_assert!(tree.selected < tree.rows.len());
            }
            let _ = tree_text(&h, 40, 12);
        }
    }
}
