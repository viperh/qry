//! Unit tests for `value_popup.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use qry_core::testing::{FakeDatabase, Reply, typed_result};

use super::*;
use crate::{
    app::{Effect, UiEvent, state::Overlay},
    config::Config,
    testing::{AppHarness, styled},
    views::inspector::inspector_tests::ran,
};

/// A long sentence, longer than any popup is wide.
const LONG: &str = "first line\nsecond line with a long sentence that wraps across the dialog \
                    width without cutting words in half, and then goes on for a while longer \
                    still so that it needs a third line even at a hundred and twenty columns";
const JSON: &str = r#"{"id":7,"tags":["a","b"],"active":true,"owner":null,"score":-1.5}"#;

/// The popup on a one-cell result of type `type_name` holding `value`.
fn popup_on(type_name: &str, value: Option<&str>, w: u16, h: u16) -> AppHarness {
    let mut app = ran(typed_result(&[("v", type_name)], &[&[value]]), w, h);
    app.keys("enter");
    app
}

fn popup(app: &AppHarness) -> &crate::app::state::ValuePopup {
    match app.state().overlays.last() {
        Some(Overlay::Value(popup)) => popup,
        other => panic!("no popup: {other:?}"),
    }
}

fn snapshot(name: &str, app: &mut AppHarness, w: u16, h: u16) {
    insta::assert_snapshot!(format!("{name}_{w}x{h}"), app.render(w, h));
    insta::assert_snapshot!(
        format!("{name}_{w}x{h}_styled"),
        styled(&app.render_buffer(w, h))
    );
}

fn copied(app: &mut AppHarness) -> Vec<String> {
    app.take_effects()
        .into_iter()
        .filter_map(|e| match e {
            Effect::CopyToClipboard(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn the_popup_is_80_percent_at_most_120x40() {
    assert_eq!(popup_rect(Rect::new(0, 0, 80, 24)), Rect::new(8, 3, 64, 19));
    assert_eq!(
        popup_rect(Rect::new(0, 0, 300, 100)),
        Rect::new(90, 30, 120, 40)
    );
    assert_eq!(content_size((80, 24)), (60, 17));
}

#[test]
fn popup_text_wrapped_120x40() {
    let mut app = popup_on("text", Some(LONG), 120, 40);
    let text = app.render(120, 40);
    assert!(
        text.contains("result.v · text · row 1") || text.contains("users.v · text · row 1"),
        "{text}"
    );
    snapshot("popup_text_wrapped", &mut app, 120, 40);
}

#[test]
fn popup_json_pretty_120x40() {
    let mut app = popup_on("jsonb", Some(JSON), 120, 40);
    let text = app.render(120, 40);
    assert!(text.contains(r#"  "tags": ["#), "{text}");
    assert!(text.contains("f format"), "{text}");
    snapshot("popup_json_pretty", &mut app, 120, 40);
}

#[test]
fn popup_hex_120x30() {
    let mut app = popup_on(
        "bytea",
        Some("\\x48656c6c6f2c20776f726c64210a00010203ff"),
        120,
        30,
    );
    let text = app.render(120, 30);
    assert!(
        text.contains(
            "00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21 0a 00 01  |Hello, world!...|"
        ),
        "{text}"
    );
    snapshot("popup_hex", &mut app, 120, 30);
}

#[test]
fn popup_null_80x24() {
    let mut app = popup_on("text", None, 80, 24);
    snapshot("popup_null", &mut app, 80, 24);
    app.keys("y");
    assert!(copied(&mut app).is_empty(), "nothing to copy");
}

#[test]
fn popup_scrolled_80x24() {
    let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
    let mut app = popup_on("text", Some(&lines.join("\n")), 80, 24);
    app.keys("j j j");
    assert_eq!(popup(&app).scroll, 3);
    let text = app.render(80, 24);
    assert!(text.contains(" 4–20 of 40 "), "{text}");
    snapshot("popup_scrolled", &mut app, 80, 24);
    app.keys("G");
    assert_eq!(popup(&app).scroll, 23, "the last page");
    app.keys("g");
    assert_eq!(popup(&app).scroll, 0);
    app.keys("pagedown");
    assert_eq!(popup(&app).scroll, 17);
}

#[test]
fn popup_y_copies_full_value_not_truncated() {
    let long = "x".repeat(5_000);
    let mut app = popup_on("text", Some(&long), 80, 24);
    app.keys("y");
    assert_eq!(copied(&mut app), [long]);
    assert!(app.render(80, 24).contains("copied value"));
}

#[test]
fn popup_f_toggles_pretty_json() {
    let mut app = popup_on("jsonb", Some(JSON), 120, 40);
    assert!(matches!(popup(&app).rendered, Rendered::Json(_)));
    app.keys("f");
    assert_eq!(popup(&app).rendered, Rendered::Text(vec![JSON.into()]));
    app.keys("f");
    assert!(matches!(popup(&app).rendered, Rendered::Json(_)));
    // Text that is not JSON has no format to switch.
    let mut text = popup_on("text", Some("{not json"), 120, 40);
    text.keys("f");
    assert!(!popup(&text).json);
    assert!(!text.render(120, 40).contains("f format"));
}

#[test]
fn popup_w_toggles_wrap() {
    let mut app = popup_on("text", Some(LONG), 80, 24);
    let wrapped = popup(&app).rendered.len();
    assert!(wrapped > 2, "{wrapped}");
    app.keys("w");
    assert_eq!(popup(&app).rendered.len(), 2, "one line per line of text");
    let before = app.render(80, 24);
    app.keys("l l");
    assert_eq!(popup(&app).hscroll, 8);
    assert_ne!(app.render(80, 24), before, "panned");
    app.keys("w");
    assert_eq!(popup(&app).hscroll, 0);
    assert_eq!(popup(&app).rendered.len(), wrapped);
}

#[test]
fn a_resize_lays_the_value_out_again() {
    let mut app = popup_on("text", Some(LONG), 120, 40);
    let wide = popup(&app).rendered.len();
    app.resize(80, 24);
    assert!(popup(&app).rendered.len() > wide);
}

#[test]
fn q_and_esc_close_the_popup() {
    for key in ["q", "esc"] {
        let mut app = popup_on("text", Some("x"), 80, 24);
        app.keys(key);
        assert!(app.state().overlays.is_empty(), "{key}");
    }
}

/// The flow from the task: a query returning a JSON column, then
/// `enter f y esc`.
#[test]
fn json_cell_enter_f_y_esc() {
    let db = FakeDatabase::default().reply(Reply::Rows(typed_result(
        &[("id", "int8"), ("doc", "jsonb")],
        &[&[Some("1"), Some(JSON)]],
    )));
    let mut h = AppHarness::new(Config::embedded()).with_database(db);
    h.send(UiEvent::Started);
    h.keys("tab")
        .type_str("select id, doc from docs")
        .keys("f8");
    h.advance(60_000);
    // F6 leaves the editor, where Tab indents.
    h.keys("f6 l");
    h.take_effects();
    h.keys("enter f y");
    assert_eq!(copied(&mut h), [JSON.to_string()]);
    assert!(h.render(80, 24).contains("copied value"));
    h.keys("esc");
    assert!(h.state().overlays.is_empty(), "{:?}", h.state().overlays);
}
