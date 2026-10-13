//! Unit tests for `testing.rs`: the harness checks itself.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ratatui::text::{Line, Span};

use super::*;

fn plain(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn parse_keys_reads_chords_and_named_keys() {
    assert_eq!(
        parse_keys("ctrl-n esc shift-tab f8 enter j"),
        [
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
            plain(KeyCode::Esc),
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            plain(KeyCode::F(8)),
            plain(KeyCode::Enter),
            plain(KeyCode::Char('j')),
        ]
    );
    assert_eq!(
        parse_keys("G"),
        [KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)]
    );
    assert!(parse_keys("  ").is_empty());

    let panic = std::panic::catch_unwind(|| parse_keys("j ctrl-foo")).unwrap_err();
    let message = panic.downcast_ref::<String>().unwrap();
    assert!(message.contains("`ctrl-foo`"), "{message}");
}

#[test]
fn buffer_to_string_keeps_trailing_spaces_and_row_count() {
    let buffer = render_with(5, 3, |frame| {
        frame.render_widget(Line::raw("ab"), frame.area());
    });
    assert_eq!(buffer_to_string(&buffer), "ab   \n     \n     \n");
    assert_eq!(buffer_lines(&buffer), ["ab   ", "     ", "     "]);
}

#[test]
fn styled_lists_one_run_per_style_change() {
    let buffer = render_with(6, 1, |frame| {
        let line = Line::from(vec![
            Span::styled("ab", Theme::DARK.value_error()),
            Span::styled("cd", Theme::DARK.editor_selection()),
        ]);
        frame.render_widget(line, frame.area());
    });
    assert_eq!(
        styled(&buffer),
        "abcd  \n\
         --- styles (x,y fg bg modifiers) ---\n\
         0,0 Rgb(242, 139, 107) Reset NONE\n\
         2,0 Reset Rgb(27, 37, 51) NONE\n\
         4,0 Reset Reset NONE\n"
    );
}

#[test]
fn keys_open_the_new_connection_modal() {
    let mut h = AppHarness::new(Config::embedded());
    assert!(!h.render(80, 24).contains("New Connection"));

    h.keys("ctrl-n");
    assert_eq!(h.dialog_title(), Some("New Connection"));
    assert_eq!(h.contexts(), [KeyContext::Dialog]);
    assert!(h.render(80, 24).contains("New Connection"));

    // Esc is bound in the dialogs' context, so it closes it again.
    h.keys("esc");
    assert_eq!(h.dialog_title(), None);
    assert_eq!(h.contexts(), [KeyContext::Tree, KeyContext::Global]);
    assert!(!h.render(80, 24).contains("New Connection"));
    assert!(h.take_effects().is_empty());
}

#[test]
fn io_actions_become_effects_not_dispatches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rows.csv").display().to_string();
    let mut h = AppHarness::new(Config::embedded());

    // ctrl-s in the results opens the Export modal.
    h.keys("shift-tab ctrl-s").type_str(&path).keys("enter");

    let effects = h.take_effects();
    let [Effect::Db(_, DbCommand::Export(config))] = effects.as_slice() else {
        panic!("expected one export, got {effects:?}");
    };
    assert_eq!(config.path, path);
    // Nothing carried it out: the modal still waits and the disk is untouched.
    assert!(h.render(80, 24).contains("Exporting"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test(start_paused = true)]
async fn advance_delivers_ticks_without_real_time() {
    let wall = std::time::Instant::now();
    let mut h = AppHarness::new(Config::embedded());
    assert_eq!(h.ticks(), 0);

    h.advance(1000);
    assert_eq!(h.ticks(), 4, "4 Hz for one second");
    assert_eq!(h.now(), Duration::from_secs(1));

    // A partial interval delivers nothing until it is complete.
    h.advance(100);
    assert_eq!(h.ticks(), 4);
    h.advance(150);
    assert_eq!(h.ticks(), 5);

    assert!(wall.elapsed() < Duration::from_millis(50));
}

#[test]
fn the_apps_timers_fire_on_the_virtual_clock() {
    let mut h = AppHarness::new(Config::embedded());
    // `g` starts `g g` in the tree, and waits a second for the rest.
    h.keys("g");
    assert!(matches!(
        h.take_effects().as_slice(),
        [Effect::ScheduleTimer { .. }]
    ));
    assert_eq!(h.state().pending_keys.len(), 1);
    h.advance(999);
    assert_eq!(h.state().pending_keys.len(), 1);
    h.advance(1);
    assert!(h.state().pending_keys.is_empty());
}

#[test]
fn a_fake_database_answers_queries_on_the_virtual_clock() {
    let db = FakeDatabase::default()
        .reply(Reply::Delay(
            Duration::from_millis(500),
            Box::new(Reply::Rows(grid(2, 2))),
        ))
        .reply(Reply::Error("no such table: nope".into()));
    let mut h = AppHarness::new(Config::embedded()).with_database(db.clone());

    // The editor's run key sends what is typed.
    h.keys("tab").type_str("SELECT * FROM t").keys("f8");
    h.keys("ctrl-a backspace")
        .type_str("SELECT * FROM nope")
        .keys("f8");
    assert_eq!(db.calls(), ["SELECT * FROM t", "SELECT * FROM nope"]);
    assert!(
        !h.render(80, 24).contains("r1c1"),
        "answered before its delay"
    );

    h.advance(500);
    let text = h.render(80, 24);
    assert!(text.contains("r1c1"), "{text}");
    // The second answer waited for the first, as the worker runs in order;
    // its toast waits behind the first one's.
    assert!(text.contains("(+1)"), "{text}");
    h.advance(4000);
    assert!(h.render(80, 24).contains("no such table: nope"));
    assert_eq!(h.take_effects().len(), 2);
}

#[test]
fn every_view_renders_at_every_size() {
    // Each view, and how to open it from the main screen.
    type Open = fn(&mut AppHarness);
    let views: [(&str, Open); 7] = [
        ("home", |_| {}),
        ("new connection", |h| _ = h.keys("ctrl-n")),
        ("export", |h| _ = h.keys("shift-tab ctrl-s")),
        ("password", |h| {
            h.db(DbAnswer::NeedPassword {
                id: "c7f3a1e2".into(),
                name: "prod".into(),
            });
        }),
        ("import", |h| _ = h.keys("ctrl-o")),
        ("export connections", |h| _ = h.keys("ctrl-s")),
        ("help", |h| _ = h.keys("f1")),
    ];
    for &(w, h) in SIZES {
        for (view, open) in views {
            let mut harness = AppHarness::new(Config::embedded());
            open(&mut harness);
            assert_eq!(
                harness.state().overlays.is_empty(),
                view == "home",
                "{view} did not open"
            );
            assert_screen(&harness.render(w, h), w, h, view);
        }
    }
}

/// `text` is exactly `h` lines of `w` cells each.
fn assert_screen(text: &str, w: u16, h: u16, view: &str) {
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), usize::from(h), "{view} at {w}x{h}:\n{text}");
    for line in lines {
        assert_eq!(
            line.chars().count(),
            usize::from(w),
            "{view} at {w}x{h}: {line:?}"
        );
    }
}
