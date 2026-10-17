//! The query history's files: whole-line appends, atomic compaction,
//! private permissions, and nothing written when nothing should be. Temp
//! directories live under `CARGO_TARGET_TMPDIR`, never the real data dir.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{fs, path::Path, thread};

use qry::{
    app::Effect,
    config::Config,
    history_store::{self, FileStore, HistoryStore, HistoryWrite},
    testing::{AppHarness, Reply},
};
use qry_core::{
    history::{Entry, History, to_line},
    testing::FakeDatabase,
};

fn temp() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

fn line(sql: &str) -> String {
    to_line(&Entry {
        ts: Some(1_791_892_800),
        sql: sql.into(),
        ok: true,
        rows: Some(1),
        ms: 1,
        error: None,
    })
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

/// Runs `statements` on the scratch session with `config`, and carries out
/// the history writes the app asks for on a store in `data`.
fn run_and_write(config: Config, statements: &[&str], data: &Path) {
    let mut db = FakeDatabase::default();
    for _ in statements {
        db = db.reply(Reply::Affected(1));
    }
    let mut h = AppHarness::new(config).with_database(db);
    h.keys("tab");
    for sql in statements {
        h.type_str(sql).keys("f5 ctrl-a delete");
    }
    let store = FileStore::new(data);
    for effect in h.take_effects() {
        if let Effect::History(write) = effect {
            history_store::perform(&store, &write).unwrap();
        }
    }
}

#[test]
fn append_is_line_atomic_under_two_writers() {
    let dir = temp();
    let writers: Vec<_> = (0..2)
        .map(|w| {
            let store = FileStore::new(dir.path());
            thread::spawn(move || {
                for i in 0..500 {
                    // Long lines, so a torn write would show.
                    let sql = format!("SELECT {w}, {i}, '{}'", "x".repeat(300));
                    store.append("c1", &line(&sql)).unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let text = fs::read_to_string(dir.path().join("history/c1.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 1000);
    let (history, warnings) = History::from_jsonl(&text);
    assert!(warnings.is_empty(), "a line was broken: {warnings:?}");
    assert_eq!(history.len(), 1000);
}

#[test]
fn compaction_is_atomic_rename() {
    let dir = temp();
    let store = FileStore::new(dir.path());
    for i in 0..5 {
        store.append("c1", &line(&format!("SELECT {i}"))).unwrap();
    }
    let path = dir.path().join("history/c1.jsonl");
    #[cfg(unix)]
    let before = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&path).unwrap().ino()
    };
    let compacted = line("SELECT 4");
    assert!(store.rewrite("c1", &compacted).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), compacted);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_ne!(
            fs::metadata(&path).unwrap().ino(),
            before,
            "a new file renamed over the old, not the old one truncated"
        );
    }
    let left: Vec<_> = files(dir.path());
    assert_eq!(
        left,
        std::slice::from_ref(&path),
        "no temp or lock file is left behind"
    );

    // Another instance holding the lock: this one skips compacting.
    fs::write(dir.path().join("history/c1.lock"), "").unwrap();
    assert!(!store.rewrite("c1", "").unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), compacted);
}

#[cfg(unix)]
#[test]
fn files_created_0600_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp();
    let store = FileStore::new(dir.path());
    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    store.append("c1", &line("SELECT 1")).unwrap();
    assert_eq!(mode(&dir.path().join("history/c1.jsonl")), 0o600);
    assert_eq!(mode(&dir.path().join("history")), 0o700);
    store.rewrite("c2", &line("SELECT 2")).unwrap();
    assert_eq!(mode(&dir.path().join("history/c2.jsonl")), 0o600);
    history_store::save_snippets(&dir.path().join("config"), &[]).unwrap();
    assert_eq!(
        mode(&history_store::snippets_path(&dir.path().join("config"))),
        0o600
    );
}

#[test]
fn disabled_writes_nothing() {
    let dir = temp();
    let config = Config::with_user_file(
        "config.json",
        r#"{"history": {"enabled": false, "ephemeral_to_disk": true}}"#,
    )
    .unwrap();
    run_and_write(config, &["UPDATE t SET a = 1", "DELETE FROM t"], dir.path());
    assert!(files(dir.path()).is_empty());
    assert!(!dir.path().join("history").exists());
}

#[test]
fn ephemeral_connection_not_written() {
    let dir = temp();
    run_and_write(Config::embedded(), &["UPDATE t SET a = 1"], dir.path());
    assert!(
        files(dir.path()).is_empty(),
        "the scratch database stays in memory"
    );

    // The same run with the config asking for it does write.
    let config =
        Config::with_user_file("config.json", r#"{"history": {"ephemeral_to_disk": true}}"#)
            .unwrap();
    run_and_write(config, &["UPDATE t SET a = 1"], dir.path());
    let text = fs::read_to_string(dir.path().join("history/scratch.jsonl")).unwrap();
    assert!(text.contains("UPDATE t SET a = 1"), "{text}");
}

#[test]
fn a_corrupt_line_does_not_stop_the_rest_loading() {
    let dir = temp();
    let store = FileStore::new(dir.path());
    store.append("c1", &line("SELECT 1")).unwrap();
    store.append("c1", "{\"sql\":\"SEL").unwrap();
    store.append("c1", "\n").unwrap();
    store.append("c1", &line("SELECT 2")).unwrap();
    let loaded = history_store::load(&store).unwrap();
    let sql: Vec<_> = loaded[0]
        .history
        .entries()
        .map(|e| e.sql.as_str())
        .collect();
    assert_eq!(sql, ["SELECT 1", "SELECT 2"]);
    // A write the app asked for goes through the same store.
    history_store::perform(
        &store,
        &HistoryWrite::Rewrite {
            connection: "c1".into(),
            content: loaded[0].history.to_jsonl(),
        },
    )
    .unwrap();
    assert_eq!(
        history_store::load(&store).unwrap()[0].lines,
        2,
        "compaction drops the broken line"
    );
}
