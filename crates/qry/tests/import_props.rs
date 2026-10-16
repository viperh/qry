#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Properties of the connection file, the importers and key bindings that
//! hold for any input of the right shape, beyond "does not panic" (the fuzz
//! twins in `src/fuzz_tests.rs` cover that).

use proptest::prelude::*;
use qry::{
    connections::{self, Driver, Secret, StoredConnection},
    import::{self, Format},
    keymap,
};
use serde_json::{Map, Value, json};

const DRIVERS: [Driver; 5] = [
    Driver::Postgres,
    Driver::Mariadb,
    Driver::Mysql,
    Driver::Oracle,
    Driver::Sqlite,
];

fn arb_text() -> impl Strategy<Value = String> {
    "\\PC{0,12}"
}

fn arb_opt_text() -> impl Strategy<Value = Option<String>> {
    proptest::option::of(arb_text())
}

fn arb_secret() -> impl Strategy<Value = Secret> {
    prop_oneof![
        Just(Secret::Keyring),
        Just(Secret::Prompt),
        Just(Secret::None),
        "[A-Z_]{1,12}".prop_map(|var| Secret::Env { var }),
    ]
}

/// Fields a newer qry might have written: never one of the known names,
/// nor one named like a password (an import drops those).
fn arb_extra() -> impl Strategy<Value = Map<String, Value>> {
    proptest::collection::btree_map(
        "x_[a-z]{1,6}",
        prop_oneof![
            arb_text().prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            any::<bool>().prop_map(Value::from),
            Just(json!({"nested": [1, "two", null]})),
        ],
        0..3,
    )
    .prop_map(|m| m.into_iter().collect())
}

fn arb_stored_connection() -> impl Strategy<Value = StoredConnection> {
    (
        (
            "[0-9a-f]{8}",
            arb_text(),
            proptest::sample::select(DRIVERS.to_vec()),
        ),
        (
            arb_opt_text(),
            proptest::option::of(any::<u16>()),
            arb_opt_text(),
            arb_opt_text(),
        ),
        (
            arb_opt_text(),
            arb_opt_text(),
            arb_opt_text(),
            arb_opt_text(),
        ),
        (arb_opt_text(), arb_opt_text(), arb_opt_text()),
        arb_secret(),
        arb_extra(),
    )
        .prop_map(
            |(
                (id, name, driver),
                (host, port, user, database),
                (service, path, schema, tls),
                (wallet, mode, dsn),
                secret,
                extra,
            )| StoredConnection {
                id,
                name,
                driver,
                host,
                port,
                user,
                database,
                service,
                path,
                schema,
                tls,
                wallet,
                mode,
                dsn,
                secret,
                extra,
                ephemeral: false,
            },
        )
}

fn without_ids(connections: &[StoredConnection]) -> Vec<StoredConnection> {
    connections
        .iter()
        .cloned()
        .map(|mut c| {
            c.id = String::new();
            c
        })
        .collect()
}

/// A chord in either spelling, from the key names the parser knows, that
/// must parse.
fn arb_key_sequence() -> impl Strategy<Value = String> {
    let name = prop_oneof![
        proptest::sample::select(vec![
            "esc",
            "Escape",
            "enter",
            "Return",
            "left",
            "←",
            "right",
            "→",
            "up",
            "↑",
            "down",
            "↓",
            "home",
            "end",
            "pageup",
            "PgUp",
            "pagedown",
            "PgDn",
            "backtab",
            "tab",
            "Tab",
            "backspace",
            "delete",
            "Del",
            "insert",
            "Ins",
            "space",
            "hyphen",
            "minus",
            "lt",
            "gt",
            "f1",
            "F5",
            "f10",
            "F12",
        ])
        .prop_map(str::to_string),
        "[!-;=?-~é]".prop_map(|c: String| c),
    ];
    let mods = proptest::collection::btree_set(
        proptest::sample::select(vec!["ctrl-", "alt-", "shift-"]),
        0..=3,
    );
    let chord = (mods, name).prop_map(|(mods, name)| {
        let mods: String = mods.into_iter().collect();
        format!("{mods}{name}")
    });
    (proptest::collection::vec(chord, 1..4), any::<bool>()).prop_map(|(chords, wrapped)| {
        if wrapped {
            chords.iter().map(|c| format!("<{c}>")).collect::<String>()
        } else {
            chords.join(" ")
        }
    })
}

