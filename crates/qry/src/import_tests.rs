//! Unit tests for `import.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::connections::Secret;

fn import(text: &str) -> Imported {
    from_text(text, Format::Auto, Path::new("test.json")).unwrap()
}

/// A shared fixture from `tests/fixtures/` at the repository root.
fn fixture(relative: &str) -> String {
    std::fs::read_to_string(crate::testing::fixture(relative)).unwrap()
}

/// The shape SQL Developer's Export Connections writes. Values are strings,
/// the password is encrypted, and it is never read.
fn sql_developer() -> String {
    fixture("sqldeveloper/connections.json")
}

/// The shape of DBeaver's `data-sources.json`, from its reference docs.
fn dbeaver() -> String {
    fixture("dbeaver/data-sources.json")
}

fn only(imported: &Imported) -> &StoredConnection {
    assert_eq!(imported.connections.len(), 1, "{imported:#?}");
    &imported.connections[0]
}

#[test]
fn a_sql_developer_export_is_detected_and_read() {
    let imported = import(&sql_developer());
    assert_eq!(imported.format, Format::SqlDeveloper);
    let names: Vec<&str> = imported
        .connections
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["hr", "legacy", "url", "mysql"]);

    let hr = &imported.connections[0];
    assert_eq!(hr.driver, Driver::Oracle);
    assert_eq!(hr.host.as_deref(), Some("db.local"));
    assert_eq!(hr.port, None, "1521 is the default, so it is left out");
    assert_eq!(hr.service.as_deref(), Some("ORCLPDB1"));
    assert_eq!(hr.user.as_deref(), Some("hr"));
    assert_eq!(hr.secret, Secret::Prompt, "no password is ever imported");

    let legacy = &imported.connections[1];
    assert_eq!(
        (legacy.port, legacy.service.as_deref()),
        (Some(1522), Some("XE"))
    );
    assert!(
        imported.warnings.iter().any(|w| w.contains("SID XE")),
        "{:?}",
        imported.warnings
    );

    let url = &imported.connections[2];
    assert_eq!(url.host.as_deref(), Some("adb.example.com"));
    assert_eq!(url.service.as_deref(), Some("svc_high"));
    assert_eq!(url.tls.as_deref(), Some("verify-full"), "tcps:// means TLS");

    let mysql = &imported.connections[3];
    assert_eq!(mysql.driver, Driver::Mysql);
    assert_eq!(
        (mysql.port, mysql.database.as_deref()),
        (Some(3307), Some("shop"))
    );
    assert_eq!(mysql.user.as_deref(), Some("root"));

    let skipped: Vec<(&str, &str)> = imported
        .skipped
        .iter()
        .map(|s| (s.name.as_str(), s.reason.as_str()))
        .collect();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert!(
        skipped[0].0 == "tns" && skipped[0].1.contains("PRODDB"),
        "{skipped:?}"
    );
    assert!(
        skipped[1].0 == "kerb" && skipped[1].1.contains("Kerberos"),
        "{skipped:?}"
    );

    let written = serde_json::to_string(&imported.connections).unwrap();
    assert!(
        !written.contains("v2_mBJ"),
        "the encrypted password stays behind"
    );
}

#[test]
fn a_dbeaver_data_sources_file_is_detected_and_read() {
    let imported = import(&dbeaver());
    assert_eq!(imported.format, Format::DBeaver);
    let find = |name: &str| {
        imported
            .connections
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| {
                panic!("{name} missing: {imported:#?}");
            })
    };

    let prod = find("PostgreSQL - prod");
    assert_eq!(prod.driver, Driver::Postgres);
    assert_eq!(prod.host.as_deref(), Some("db.example.com"));
    assert_eq!((prod.port, prod.database.as_deref()), (None, Some("appdb")));
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("PostgreSQL - prod has no user")),
        "{:?}",
        imported.warnings
    );

    let maria = find("maria");
    assert_eq!(maria.driver, Driver::Mariadb);
    assert_eq!(maria.user.as_deref(), Some("alice"));

    let ora = find("ora");
    assert_eq!(
        (ora.driver, ora.service.as_deref()),
        (Driver::Oracle, Some("XE"))
    );
    assert!(imported.warnings.iter().any(|w| w.contains("SID XE")));

    let local = find("local");
    assert_eq!(local.driver, Driver::Sqlite);
    assert_eq!(local.path.as_deref(), Some("/home/me/app.db"));
    assert_eq!(local.mode.as_deref(), Some("ro"));
    assert_eq!(local.secret, Secret::None);

    let by_url = find("by url");
    assert_eq!(by_url.host.as_deref(), Some("::1"));
    assert_eq!(
        (by_url.port, by_url.database.as_deref()),
        (Some(6543), Some("reports"))
    );

    assert_eq!(imported.skipped.len(), 1);
    assert_eq!(imported.skipped[0].name, "docs");
    assert!(imported.skipped[0].reason.contains("mongodb"));
}

