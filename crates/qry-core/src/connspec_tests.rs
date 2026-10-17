#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn kinds(engine: Engine) -> Vec<FieldKind> {
    fields(engine).iter().map(|f| f.kind).collect()
}

fn draft(engine: Engine) -> Draft {
    let mut draft = Draft::new(engine);
    draft.name = "local-app".into();
    for (kind, value) in [
        (FieldKind::Host, "localhost"),
        (FieldKind::User, "app"),
        (FieldKind::Database, "app"),
        (FieldKind::Service, "ORCLPDB1"),
        (FieldKind::Path, "~/app.db"),
    ] {
        if kinds(engine).contains(&kind) {
            draft.values.insert(kind, value.into());
        }
    }
    draft
}

fn errors_for(draft: &Draft, taken: &[&str], field: FieldKind) -> Vec<String> {
    validate(draft, taken)
        .into_iter()
        .filter(|e| e.field == field)
        .map(|e| e.message)
        .collect()
}

#[test]
fn fields_per_driver_match_table() {
    use FieldKind::*;
    assert_eq!(
        kinds(Engine::Postgres),
        [Host, Port, User, Password, Database, Schema, Ssl]
    );
    assert_eq!(
        kinds(Engine::Mysql),
        [Host, Port, User, Password, Database, Ssl]
    );
    assert_eq!(kinds(Engine::Mariadb), kinds(Engine::Mysql));
    assert_eq!(kinds(Engine::Sqlite), [Path, Mode]);
    assert_eq!(
        kinds(Engine::Oracle),
        [Host, Port, User, Password, Service, Schema, Ssl, Wallet]
    );

    // The defaults the dialog starts with.
    let port = |e: Engine| Draft::new(e).value(Port).to_string();
    assert_eq!(port(Engine::Postgres), "5432");
    assert_eq!(port(Engine::Mysql), "3306");
    assert_eq!(port(Engine::Oracle), "1521");
    assert_eq!(Draft::new(Engine::Sqlite).value(Mode), "rw");
    assert_eq!(ssl_modes(Engine::Postgres).len(), 6);
    assert_eq!(
        ssl_modes(Engine::Mysql),
        ["disable", "prefer", "require", "verify-ca", "verify-full"]
    );
}

#[test]
fn sqlite_has_no_password_or_host() {
    let shown = visible(&Draft::new(Engine::Sqlite));
    assert_eq!(shown, [FieldKind::Path, FieldKind::Mode]);
    assert!(!Engine::Sqlite.needs_password());

    // A DSN for SQLite asks for no password either.
    let mut dsn = Draft::new(Engine::Sqlite);
    dsn.use_dsn = true;
    assert_eq!(visible(&dsn), [FieldKind::Dsn]);
}

#[test]
fn oracle_wallet_only_with_ssl() {
    let mut draft = Draft::new(Engine::Oracle);
    assert!(visible(&draft).contains(&FieldKind::Wallet), "prefer is on");
    draft.values.insert(FieldKind::Ssl, "disable".into());
    assert!(!visible(&draft).contains(&FieldKind::Wallet));
    draft.values.insert(FieldKind::Ssl, "verify-full".into());
    assert!(visible(&draft).contains(&FieldKind::Wallet));
}

#[test]
fn environment_swaps_the_password_for_a_variable() {
    let mut draft = Draft::new(Engine::Postgres);
    draft.store = PasswordStore::Environment;
    let shown = visible(&draft);
    assert!(shown.contains(&FieldKind::EnvVar));
    assert!(!shown.contains(&FieldKind::Password));
    assert_eq!(default_env_var("local-app"), "QRY_PW_LOCAL_APP");
    assert_eq!(default_env_var(" prod db 2 "), "QRY_PW_PROD_DB_2");
}

#[test]
fn validate_name_required_and_unique() {
    let mut d = draft(Engine::Postgres);
    assert!(validate(&d, &[]).is_empty(), "{:?}", validate(&d, &[]));

    d.name = "   ".into();
    assert_eq!(errors_for(&d, &[], FieldKind::Name), ["a name is required"]);

    d.name = "Prod".into();
    assert_eq!(
        errors_for(&d, &["staging", "prod"], FieldKind::Name),
        ["\"Prod\" is already a saved connection"]
    );
    assert!(errors_for(&d, &["staging"], FieldKind::Name).is_empty());
}

