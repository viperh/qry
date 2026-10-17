//! Unit tests for `history_store.rs`. The file behaviour (atomic appends,
//! compaction, permissions) is in `tests/history_store.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn file_names_round_trip_and_stay_in_the_directory() {
    for id in ["3fa2c1d0", "scratch", "../../etc/passwd", "a b/ç"] {
        let stem = file_stem(id);
        assert!(
            stem.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_%".contains(&b)),
            "{stem}"
        );
        assert_eq!(connection_of(&stem).as_deref(), Some(id));
    }
    assert_eq!(file_stem("3fa2c1d0"), "3fa2c1d0");
    assert_eq!(connection_of("bad%Z"), None);
}

#[test]
fn memory_store_appends_rewrites_and_loads() {
    let store = MemoryStore::default();
    let line = qry_core::history::to_line(&qry_core::history::Entry {
        ts: Some(0),
        sql: "SELECT 1".into(),
        ok: true,
        rows: Some(1),
        ms: 1,
        error: None,
    });
    perform(
        &store,
        &HistoryWrite::Append {
            connection: "a".into(),
            line: line.clone(),
        },
    )
    .unwrap();
    perform(
        &store,
        &HistoryWrite::Append {
            connection: "a".into(),
            line: "half a li".into(),
        },
    )
    .unwrap();
    let loaded = load(&store).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].history.len(), 1, "the broken line is skipped");
    assert_eq!(loaded[0].lines, 2);
    perform(
        &store,
        &HistoryWrite::Rewrite {
            connection: "a".into(),
            content: line.clone(),
        },
    )
    .unwrap();
    assert_eq!(store.file("a"), Some(line));
}