#[test]
fn a_qry_backup_keeps_ids_and_password_modes() {
    let mut record = StoredConnection::new("prod", Driver::Postgres);
    record.host = Some("db.local".into());
    record.user = Some("alice".into());
    record.database = Some("app".into());
    record.secret = Secret::Env {
        var: "PGPASSWORD".into(),
    };
    let text = serde_json::json!({"version": 1, "connections": [record]}).to_string();

    let imported = import(&text);
    assert_eq!(imported.format, Format::Qry);
    assert_eq!(
        only(&imported),
        &record,
        "a backup comes back exactly as written"
    );
}

#[test]
fn a_backup_from_a_newer_qry_is_refused() {
    let text = fixture("qry-backup/from-a-newer-qry.json");
    let err = from_text(&text, Format::Auto, Path::new("b.json")).unwrap_err();
    assert!(err.to_string().contains("newer qry"), "{err}");
}

#[test]
fn files_qry_cannot_read_say_why() {
    let path = Path::new("x");
    let err = |text: &str| from_text(text, Format::Auto, path).unwrap_err().to_string();
    assert!(err("<?xml version=\"1.0\"?><References/>").contains("XML"));
    assert!(err("not json").contains("not JSON"));
    assert!(err(r#"{"something": 1}"#).contains("not a qry backup"));

    // A format chosen by hand that the file does not match.
    let err = from_text(&dbeaver(), Format::SqlDeveloper, path)
        .unwrap_err()
        .to_string();
    assert!(err.contains("connections` array"), "{err}");
}

#[test]
fn a_zip_archive_is_explained() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let file = dir.join("project.dbp");
    std::fs::write(&file, b"PK\x03\x04rest").unwrap();
    let err = from_file(file.to_str().unwrap(), Format::Auto)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("zip archive") && err.contains("data-sources.json"),
        "{err}"
    );

    let err = from_file(dir.join("missing.json").to_str().unwrap(), Format::Auto)
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn jdbc_urls_for_each_database() {
    let (driver, target) = jdbc("jdbc:postgresql://pg.local/app?sslmode=require").unwrap();
    assert_eq!(driver, Driver::Postgres);
    assert_eq!(
        target,
        Target {
            host: Some("pg.local".into()),
            database: Some("app".into()),
            ..Target::default()
        }
    );

    let (driver, target) = jdbc("jdbc:mariadb://a.local:3310,b.local/shop").unwrap();
    assert_eq!(
        (driver, target.host.as_deref(), target.port),
        (Driver::Mariadb, Some("a.local"), Some(3310))
    );

    let (_, target) = jdbc("jdbc:oracle:thin:scott/tiger@db.local:1521:ORCL").unwrap();
    assert_eq!(target.user.as_deref(), Some("scott"));
    assert_eq!(
        (target.database.as_deref(), target.sid),
        (Some("ORCL"), true)
    );
    assert!(
        !format!("{target:?}").contains("tiger"),
        "the password is dropped"
    );

    let (_, target) = jdbc("jdbc:oracle:thin:@db.local:1521/svc").unwrap();
    assert_eq!(
        (target.database.as_deref(), target.sid, target.tls),
        (Some("svc"), false, false)
    );

    let (driver, target) = jdbc("jdbc:sqlite:file:/tmp/x.db").unwrap();
    assert_eq!(
        (driver, target.database.as_deref()),
        (Driver::Sqlite, Some("/tmp/x.db"))
    );

    for bad in [
        "jdbc:oracle:thin:@(DESCRIPTION=(ADDRESS=(HOST=x)))",
        "jdbc:oracle:thin:@PRODDB",
        "jdbc:oracle:oci:@db",
        "jdbc:sqlserver://x",
        "postgresql://x/y",
        "jdbc:postgresql://x:99999/y",
    ] {
        assert!(jdbc(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_tilde_means_the_home_directory() {
    let home = directories::BaseDirs::new()
        .unwrap()
        .home_dir()
        .to_path_buf();
    assert_eq!(expand_home("~/a/b.json"), home.join("a/b.json"));
    assert_eq!(expand_home(" /abs.json "), PathBuf::from("/abs.json"));
}
