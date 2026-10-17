#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The proptest twins of the fuzz targets: every `fuzz_*` body over random
//! bytes and over structured inputs, so the fuzz code runs on stable in
//! every `cargo test`, plus the replay of inputs the fuzzers once crashed.

use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::*;
use crate::testing::fixture;

/// Random bytes, up to 4 KiB.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..4096)
}

/// A JSON value of the kind a field might wrongly hold: of any type,
/// unicode, out of range, or nested.
fn arb_field_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(|f| json!(f)),
        (0u32..70_000).prop_map(|n| Value::from(n.to_string())),
        "\\PC{0,16}".prop_map(Value::from),
        proptest::sample::select(vec![
            "jdbc:postgresql://h:5432/db",
            "jdbc:mysql://[::1]:3306/x?a=b",
            "jdbc:mariadb://a,b/c",
            "jdbc:sqlite:file:/tmp/x.db",
            "jdbc:oracle:thin:u/p@//h:1521/svc",
            "jdbc:oracle:thin:@tcps://h:2484/svc",
            "jdbc:oracle:thin:@h:1521:SID",
            "jdbc:oracle:thin:@(DESCRIPTION=(x))",
            "jdbc:",
            "jdbc:oracle:thin:@h:",
            "jdbc:postgresql://[",
            "jdbc:postgresql://:0/",
            "TNS",
            "SID",
            "URL",
            "BASIC",
            "ADVANCED",
            "true",
            "",
        ])
        .prop_map(Value::from),
    ];
    leaf.prop_recursive(2, 16, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            proptest::collection::btree_map("\\PC{0,8}", inner, 0..4)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// An object holding some of `keys`, each with a random value.
fn arb_object(keys: &'static [&'static str]) -> impl Strategy<Value = Map<String, Value>> {
    proptest::collection::btree_map(
        proptest::sample::select(keys),
        arb_field_value(),
        0..=keys.len(),
    )
    .prop_map(|m| m.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

const DBEAVER_ENTRY: &[&str] = &["provider", "driver", "name", "read-only", "folder"];
const DBEAVER_CONFIG: &[&str] = &[
    "host",
    "port",
    "database",
    "url",
    "user",
    "password",
    "configurationType",
];
const DBEAVER_PROVIDERS: &[&str] = &[
    "postgresql",
    "mysql",
    "mariadb",
    "oracle",
    "sqlite",
    "generic",
    "mongodb",
    "",
];
const SQLDEV_INFO: &[&str] = &[
    "RaptorConnectionType",
    "OracleConnectionType",
    "hostname",
    "host",
    "port",
    "serviceName",
    "sid",
    "user",
    "password",
    "customUrl",
    "TNS_ALIAS",
    "OS_AUTHENTICATION",
    "KERBEROS_AUTHENTICATION",
    "ConnName",
];
const QRY_RECORD: &[&str] = &[
    "id", "name", "driver", "host", "port", "user", "database", "service", "path", "schema", "tls",
    "wallet", "mode", "dsn", "secret", "password",
];

/// DBeaver-shaped JSON with random field values.
fn arb_dbeaver_json() -> impl Strategy<Value = Value> {
    let entry = (
        arb_object(DBEAVER_ENTRY),
        proptest::sample::select(DBEAVER_PROVIDERS),
        arb_object(DBEAVER_CONFIG),
        arb_object(&["@dbeaver-sid-service@", "@dbeaver-connection-type@"]),
        any::<bool>(),
    )
        .prop_map(|(mut entry, provider, mut config, props, keep)| {
            if keep {
                entry.insert("provider".into(), provider.into());
            }
            config.insert("provider-properties".into(), Value::Object(props));
            entry.insert("configuration".into(), Value::Object(config));
            Value::Object(entry)
        });
    proptest::collection::btree_map("\\PC{0,8}", entry, 0..4)
        .prop_map(|m| json!({ "folders": {}, "connections": m }))
}

/// SQL Developer-shaped JSON with random field values.
fn arb_sqldeveloper_json() -> impl Strategy<Value = Value> {
    let entry = (arb_field_value(), arb_object(SQLDEV_INFO))
        .prop_map(|(name, info)| json!({ "name": name, "type": "jdbc", "info": info }));
    proptest::collection::vec(entry, 0..4).prop_map(|c| json!({ "connections": c }))
}

/// qry-backup-shaped JSON with random field values.
fn arb_qry_json() -> impl Strategy<Value = Value> {
    (
        proptest::collection::vec(arb_object(QRY_RECORD), 0..4),
        prop_oneof![Just(json!(1)), Just(json!(0)), arb_field_value()],
    )
        .prop_map(|(records, version)| json!({ "version": version, "connections": records }))
}

/// Valid-shaped DBeaver, SQL Developer and qry JSON: missing keys, wrong
/// types, huge ports, unicode names.
fn arb_json_connections() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![arb_dbeaver_json(), arb_sqldeveloper_json(), arb_qry_json()]
        .prop_map(|value| serde_json::to_vec(&value).unwrap())
}

/// A chord in the config's `<…>` spelling or the printed one, built from
/// the real key names, with junk mixed in.
fn arb_key_text() -> impl Strategy<Value = String> {
    let key = prop_oneof![
        proptest::sample::select(vec![
            "esc",
            "Enter",
            "return",
            "left",
            "←",
            "→",
            "↑",
            "↓",
            "home",
            "End",
            "PgUp",
            "pagedown",
            "backtab",
            "tab",
            "Backspace",
            "del",
            "ins",
            "space",
            "minus",
            "lt",
            "gt",
            "f1",
            "F12",
            "f0",
            "f13",
            "f",
            "-",
            "<",
            ">",
            "<>",
            "",
        ])
        .prop_map(str::to_string),
        any::<char>().prop_map(|c| c.to_string()),
    ];
    let mods = proptest::collection::vec(
        proptest::sample::select(vec!["ctrl-", "alt-", "shift-", "CTRL-", "Shift-"]),
        0..4,
    );
    let chord = (mods, key, any::<bool>()).prop_map(|(mods, key, wrap)| {
        let chord = format!("{}{key}", mods.concat());
        if wrap { format!("<{chord}>") } else { chord }
    });
    (proptest::collection::vec(chord, 1..4), any::<bool>())
        .prop_map(|(chords, spaced)| chords.join(if spaced { " " } else { "" }))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn import_qry_never_panics(data in bytes(), shaped in arb_json_connections()) {
        fuzz_import_qry(&data);
        fuzz_import_qry(&shaped);
    }

    #[test]
    fn import_dbeaver_never_panics(data in bytes(), shaped in arb_json_connections()) {
        fuzz_import_dbeaver(&data);
        fuzz_import_dbeaver(&shaped);
    }

    #[test]
    fn import_sqldeveloper_never_panics(data in bytes(), shaped in arb_json_connections()) {
        fuzz_import_sqldeveloper(&data);
        fuzz_import_sqldeveloper(&shaped);
    }

    #[test]
    fn import_auto_never_panics(data in bytes(), shaped in arb_json_connections()) {
        fuzz_import_auto(&data);
        fuzz_import_auto(&shaped);
    }

    #[test]
    fn connections_parse_never_panics(data in bytes(), shaped in arb_qry_json()) {
        fuzz_connections_parse(&data);
        fuzz_connections_parse(&serde_json::to_vec(&shaped).unwrap());
    }

    #[test]
    fn keymap_parse_never_panics_and_round_trips(data in bytes(), text in arb_key_text()) {
        fuzz_keymap_parse(&data);
        fuzz_keymap_parse(text.as_bytes());
    }

    #[test]
    fn palette_score_never_panics(data in bytes(), query in "\\PC{0,8}", text in "\\PC{0,40}") {
        fuzz_palette_score(&data);
        fuzz_palette_score(format!("{query}\0{text}").as_bytes());
    }
}

#[test]
fn a_password_in_a_hand_edited_backup_is_dropped() {
    let backup = json!({
        "version": 1,
        "connections": [{
            "id": "a", "name": "a", "driver": "postgres", "secret": {"mode": "prompt"},
            "password": "hunter2", "DbPwd": "hunter2",
            "options": {"nested": [{"passwd": "hunter2", "keep": 1}]}
        }]
    });
    let imported = import::from_bytes(backup.to_string().as_bytes(), Format::Auto).unwrap();
    let written = serde_json::to_string(&imported.connections).unwrap();
    assert!(!written.contains("hunter2"), "{written}");
    assert!(written.contains("keep"), "other fields are kept: {written}");
    fuzz_import_qry(backup.to_string().as_bytes());
}

#[test]
fn the_password_check_sees_nested_fields() {
    assert!(names_a_password(&json!({"a": [{"b": {"Password": 1}}]})));
    assert!(!names_a_password(
        &json!({"a": [{"b": {"user": "password"}}]})
    ));
}

/// The targets whose bodies live in `qry_core::fuzz`, replayed there.
const CORE_TARGETS: [&str; 4] = ["sql_lex", "sql_split", "grid", "copy_render"];

/// Which `fuzz_*` body replays the inputs under each target's directory.
fn entry_point(target: &str) -> Option<fn(&[u8])> {
    Some(match target {
        "import_qry" => fuzz_import_qry,
        "import_dbeaver" => fuzz_import_dbeaver,
        "import_sqldeveloper" => fuzz_import_sqldeveloper,
        "import_auto" => fuzz_import_auto,
        "connections_parse" => fuzz_connections_parse,
        "keymap_parse" => fuzz_keymap_parse,
        "palette_score" => fuzz_palette_score,
        _ => return None,
    })
}

/// Every input a fuzzer once crashed on, under
/// `tests/fixtures/fuzz-regressions/<target>/`, runs through its target's
/// body again. Targets of `qry-core` are replayed by its own copy of this
/// test.
#[test]
fn known_crash_inputs_stay_fixed() {
    let root = fixture("fuzz-regressions");
    let mut replayed = 0;
    for dir in std::fs::read_dir(&root).unwrap() {
        let dir = dir.unwrap().path();
        let target = dir.file_name().unwrap().to_str().unwrap().to_string();
        if !dir.is_dir() || CORE_TARGETS.contains(&target.as_str()) {
            continue;
        }
        let run = entry_point(&target)
            .unwrap_or_else(|| panic!("{} names no fuzz target of qry", dir.display()));
        for file in std::fs::read_dir(&dir).unwrap() {
            let file = file.unwrap().path();
            if file.file_name().is_some_and(|n| n == ".gitkeep") {
                continue;
            }
            run(&std::fs::read(&file).unwrap());
            replayed += 1;
        }
    }
    assert!(
        replayed > 0,
        "no regression inputs under {}",
        root.display()
    );
}
