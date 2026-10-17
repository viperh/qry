//! The canary: a password typed into qry never reaches a history or
//! snippet file, and one typed into SQL is redacted there.
//!
//! The app runs on the harness with a fake session. A password prompt is
//! answered with `CANARY-PW-7f3a`, statements run (one of them
//! `ALTER USER … PASSWORD 'CANARY-PW-2'`), a snippet is saved and qry
//! quits. Every file effect is carried out into temp data and config
//! directories, which are then scanned for both canaries.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{fs, path::Path};

use qry::{
    app::{DbAnswer, Effect},
    config::Config,
    history_store::{self, FileStore},
    testing::{AppHarness, Reply},
};
use qry_core::testing::FakeDatabase;

const TYPED: &str = "CANARY-PW-7f3a";
const IN_SQL: &str = "CANARY-PW-2";

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

#[test]
fn no_password_reaches_history_or_snippet_files() {
    let root = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let (data, config_dir) = (root.path().join("data"), root.path().join("config"));
    // The scratch session's history goes to disk too, so there is a file
    // to scan.
    let config =
        Config::with_user_file("config.json", r#"{"history": {"ephemeral_to_disk": true}}"#)
            .unwrap();
    let db = FakeDatabase::default()
        .reply(Reply::Affected(0))
        .reply(Reply::Affected(1))
        .reply(Reply::Error("syntax error".into()));
    let mut h = AppHarness::new(config).with_database(db);

    // The worker asks for a password, and the canary is typed.
    h.db(DbAnswer::NeedPassword {
        id: "scratch".into(),
        name: "scratch".into(),
    });
    h.type_str(TYPED).keys("enter");
    h.db(DbAnswer::Connected("fake".into()));

    h.keys("tab")
        .type_str(&format!(
            "ALTER USER x PASSWORD '{IN_SQL}';\nUPDATE t SET a = 1;\nSELEC 1"
        ))
        .keys("f8");
    // The statement under the cursor becomes a snippet.
    h.keys("ctrl-home down ctrl-s")
        .type_str("bump")
        .keys("enter");
    h.keys("ctrl-q");

    let store = FileStore::new(&data);
    let mut stashed = false;
    for effect in h.take_effects() {
        match effect {
            Effect::History(write) => history_store::perform(&store, &write).unwrap(),
            Effect::SaveSnippets(list) => history_store::save_snippets(&config_dir, &list).unwrap(),
            Effect::SaveConnections(list) => {
                fs::create_dir_all(&data).unwrap();
                fs::write(
                    data.join("connections.json"),
                    serde_json::to_string_pretty(&list).unwrap(),
                )
                .unwrap();
            }
            // The typed password goes to the in-memory stash, nowhere else.
            Effect::StashPassword { .. } => stashed = true,
            _ => {}
        }
    }
    assert!(stashed, "the prompt's answer was stashed");

    let written = files(root.path());
    assert!(
        written.iter().any(|p| p.ends_with("history/scratch.jsonl")),
        "{written:?}"
    );
    assert!(
        written.iter().any(|p| p.ends_with("snippets.json")),
        "{written:?}"
    );
    for path in &written {
        let text = String::from_utf8_lossy(&fs::read(path).unwrap()).into_owned();
        assert!(
            !text.contains(TYPED),
            "{} holds the typed password",
            path.display()
        );
        assert!(
            !text.contains(IN_SQL),
            "{} holds the password in SQL",
            path.display()
        );
    }
    let history = fs::read_to_string(data.join("history/scratch.jsonl")).unwrap();
    assert_eq!(history.lines().count(), 3, "{history}");
    assert!(history.contains("ALTER USER x PASSWORD '•••'"), "{history}");
    let snippets = fs::read_to_string(history_store::snippets_path(&config_dir)).unwrap();
    assert!(snippets.contains("UPDATE t SET a = 1"), "{snippets}");
}
