//! Unit tests for `history.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;

use super::*;

fn entry(sql: &str, ts: i64) -> Entry {
    Entry {
        ts: Some(ts),
        sql: sql.into(),
        ok: true,
        rows: Some(1),
        ms: 4,
        error: None,
    }
}

fn redact_default(dialect: Dialect, sql: &str) -> String {
    redact(dialect, sql, &DEFAULT_REDACT)
}

#[test]
fn push_dedupes_consecutive_normalised() {
    let mut h = History::default();
    assert!(!h.push(entry("SELECT 1", 10)));
    let mut again = entry("  SELECT\n   1 ", 20);
    again.ok = false;
    again.ms = 9;
    again.error = Some("gone".into());
    assert!(h.push(again));
    assert_eq!(h.len(), 1);
    let last = h.get(0).unwrap();
    assert_eq!(
        (last.ts, last.ok, last.ms, last.sql.as_str()),
        (Some(20), false, 9, "SELECT 1")
    );
    // Only consecutive ones merge.
    h.push(entry("SELECT 2", 30));
    h.push(entry("SELECT 1", 40));
    assert_eq!(h.len(), 3);
}

#[test]
fn push_enforces_cap_dropping_oldest() {
    let mut h = History::new(3);
    for i in 0..5 {
        h.push(entry(&format!("SELECT {i}"), i));
    }
    let sql: Vec<_> = h.entries().map(|e| e.sql.as_str()).collect();
    assert_eq!(sql, ["SELECT 2", "SELECT 3", "SELECT 4"]);
    h.set_cap(1);
    assert_eq!(h.len(), 1);
    assert_eq!(h.get(0).unwrap().sql, "SELECT 4");
}

#[test]
fn jsonl_round_trip() {
    let mut h = History::default();
    h.push(entry("SELECT * FROM users WHERE id = 3", 1_791_890_000));
    h.push(Entry {
        ts: None,
        sql: "DROP TABLE tmp\n-- \"quoted\" ✓".into(),
        ok: false,
        rows: None,
        ms: 2,
        error: Some("table \"tmp\" does not exist".into()),
    });
    let text = h.to_jsonl();
    assert_eq!(text.lines().count(), 2);
    assert!(text.ends_with('\n'));
    let (back, warnings) = History::from_jsonl(&text);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back, h);
}

#[test]
fn documented_lines_read() {
    let text = concat!(
        r#"{"ts":"2026-10-13T09:12:44Z","sql":"SELECT * FROM users WHERE id = 3","ok":true,"rows":1,"ms":4}"#,
        "\n",
        r#"{"ts":"2026-10-13T09:13:02Z","sql":"DROP TABLE tmp","ok":false,"error":"table \"tmp\" does not exist","ms":2}"#,
        "\n",
    );
    let (h, warnings) = History::from_jsonl(text);
    assert!(warnings.is_empty());
    assert_eq!(
        h.get(0).unwrap().ts.map(format_ts).unwrap(),
        "2026-10-13T09:12:44Z"
    );
    assert_eq!(
        h.get(1).unwrap().error.as_deref(),
        Some("table \"tmp\" does not exist")
    );
    // Written back, the lines are the same.
    assert_eq!(h.to_jsonl(), text);
}

#[test]
fn from_jsonl_skips_corrupt_lines_with_warning() {
    let text = concat!(
        r#"{"ts":"2026-10-13T09:12:44Z","sql":"SELECT 1","ok":true,"ms":4}"#,
        "\n",
        "garbage\n",
        r#"{"ts":"not a time","sql":"SELECT 2","ok":true,"ms":1}"#,
        "\n",
        // A crash mid-append leaves half a line.
        r#"{"ts":"2026-10-13T09:13:02Z","sql":"SEL"#,
    );
    let (h, warnings) = History::from_jsonl(text);
    let sql: Vec<_> = h.entries().map(|e| e.sql.as_str()).collect();
    assert_eq!(sql, ["SELECT 1", "SELECT 2"]);
    assert_eq!(
        h.get(1).unwrap().ts,
        None,
        "an invalid time is kept as unknown"
    );
    let lines: Vec<_> = warnings.iter().map(|w| w.line).collect();
    assert_eq!(lines, [2, 4]);
}

#[test]
fn search_scores_subsequence_and_returns_match_positions() {
    let mut h = History::default();
    h.push(entry("SELECT * FROM users", 1));
    h.push(entry("DELETE FROM sessions", 2));
    h.push(entry("SELECT\nname FROM usr", 3));
    let found = h.search("sel usr");
    let order: Vec<_> = found.iter().map(|(i, _, _)| *i).collect();
    assert_eq!(
        order,
        [2, 0],
        "both selects match; the delete has no `sel … usr`"
    );
    let (_, _, positions) = &found[0];
    let line: Vec<char> = one_line(&h.get(2).unwrap().sql).chars().collect();
    let picked: String = positions.iter().map(|&p| line[p]).collect();
    assert_eq!(picked.to_lowercase(), "sel usr");
    // Empty: everything, newest first.
    let all: Vec<_> = h.search("").into_iter().map(|(i, _, _)| i).collect();
    assert_eq!(all, [2, 1, 0]);
}

