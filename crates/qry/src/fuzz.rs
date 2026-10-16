#![doc(hidden)]
//! Entry points for the cargo-fuzz targets in `fuzz/` and their proptest
//! twins in `fuzz_tests.rs`, so both run exactly the same code and the fuzz
//! bodies cannot rot on stable.
//!
//! Each must never panic, whatever the input; returning an error is fine.
//! Beyond that, each asserts the invariant that matters for its parser:
//! imports never carry a password, and key bindings survive being printed
//! and read back. A new parser of untrusted input gets a `fuzz_*` function
//! here, a target in `fuzz/`, seeds in `fuzz/seed-corpus.sh`, a twin in
//! `fuzz_tests.rs` and a line in `.github/workflows/fuzz.yml`; the
//! `fuzz_inventory` test checks the names line up.

use std::path::Path;

use serde_json::Value;

use crate::{
    connections::{self, Secret},
    import::{self, Format, Imported},
    keymap,
    palette::fuzzy,
};

/// A qry backup.
pub fn fuzz_import_qry(data: &[u8]) {
    check_import(import::from_bytes(data, Format::Qry));
}

/// A DBeaver `data-sources.json`.
pub fn fuzz_import_dbeaver(data: &[u8]) {
    check_import(import::from_bytes(data, Format::DBeaver));
}

/// A SQL Developer `connections.json`.
pub fn fuzz_import_sqldeveloper(data: &[u8]) {
    check_import(import::from_bytes(data, Format::SqlDeveloper));
}

/// Any of the three, by detection.
pub fn fuzz_import_auto(data: &[u8]) {
    check_import(import::from_bytes(data, Format::Auto));
}

/// The stored connection file.
pub fn fuzz_connections_parse(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    let _ = connections::parse(&text, Path::new("fuzz.json"));
}

/// A key binding from the config: if it parses, printing it and parsing
/// that again gives the same keys.
pub fn fuzz_keymap_parse(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    if let Ok(keys) = keymap::parse_key_sequence(&text) {
        let shown = keymap::chord_to_string(&keys);
        let again = keymap::parse_key_sequence(&shown);
        assert_eq!(
            again.as_deref(),
            Ok(&keys[..]),
            "{text:?} printed as {shown:?}"
        );
    }
}

/// The palette's fuzzy match. The bytes before the first NUL are the
/// query, the rest the text. The positions are increasing, in bounds, and
/// one per query character.
pub fn fuzz_palette_score(data: &[u8]) {
    let (query, text) = match data.iter().position(|&b| b == 0) {
        Some(at) => (&data[..at], &data[at + 1..]),
        None => (data, &[][..]),
    };
    let (query, text) = (
        String::from_utf8_lossy(query),
        String::from_utf8_lossy(text),
    );
    if let Some(found) = fuzzy::score(&query, &text) {
        let len = text.chars().count();
        assert!(found.positions.windows(2).all(|w| w[0] < w[1]));
        assert!(found.positions.iter().all(|&p| p < len));
        assert_eq!(found.positions.len(), query.chars().count());
    }
}

/// What every import must hold to: no connection carries a password.
/// [`Secret`] has no variant that holds one, so this checks that the record
/// written out has no field named like one either.
fn check_import(result: color_eyre::Result<Imported>) {
    let Ok(imported) = result else {
        return;
    };
    for record in &imported.connections {
        assert!(
            matches!(
                record.secret,
                Secret::None | Secret::Keyring | Secret::Env { .. } | Secret::Prompt
            ),
            "{:?}",
            record.secret
        );
        let written = serde_json::to_value(record).unwrap_or(Value::Null);
        assert!(
            !names_a_password(&written),
            "an imported connection kept a password field: {written}"
        );
    }
}

/// Whether any key, at any depth, is named like a password field.
pub(crate) fn names_a_password(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, value)| import::is_password_key(key) || names_a_password(value)),
        Value::Array(items) => items.iter().any(names_a_password),
        _ => false,
    }
}

#[cfg(test)]
#[path = "fuzz_tests.rs"]
mod fuzz_tests;
