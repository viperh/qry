//! The reducer is a function of its state and the event: the same events
//! into two copies of one app give the same effects and the same state.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proptest::prelude::*;
use qry_core::schema::{Catalog, Children, DbObject, ObjectKind, SchemaPath};

use super::*;
use crate::{
    connections::{Driver, Secret, StoredConnection},
    import::{Format, Imported},
    testing::{grid, instant},
};

/// No single event may ask for more than this many effects.
const MAX_EFFECTS: usize = 64;

/// Keys that do something somewhere: the global ones, the panes', the
/// forms' and plain typing.
fn key() -> impl Strategy<Value = KeyEvent> {
    let plain = [
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::F(1),
        KeyCode::F(5),
        KeyCode::F(8),
        KeyCode::Char(' '),
        KeyCode::Char('?'),
        KeyCode::Char(':'),
        KeyCode::Char('G'),
        KeyCode::Char('e'),
        KeyCode::Char('g'),
        KeyCode::Char('x'),
        KeyCode::Char('1'),
        KeyCode::Char('/'),
        KeyCode::Char('.'),
        KeyCode::Char('D'),
        KeyCode::Char('R'),
    ];
    let ctrl = [
        'n', 'e', 'o', 's', 'a', 'w', 'x', 'y', 'z', 'c', 'v', 'p', '3',
    ];
    prop_oneof![
        proptest::sample::select(plain.to_vec()).prop_map(|code| {
            let modifiers = if code == KeyCode::BackTab {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            KeyEvent::new(code, modifiers)
        }),
        proptest::sample::select(ctrl.to_vec())
            .prop_map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)),
        proptest::char::range('a', 'z')
            .prop_map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
    ]
}

/// A saved connection with a fixed id, so a failing case replays exactly.
fn record(n: u8) -> StoredConnection {
    let mut record = StoredConnection::with_id(format!("c{n}"), format!("db{n}"), Driver::Sqlite);
    record.path = Some(format!("{n}.db"));
    record.secret = if n.is_multiple_of(2) {
        Secret::None
    } else {
        Secret::Keyring
    };
    record
}

/// Answers a worker could give, at any moment.
fn db_answer() -> impl Strategy<Value = DbAnswer> {
    prop_oneof![
        Just(DbAnswer::Connected(":memory:".into())),
        Just(DbAnswer::ConnectFailed("no such file".into())),
        Just(DbAnswer::NeedPassword {
            id: "c1".into(),
            name: "db1".into()
        }),
        Just(DbAnswer::SecretNotStored("c1".into())),
        (1..30usize, 1..6usize)
            .prop_map(|(rows, cols)| DbAnswer::QueryDone(Arc::new(grid(rows, cols)))),
        Just(DbAnswer::Exported("rows.csv".into())),
        Just(DbAnswer::ExportFailed("cannot write".into())),
        (0..4u64).prop_map(|generation| DbAnswer::Schema {
            path: SchemaPath::Catalogs,
            generation,
            outcome: Ok(Children::Catalogs(vec![Catalog {
                name: "main".into(),
                is_default: true,
                is_system: false,
            }])),
        }),
        (0..4u64).prop_map(|generation| DbAnswer::Schema {
            path: SchemaPath::objects("main"),
            generation,
            outcome: Ok(Children::Objects(
                ["users", "orders"]
                    .into_iter()
                    .map(|name| DbObject {
                        catalog: "main".into(),
                        name: name.into(),
                        kind: ObjectKind::Table,
                    })
                    .collect()
            )),
        }),
        (0..4u64).prop_map(|generation| DbAnswer::Schema {
            path: SchemaPath::details("main", "users"),
            generation,
            outcome: Err("connection lost".into()),
        }),
        Just(DbAnswer::Notice(crate::notice::Notice::error("e"))),
        Just(DbAnswer::WorkerStopped),
    ]
}

/// Outcomes of the runtime's file effects, at any moment.
fn file_answer() -> impl Strategy<Value = FileAnswer> {
    prop_oneof![
        (0..4u8).prop_map(|n| FileAnswer::ConnectionsLoaded((0..n).map(record).collect())),
        (0..4u8).prop_map(|n| FileAnswer::Imported(Box::new(Imported {
            format: Format::Qry,
            connections: (n..n + 2).map(record).collect(),
            skipped: Vec::new(),
            warnings: Vec::new(),
        }))),
        Just(FileAnswer::ImportFailed("cannot read".into())),
        Just(FileAnswer::ConnectionsExported {
            path: "b.json".into(),
            count: 1
        }),
        Just(FileAnswer::ConnectionsExportFailed("cannot write".into())),
        Just(FileAnswer::SaveFailed("cannot save".into())),
    ]
}

fn event() -> impl Strategy<Value = UiEvent> {
    prop_oneof![
        8 => key().prop_map(UiEvent::Key),
        1 => (0..300u16, 0..100u16).prop_map(|(w, h)| UiEvent::Resize(w, h)),
        1 => (0..10_000u64).prop_map(|ms| UiEvent::Tick(instant(ms))),
        // The live key-sequence timer, or a stale or future one.
        1 => (0..6u64).prop_map(|n| UiEvent::Timer(Timer::KeySequence(n))),
        1 => (0..6u64).prop_map(|n| UiEvent::Timer(Timer::Tree(n))),
        2 => db_answer().prop_map(|answer| UiEvent::Db(SessionId::FIRST, answer)),
        1 => file_answer().prop_map(UiEvent::File),
        1 => Just(UiEvent::Started),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn reducer_is_deterministic(events in proptest::collection::vec(event(), 200)) {
        let mut one = App::embedded();
        let mut two = one.clone();
        for event in events {
            let a = one.handle(event.clone());
            let b = two.handle(event.clone());
            prop_assert_eq!(a, b, "after {:?}", event);
        }
        prop_assert!(one == two, "the states differ");
    }

    #[test]
    fn reducer_bounds_effects_per_event(events in proptest::collection::vec(event(), 200)) {
        let mut app = App::embedded();
        for event in events {
            let count = app.handle(event.clone()).len();
            prop_assert!(count <= MAX_EFFECTS, "{count} effects for {event:?}");
        }
    }
}
