//! Unit tests for `connections.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tempfile::{TempDir, tempdir};

use super::*;

/// A directory of its own per test, removed when it is dropped.
fn temp_dir() -> TempDir {
    tempdir().unwrap()
}

fn postgres() -> StoredConnection {
    StoredConnection {
        host: Some("db.local".into()),
        user: Some("alice".into()),
        database: Some("app".into()),
        secret: Secret::Keyring,
        ..StoredConnection::new("prod", Driver::Postgres)
    }
}

#[test]
fn ids_are_unique() {
    let ids: Vec<String> = (0..100).map(|_| mint_id()).collect();
    assert!(ids.iter().all(|id| id.len() == 8));
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), ids.len());
}

#[test]
fn a_missing_file_is_an_empty_tree() {
    let dir = temp_dir();
    assert!(load_from(dir.path()).unwrap().is_empty());
}

#[test]
fn what_is_saved_comes_back() {
    let dir = temp_dir();
    let connections = vec![postgres(), StoredConnection::new("scratch", Driver::Sqlite)];
    save_to(dir.path(), &connections).unwrap();

    assert_eq!(load_from(dir.path()).unwrap(), connections);
    // The temporary file does not outlive the save.
    assert!(!dir.path().join(TEMP).exists());
}

#[test]
fn the_file_holds_no_password_and_omits_fields_that_do_not_apply() {
    let dir = temp_dir();
    let mut connection = postgres();
    connection.port = Some(5432);
    save_to(dir.path(), &[connection]).unwrap();

    let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
    assert!(!text.contains("password"), "{text}");
    assert!(text.contains("\"driver\": \"postgres\""), "{text}");
    assert!(text.contains("\"mode\": \"keyring\""), "{text}");
    // SQLite fields have no place on a Postgres record.
    assert!(!text.contains("\"path\""), "{text}");
    assert!(!text.contains("\"service\""), "{text}");
}

#[test]
fn fields_from_a_newer_qry_survive_a_save() {
    let dir = temp_dir();
    fs::write(
        dir.path().join(FILE),
        r#"{
          "version": 1,
          "future_setting": true,
          "connections": [
            {"id": "c7f3a1e2", "name": "prod", "driver": "postgres", "host": "h",
             "user": "u", "database": "d", "secret": {"mode": "prompt"},
             "colour": "red"}
          ]
        }"#,
    )
    .unwrap();

    let mut loaded = load_from(dir.path()).unwrap();
    assert_eq!(
        loaded[0].extra.get("colour").and_then(Value::as_str),
        Some("red")
    );
    loaded[0].name = "renamed".into();
    save_to(dir.path(), &loaded).unwrap();

    let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
    assert!(text.contains("future_setting"), "{text}");
    assert!(text.contains("\"colour\": \"red\""), "{text}");
    assert!(text.contains("renamed"), "{text}");
}

#[test]
fn a_newer_version_is_refused_and_the_file_left_alone() {
    let dir = temp_dir();
    let original = r#"{"version": 99, "connections": []}"#;
    fs::write(dir.path().join(FILE), original).unwrap();

    let err = load_from(dir.path()).unwrap_err().to_string();
    assert!(err.contains("newer qry"), "{err}");
    assert_eq!(fs::read_to_string(dir.path().join(FILE)).unwrap(), original);
}

#[test]
fn a_malformed_file_is_an_error_and_is_left_alone() {
    let dir = temp_dir();
    fs::write(dir.path().join(FILE), "{ not json").unwrap();

    let err = load_from(dir.path()).unwrap_err().to_string();
    assert!(err.contains("connection file"), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join(FILE)).unwrap(),
        "{ not json"
    );
}

#[test]
fn a_record_builds_what_the_driver_needs() {
    let ConnectionConfig::Postgres(config) = postgres().to_config("hunter2").unwrap() else {
        panic!("expected Postgres");
    };
    assert_eq!((config.port, config.password.as_str()), (5432, "hunter2"));
    assert_eq!(config.ssl_mode, SslMode::Prefer);

    let mut sqlite = StoredConnection::new("scratch", Driver::Sqlite);
    sqlite.path = Some("app.db".into());
    sqlite.mode = Some("ro".into());
    let ConnectionConfig::Sqlite(config) = sqlite.to_config("").unwrap() else {
        panic!("expected Sqlite");
    };
    assert!(config.read_only);
}