/// A DBeaver file whose connections, all openable, carry `password`.
fn arb_dbeaver_file(password: String) -> impl Strategy<Value = Value> {
    let entry = (
        proptest::sample::select(vec!["postgresql", "mysql", "mariadb", "oracle", "sqlite"]),
        "[a-z]{1,8}",
        "[a-z.]{1,12}",
        proptest::option::of(1u16..),
        "[a-z]{1,8}",
    );
    proptest::collection::vec(entry, 0..4).prop_map(move |entries| {
        let connections: Map<String, Value> = entries
            .into_iter()
            .enumerate()
            .map(|(i, (provider, name, host, port, db))| {
                let config = if provider == "sqlite" {
                    json!({"database": format!("/tmp/{db}.db"), "password": password})
                } else {
                    json!({"host": host, "port": port.map(|p| p.to_string()), "database": db,
                           "user": name, "password": password, "auth-model": "native"})
                };
                let entry = json!({"provider": provider, "driver": provider, "name": name,
                                   "save-password": true, "configuration": config});
                (format!("{provider}-{i}"), entry)
            })
            .collect();
        json!({"folders": {}, "connections": connections})
    })
}

/// A SQL Developer export whose connections, all openable, carry `password`.
fn arb_sqldeveloper_file(password: String) -> impl Strategy<Value = Value> {
    let entry = (
        "[a-z]{1,8}",
        "[a-z.]{1,12}",
        1u16..,
        "[A-Z]{1,6}",
        any::<bool>(),
    );
    proptest::collection::vec(entry, 0..4).prop_map(move |entries| {
        let connections: Vec<Value> = entries
            .into_iter()
            .map(|(name, host, port, service, sid)| {
                let target = if sid { "sid" } else { "serviceName" };
                json!({"name": name, "type": "jdbc", "info": {
                    "role": "", "SavePassword": "true", "OracleConnectionType": "BASIC",
                    "RaptorConnectionType": "Oracle", "hostname": host, "port": port.to_string(),
                    target: service, "user": name, "password": password,
                }})
            })
            .collect();
        json!({"connections": connections})
    })
}

/// A file of one of the three formats, and that format.
fn arb_file_of_some_format() -> impl Strategy<Value = (Value, Format)> {
    prop_oneof![
        proptest::collection::vec(arb_stored_connection(), 0..4)
            .prop_map(|c| (json!({"version": 1, "connections": c}), Format::Qry)),
        arb_dbeaver_file("pw".into()).prop_map(|v| (v, Format::DBeaver)),
        arb_sqldeveloper_file("pw".into()).prop_map(|v| (v, Format::SqlDeveloper)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// A backup qry writes imports back as the same connections.
    #[test]
    fn import_of_an_export_is_the_identity(
        records in proptest::collection::vec(arb_stored_connection(), 0..5),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let written = connections::export(&dir.path().join("backup"), &records).unwrap();
        let imported = import::from_file(written.to_str().unwrap(), Format::Qry).unwrap();
        prop_assert_eq!(imported.format, Format::Qry);
        prop_assert_eq!(without_ids(&imported.connections), without_ids(&records));
    }

    #[test]
    fn connections_save_then_load_round_trips(
        records in proptest::collection::vec(arb_stored_connection(), 0..5),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        connections::save_to(&data, &records).unwrap();
        prop_assert_eq!(connections::load_from(&data).unwrap(), records);
    }

    /// Any binding written with the real key names parses, and printing it
    /// and parsing that gives the same keys.
    #[test]
    fn keybinding_display_parse_round_trip(text in arb_key_sequence()) {
        let keys = keymap::parse_key_sequence(&text);
        prop_assert!(keys.is_ok(), "{:?} does not parse: {:?}", text, keys);
        let keys = keys.unwrap();
        let shown = keymap::chord_to_string(&keys);
        prop_assert_eq!(keymap::parse_key_sequence(&shown), Ok(keys), "{} printed as {}", text, shown);
    }

    #[test]
    fn auto_detection_agrees_with_explicit_format((file, format) in arb_file_of_some_format()) {
        let bytes = serde_json::to_vec(&file).unwrap();
        let explicit = import::from_bytes(&bytes, format).unwrap();
        let detected = import::from_bytes(&bytes, Format::Auto).unwrap();
        prop_assert_eq!(detected.format, format);
        prop_assert_eq!(without_ids(&detected.connections), without_ids(&explicit.connections));
        prop_assert_eq!(detected.skipped, explicit.skipped);
    }

    /// Passwords in a DBeaver or SQL Developer file stay behind: every
    /// imported connection asks for its own.
    #[test]
    fn imports_never_carry_a_plaintext_password(
        (file, password) in "[A-Za-z0-9]{6,12}".prop_flat_map(|p| {
            let password = format!("PW-{p}");
            (
                prop_oneof![
                    arb_dbeaver_file(password.clone()),
                    arb_sqldeveloper_file(password.clone()),
                ],
                Just(password),
            )
        }),
    ) {
        let imported = import::from_bytes(&serde_json::to_vec(&file).unwrap(), Format::Auto).unwrap();
        for record in &imported.connections {
            prop_assert!(
                matches!(record.secret, Secret::None | Secret::Keyring | Secret::Env { .. } | Secret::Prompt),
                "{:?}", record.secret
            );
            let written = serde_json::to_string(record).unwrap();
            prop_assert!(!written.contains(&password), "{}", written);
        }
    }
}
