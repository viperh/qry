//! The generated docs match the code: run with `QRY_BLESS=1` to write them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use strum::IntoEnumIterator;

use super::*;
use crate::keymap::KeyContext;

/// The workspace's `docs/` directory.
fn docs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

/// Compares `name` under `docs/` with `wanted`, or writes it when blessing.
fn check(name: &str, wanted: &str) {
    let path = docs_dir().join(name);
    if std::env::var_os("QRY_BLESS").is_some() {
        std::fs::write(&path, wanted).unwrap();
        return;
    }
    let found = std::fs::read_to_string(&path).unwrap_or_default();
    if found != wanted {
        let line = found
            .lines()
            .zip(wanted.lines())
            .position(|(a, b)| a != b)
            .map_or_else(|| "its length".to_string(), |n| format!("line {}", n + 1));
        panic!(
            "docs/{name} is out of date (first difference at {line}); \
             run QRY_BLESS=1 cargo test -p qry docs"
        );
    }
}

#[test]
fn keybindings_doc_matches_embedded_keymap() {
    check("keybindings.md", &keybindings_markdown(&Config::embedded()));
}

#[test]
fn keybindings_doc_lists_every_bound_action() {
    let config = Config::embedded();
    let doc = keybindings_markdown(&config);
    for context in KeyContext::iter() {
        for (_, action) in config.keymap.bindings(context) {
            let text = crate::help_model::description(context, action);
            assert!(
                doc.contains(&text.replace('|', "\\|")),
                "{context} {action:?} ({text}) is not in the doc"
            );
        }
    }
    // Spot check: a key with a `|` in it would break the table.
    assert!(!doc.lines().any(|l| l.starts_with("| `|`")));
}

#[test]
fn config_doc_generated_section_matches_the_keys() {
    let path = docs_dir().join("config.md");
    let file = std::fs::read_to_string(&path).unwrap();
    let wanted = replace_generated(&file, &config_markdown(&Config::embedded()))
        .expect("docs/config.md has its BEGIN/END GENERATED markers");
    check("config.md", &wanted);
}

#[test]
fn config_schema_json_matches_the_keys() {
    check("config.schema.json", &config_schema(&Config::embedded()));
}

#[test]
fn every_printed_config_key_is_documented() {
    let documented: Vec<&str> = CONFIG_KEYS.iter().map(|k| k.path).collect();
    let mut printed = printed_keys(&Config::embedded());
    printed.sort();
    let mut sorted = documented.clone();
    sorted.sort_unstable();
    assert_eq!(printed, sorted, "CONFIG_KEYS and --print-config disagree");
}

#[test]
fn generated_docs_ignore_user_config() {
    let user = Config::with_user_file(
        "remap.json",
        r#"{"max_rows": 5, "keybindings": {"Editor": {"<f5>": null, "<alt-r>": "RunStatement"}}}"#,
    )
    .unwrap();
    // The docs are always made from the built-in config, never a user's.
    assert_ne!(
        keybindings_markdown(&user),
        keybindings_markdown(&Config::embedded())
    );
    assert_ne!(config_markdown(&user), config_markdown(&Config::embedded()));
}

#[test]
fn markdown_escapes_pipes_in_keys() {
    assert_eq!(code("|"), "`\\|`");
    assert_eq!(code("a b"), "`a` `b`");
}

#[test]
fn replace_generated_keeps_the_text_around_the_markers() {
    let file = "intro\n<!-- BEGIN GENERATED -->\nold\n<!-- END GENERATED -->\noutro\n";
    assert_eq!(
        replace_generated(file, "new\n").unwrap(),
        "intro\n<!-- BEGIN GENERATED -->\nnew\n<!-- END GENERATED -->\noutro\n"
    );
    assert!(replace_generated("no markers", "x").is_none());
}

/// `docs/assets/qry-main.svg`: the main screen after browsing a schema
/// and running a query, drawn by the real views.
#[test]
fn readme_screenshot_matches_the_views() {
    use qry_core::testing::{FakeDatabase, FakeSchema, Reply, typed_result};

    use crate::{app::UiEvent, testing::AppHarness};

    let schema = FakeSchema::new()
        .catalog("main", true, false)
        .table("main", "users")
        .table("main", "orders")
        .table("main", "products")
        .view("main", "recent_orders")
        .column("main", "users", FakeSchema::pk("id", "integer"))
        .column("main", "users", FakeSchema::col("name", "text"))
        .column("main", "users", FakeSchema::col("email", "text"));
    let users = typed_result(
        &[
            ("id", "int8"),
            ("name", "text"),
            ("email", "text"),
            ("created_at", "timestamptz"),
            ("total", "numeric"),
        ],
        &[
            &[
                Some("1"),
                Some("alice"),
                Some("alice@example.com"),
                Some("2026-10-01 09:12:44"),
                Some("12.50"),
            ],
            &[
                Some("2"),
                Some("bob"),
                None,
                Some("2026-10-02 10:00:01"),
                Some("100.00"),
            ],
            &[
                Some("3"),
                Some("carol"),
                Some("carol@example.com"),
                Some("2026-10-03 11:30:00"),
                Some("7.25"),
            ],
            &[
                Some("4"),
                Some("dave"),
                Some("dave@example.com"),
                Some("2026-10-04 08:45:10"),
                Some("42.00"),
            ],
            &[
                Some("5"),
                Some("erin"),
                Some("erin@example.com"),
                Some("2026-10-05 17:02:33"),
                Some("3.99"),
            ],
        ],
    );
    let db = FakeDatabase::default().schema(schema).reply(Reply::Delay(
        std::time::Duration::from_millis(38),
        Box::new(Reply::Rows(users)),
    ));
    let mut h = AppHarness::new(Config::embedded()).with_database(db);
    h.send(UiEvent::Started);
    h.keys("space");
    h.keys("tab")
        .type_str("select id, name, email, created_at, total\nfrom users\nwhere total > 0;")
        .keys("f5");
    h.advance(60_000);
    h.state_mut().focus = crate::app::state::Focus::Results;
    h.keys("j j l");
    let buffer = h.render_buffer(120, 30);
    let theme = *h.theme();
    check(
        "assets/qry-main.svg",
        &screenshot_svg(&buffer, theme.base().bg.unwrap_or_default()),
    );
}
