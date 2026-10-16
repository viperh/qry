#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Every fuzz entry point has a cargo-fuzz target and a line in the nightly
//! matrix, and nothing is listed that has no entry point: `fuzz_x` in
//! `qry::fuzz` or `qry_core::fuzz` is the target `x` in `fuzz/Cargo.toml`
//! and in `.github/workflows/fuzz.yml`, whose body is that one call.

use std::{collections::BTreeSet, fs, path::PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo().join(relative))
        .unwrap_or_else(|e| panic!("cannot read {relative}: {e}"))
}

/// `x` for every `pub fn fuzz_x(` in the two crates' fuzz modules, with the
/// crate it lives in.
fn entry_points() -> BTreeSet<(String, &'static str)> {
    let mut found = BTreeSet::new();
    for (krate, file) in [
        ("qry", "crates/qry/src/fuzz.rs"),
        ("qry_core", "crates/qry-core/src/fuzz.rs"),
    ] {
        for line in read(file).lines() {
            if let Some(rest) = line.trim_start().strip_prefix("pub fn fuzz_") {
                let name = rest.split('(').next().unwrap();
                found.insert((name.to_string(), krate));
            }
        }
    }
    found
}

/// The `name` of every `[[bin]]` in `fuzz/Cargo.toml`.
fn bins() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut in_bin = false;
    for line in read("fuzz/Cargo.toml").lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
        } else if in_bin && let Some(value) = line.strip_prefix("name = ") {
            names.insert(value.trim_matches('"').to_string());
        }
    }
    names
}

/// The `matrix.target` list of `.github/workflows/fuzz.yml`.
fn matrix() -> BTreeSet<String> {
    let text = read(".github/workflows/fuzz.yml");
    let mut lines = text.lines().skip_while(|l| l.trim() != "target:").skip(1);
    let mut names = BTreeSet::new();
    for line in lines.by_ref() {
        match line.trim().strip_prefix("- ") {
            Some(name) => names.insert(name.trim().to_string()),
            None => break,
        };
    }
    names
}

#[test]
fn every_fuzz_entry_point_has_a_target_and_a_matrix_line() {
    let entries = entry_points();
    let names: BTreeSet<String> = entries.iter().map(|(n, _)| n.clone()).collect();
    assert!(names.len() >= 11, "found only {names:?}");
    assert_eq!(names, bins(), "entry points vs fuzz/Cargo.toml [[bin]]s");
    assert_eq!(names, matrix(), "entry points vs the fuzz.yml matrix");

    for (name, krate) in &entries {
        let target = read(&format!("fuzz/fuzz_targets/{name}.rs"));
        let call = format!("{krate}::fuzz::fuzz_{name}(data)");
        assert!(
            target.contains(&call),
            "fuzz_targets/{name}.rs does not call {call}"
        );
    }
}

#[test]
fn every_target_has_seeds() {
    let script = read("fuzz/seed-corpus.sh");
    for name in bins() {
        assert!(
            script.contains(&name),
            "seed-corpus.sh names no seeds for {name}"
        );
    }
}

#[test]
fn the_root_workspace_leaves_the_fuzz_crate_out() {
    let root = read("Cargo.toml");
    assert!(root.contains(r#"exclude = ["fuzz"]"#), "{root}");
    let fuzz = read("fuzz/Cargo.toml");
    assert!(fuzz.contains("[workspace]") && fuzz.contains("cargo-fuzz = true"));
}