fn oracle() -> StoredConnection {
    let mut connection = StoredConnection::new("erp", Driver::Oracle);
    connection.host = Some("db.local".into());
    connection.user = Some("alice".into());
    connection.service = Some("ORCLPDB1".into());
    connection
}

fn oracle_config(connection: &StoredConnection) -> OracleConfig {
    let ConnectionConfig::Oracle(config) = connection.to_config("x").unwrap() else {
        panic!("expected Oracle");
    };
    config
}

#[test]
fn an_oracle_record_carries_its_ssl_mode_and_wallet() {
    let config = oracle_config(&oracle());
    assert_eq!(
        (config.port, config.ssl_mode, config.wallet),
        (1521, SslMode::Prefer, None)
    );

    let mut connection = oracle();
    connection.tls = Some("verify-full".into());
    connection.wallet = Some("/etc/oracle/wallet".into());
    let config = oracle_config(&connection);
    assert_eq!(config.ssl_mode, SslMode::VerifyFull);
    assert_eq!(config.wallet.as_deref(), Some("/etc/oracle/wallet"));
}

#[test]
fn an_old_oracle_record_with_a_wallet_in_tls_means_tcps() {
    let text = r#"{"id":"c1","name":"erp","driver":"oracle","host":"db.local",
        "user":"alice","service":"ORCLPDB1","tls":"/etc/oracle/wallet",
        "secret":{"mode":"prompt"}}"#;
    let connection: StoredConnection = serde_json::from_str(text).unwrap();
    let config = oracle_config(&connection);
    assert_eq!(config.ssl_mode, SslMode::VerifyFull);
    assert_eq!(config.wallet.as_deref(), Some("/etc/oracle/wallet"));
}

#[test]
fn an_export_is_a_backup_the_importer_reads_back() {
    let dir = temp_dir();
    let mut kept = postgres();
    kept.secret = Secret::Keyring;
    let list = vec![kept, oracle()];

    // No extension: `.json` is added.
    let written = export(&dir.path().join("backup"), &list).unwrap();
    assert_eq!(written, dir.path().join("backup.json"));
    let text = fs::read_to_string(&written).unwrap();
    assert!(
        !text.contains("hunter2"),
        "a record has nowhere to keep a password"
    );
    assert!(
        !dir.path().join("backup.json.tmp").exists(),
        "the temp file was renamed away"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&written).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let imported =
        crate::import::from_file(written.to_str().unwrap(), crate::import::Format::Auto).unwrap();
    assert_eq!(imported.format, crate::import::Format::Qry);
    assert_eq!(
        imported.connections, list,
        "it comes back exactly as written"
    );

    // Writing it again replaces the old backup.
    export(&written, &list[..1]).unwrap();
    assert_eq!(
        parse(&fs::read_to_string(&written).unwrap(), &written)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn an_export_never_overwrites_a_file_that_is_not_a_backup() {
    let dir = temp_dir();
    let other = dir.path().join("notes.txt");
    fs::write(&other, "my notes").unwrap();
    let err = export(&other, &[postgres()]).unwrap_err().to_string();
    assert!(err.contains("not a qry backup"), "{err}");
    assert_eq!(fs::read_to_string(&other).unwrap(), "my notes");

    let err = export(&dir.path().join("no/such/dir/b.json"), &[postgres()]).unwrap_err();
    assert!(err.to_string().contains("cannot write"), "{err}");
}

#[test]
fn a_record_missing_a_field_says_which() {
    let mut connection = postgres();
    connection.host = None;
    let err = connection.to_config("x").unwrap_err().to_string();
    assert!(err.contains("needs a host"), "{err}");

    connection = postgres();
    connection.tls = Some("nonsense".into());
    assert!(
        connection
            .to_config("x")
            .unwrap_err()
            .to_string()
            .contains("not an SSL mode")
    );

    connection = postgres();
    connection.dsn = Some("postgres://db/app".into());
    assert!(
        connection
            .to_config("x")
            .unwrap_err()
            .to_string()
            .contains("DSN")
    );
}