#[test]
fn validate_port_range() {
    let mut d = draft(Engine::Postgres);
    for bad in ["0", "65536", "abc", "-1"] {
        d.values.insert(FieldKind::Port, bad.into());
        assert_eq!(
            errors_for(&d, &[], FieldKind::Port),
            ["must be a number from 1 to 65535"],
            "{bad}"
        );
    }
    for good in ["1", "5432", "65535", ""] {
        d.values.insert(FieldKind::Port, good.into());
        assert!(errors_for(&d, &[], FieldKind::Port).is_empty(), "{good}");
    }
}

#[test]
fn validate_env_var_name() {
    let mut d = draft(Engine::Postgres);
    d.store = PasswordStore::Environment;
    assert_eq!(
        errors_for(&d, &[], FieldKind::EnvVar),
        ["a variable is required"]
    );
    for bad in ["pgpassword", "1PW", "PG-PW", "PG PW"] {
        d.env_var = bad.into();
        assert_eq!(
            errors_for(&d, &[], FieldKind::EnvVar),
            ["must look like [A-Z_][A-Z0-9_]*"],
            "{bad}"
        );
    }
    for good in ["PGPASSWORD", "_PW", "QRY_PW_LOCAL_APP2"] {
        d.env_var = good.into();
        assert!(errors_for(&d, &[], FieldKind::EnvVar).is_empty(), "{good}");
    }
    // Only checked when the password lives there.
    d.env_var = "nope".into();
    d.store = PasswordStore::Keychain;
    assert!(errors_for(&d, &[], FieldKind::EnvVar).is_empty());
}

#[test]
fn validate_sqlite_path_required() {
    let mut d = draft(Engine::Sqlite);
    assert!(validate(&d, &[]).is_empty());
    d.values.insert(FieldKind::Path, "  ".into());
    assert_eq!(
        validate(&d, &[]),
        [FieldError::new(FieldKind::Path, "a file is required")]
    );
}

#[test]
fn errors_follow_the_order_of_the_fields() {
    let mut d = Draft::new(Engine::Postgres);
    d.values.insert(FieldKind::Port, "0".into());
    let order: Vec<FieldKind> = validate(&d, &[]).into_iter().map(|e| e.field).collect();
    assert_eq!(
        order,
        [
            FieldKind::Name,
            FieldKind::Host,
            FieldKind::Port,
            FieldKind::User,
            FieldKind::Database
        ]
    );
}

#[test]
fn driver_switch_keeps_shared_fields() {
    let mut d = draft(Engine::Postgres);
    d.values.insert(FieldKind::Schema, "public".into());
    d.values.insert(FieldKind::Ssl, "allow".into());
    switch_engine(&mut d, Engine::Oracle);
    assert_eq!(d.engine, Engine::Oracle);
    assert_eq!(d.value(FieldKind::Host), "localhost");
    assert_eq!(d.value(FieldKind::User), "app");
    assert_eq!(d.value(FieldKind::Schema), "public");
    assert_eq!(d.value(FieldKind::Ssl), "allow", "Oracle offers allow too");
    assert_eq!(
        d.value(FieldKind::Database),
        "",
        "Oracle asks for a service"
    );
    assert_eq!(d.name, "local-app");

    // MySQL has no `allow`, so it goes back to the default.
    switch_engine(&mut d, Engine::Mysql);
    assert_eq!(d.value(FieldKind::Ssl), "prefer");
    assert_eq!(d.value(FieldKind::Schema), "", "MySQL has no schema field");
    assert_eq!(d.value(FieldKind::Host), "localhost");

    // SQLite keeps none of them, and gains its mode.
    switch_engine(&mut d, Engine::Sqlite);
    assert_eq!(d.value(FieldKind::Host), "");
    assert_eq!(d.value(FieldKind::Port), "");
    assert_eq!(d.value(FieldKind::Mode), "rw");
}

#[test]
fn driver_switch_updates_default_port_only_if_untouched() {
    let mut d = draft(Engine::Postgres);
    switch_engine(&mut d, Engine::Mysql);
    assert_eq!(d.value(FieldKind::Port), "3306", "it was still 5432");

    d.values.insert(FieldKind::Port, "13306".into());
    switch_engine(&mut d, Engine::Postgres);
    assert_eq!(d.value(FieldKind::Port), "13306", "a typed port stays");

    d.values.remove(&FieldKind::Port);
    switch_engine(&mut d, Engine::Oracle);
    assert_eq!(
        d.value(FieldKind::Port),
        "1521",
        "an empty one takes the default"
    );
}