#[test]
fn redact_password_literal_postgres_alter_user() {
    assert_eq!(
        redact_default(Dialect::Postgres, "ALTER USER bob WITH PASSWORD 'hunter2'"),
        "ALTER USER bob WITH PASSWORD '•••'"
    );
    assert_eq!(
        redact_default(
            Dialect::Postgres,
            "create role x login password 'a''b' valid until 'infinity'"
        ),
        "create role x login password '•••' valid until 'infinity'"
    );
    assert_eq!(
        redact_default(Dialect::MySql, "SET PASSWORD FOR 'u'@'%' = 'pw'"),
        "SET PASSWORD FOR 'u'@'%' = '•••'"
    );
}

#[test]
fn redact_identified_by_oracle() {
    assert_eq!(
        redact_default(Dialect::Oracle, "ALTER USER scott IDENTIFIED BY tiger"),
        "ALTER USER scott IDENTIFIED BY '•••'"
    );
    assert_eq!(
        redact_default(Dialect::Oracle, "CREATE USER a IDENTIFIED BY \"Pa ss\""),
        "CREATE USER a IDENTIFIED BY '•••'"
    );
    assert_eq!(
        redact_default(Dialect::MySql, "CREATE USER 'a'@'h' IDENTIFIED BY 'pw'"),
        "CREATE USER 'a'@'h' IDENTIFIED BY '•••'"
    );
}

#[test]
fn redact_ignores_password_word_inside_string() {
    for sql in [
        "SELECT 'my password is x' FROM t",
        "SELECT password FROM users",
        "-- password 'x'\nSELECT 1",
    ] {
        assert_eq!(redact_default(Dialect::Postgres, sql), sql);
    }
}

#[test]
fn redact_keeps_non_secret_literals() {
    let sql = "UPDATE users SET name = 'bob', note = 'secretive' WHERE id = 1";
    assert_eq!(redact_default(Dialect::Postgres, sql), sql);
    assert_eq!(
        redact_default(
            Dialect::Postgres,
            "CREATE SECRET s (TYPE s3, KEY_ID 'k', SECRET 'v')"
        ),
        "CREATE SECRET s (TYPE s3, KEY_ID 'k', SECRET '•••')"
    );
}

#[test]
fn timestamps_format_and_parse() {
    assert_eq!(format_ts(0), "1970-01-01T00:00:00Z");
    let t = parse_ts("2026-10-13T09:12:44Z").unwrap();
    assert_eq!(format_ts(t), "2026-10-13T09:12:44Z");
    assert_eq!(parse_ts("2026-10-13T11:12:44.250+02:00"), Some(t));
    assert_eq!(
        format_ts(parse_ts("2024-02-29T23:59:59Z").unwrap()),
        "2024-02-29T23:59:59Z"
    );
    for bad in [
        "",
        "2026-13-01T00:00:00Z",
        "yesterday",
        "2026-10-13T09:12:44",
    ] {
        assert_eq!(parse_ts(bad), None, "{bad}");
    }
}

#[test]
fn relative_time_today_yesterday_then_date() {
    let now = parse_ts("2026-10-13T12:00:00Z").unwrap();
    let at = |s| parse_ts(s);
    assert_eq!(relative(at("2026-10-13T09:12:44Z"), now, 0), "09:12");
    assert_eq!(relative(at("2026-10-12T23:59:00Z"), now, 0), "yday");
    assert_eq!(relative(at("2026-10-03T08:00:00Z"), now, 0), "Oct 03");
    assert_eq!(relative(None, now, 0), "?");
    // Local midnight decides the day: 23:30 UTC is tomorrow at +02:00.
    assert_eq!(relative(at("2026-10-12T23:30:00Z"), now, 7200), "01:30");
}

proptest! {
    /// The fuzz twin for an untrusted file on disk.
    #[test]
    fn prop_from_jsonl_never_panics(input in any::<String>()) {
        let (h, _) = History::from_jsonl(&input);
        let _ = History::from_jsonl(&h.to_jsonl());
    }

    #[test]
    fn prop_from_jsonl_never_panics_on_json_shaped_lines(
        lines in proptest::collection::vec(r#"\{("(ts|sql|ok|ms|rows|error)":("[^"\\]{0,12}"|[0-9]{1,4}|true|null),?){0,6}\}?"#, 0..8)
    ) {
        let _ = History::from_jsonl(&lines.join("\n"));
    }

    #[test]
    fn prop_redact_preserves_length_class_and_never_leaks_literal(
        password in "[A-Za-z0-9!#%&*+.:-]{8,24}",
        dialect in prop::sample::select(Dialect::ALL.to_vec()),
    ) {
        let sql = format!("ALTER USER bob WITH PASSWORD '{password}'");
        prop_assume!(!"ALTER USER bob WITH PASSWORD '•••'".contains(&password));
        let out = redact_default(dialect, &sql);
        prop_assert!(!out.contains(&password), "{out}");
        // Everything before the literal is kept, and the literal is one
        // fixed-size mask whatever its length.
        prop_assert_eq!(out, "ALTER USER bob WITH PASSWORD '•••'");
    }
}
