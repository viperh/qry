//! Unit tests for `snippets.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn snippet(name: &str, connection: Option<&str>) -> Snippet {
    Snippet {
        name: name.into(),
        sql: format!("SELECT '{name}'"),
        connection: connection.map(str::to_string),
        tags: Vec::new(),
    }
}

#[test]
fn names_unique_per_scope() {
    let list = vec![
        snippet("active users", None),
        snippet("orders", Some("prod")),
    ];
    assert_eq!(
        validate(&list, &snippet("Active Users ", None), None),
        Err(Invalid::Taken("Active Users".into()))
    );
    // Another scope may use the name.
    assert_eq!(
        validate(&list, &snippet("active users", Some("prod")), None),
        Ok(())
    );
    assert_eq!(validate(&list, &snippet("orders", None), None), Ok(()));
    // The snippet being replaced keeps its own name.
    assert_eq!(
        validate(&list, &snippet("orders", Some("prod")), Some(1)),
        Ok(())
    );
    assert_eq!(
        validate(&list, &snippet("  ", None), None),
        Err(Invalid::NoName)
    );
    let mut empty = snippet("x", None);
    empty.sql = " ".into();
    assert_eq!(validate(&list, &empty, None), Err(Invalid::NoSql));
}

#[test]
fn global_and_connection_scopes_listed_for_connection() {
    let list = vec![
        snippet("zeta", None),
        snippet("other", Some("dev")),
        snippet("mine", Some("prod")),
        snippet("alpha", None),
    ];
    let names: Vec<_> = for_connection(&list, "prod")
        .into_iter()
        .map(|(i, s)| (i, s.name.as_str()))
        .collect();
    assert_eq!(names, [(2, "mine"), (3, "alpha"), (0, "zeta")]);
}

#[test]
fn round_trip_json() {
    let mut tagged = snippet("active users", None);
    tagged.tags = vec!["users".into()];
    let list = vec![tagged, snippet("orders", Some("prod"))];
    assert_eq!(from_json(&to_json(&list)).unwrap(), list);
    // The documented shape reads, and missing optional keys default.
    let read = from_json(
        r#"[{"name":"active users","sql":"SELECT * FROM users WHERE active","connection":null,"tags":["users"]},
            {"name":"bare","sql":"SELECT 1"}]"#,
    )
    .unwrap();
    assert_eq!(read[0].tags, ["users"]);
    assert_eq!(read[1].connection, None);
    assert!(from_json("").unwrap().is_empty());
    assert!(from_json("{not json").is_err());
}
