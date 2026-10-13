//! Structural checks on the workspace itself: every crate shares the lint
//! baseline, the MSRV and the license, and unit tests live in sibling files
//! rather than inline `mod tests { … }` blocks. T003 extends these.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::{Path, PathBuf},
};

use cargo_metadata::{Metadata, MetadataCommand, Package};

/// The workspace, without resolving dependencies (fast, and works offline).
fn metadata() -> Metadata {
    MetadataCommand::new()
        .manifest_path(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .no_deps()
        .exec()
        .expect("cargo metadata runs")
}

fn members(metadata: &Metadata) -> Vec<&Package> {
    let members = metadata.workspace_packages();
    assert!(!members.is_empty(), "the workspace has no members");
    members
}

/// Whether a manifest opts into the workspace lints, either as a `[lints]`
/// table with `workspace = true` or as `lints.workspace = true`.
fn inherits_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_lints = line == "[lints]";
            continue;
        }
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if (in_lints && compact == "workspace=true") || compact == "lints.workspace=true" {
            return true;
        }
    }
    false
}

#[test]
fn every_crate_inherits_workspace_lints() {
    let metadata = metadata();
    let missing: Vec<_> = members(&metadata)
        .into_iter()
        .filter(|package| {
            let manifest = fs::read_to_string(&package.manifest_path).unwrap();
            !inherits_workspace_lints(&manifest)
        })
        .map(|package| package.name.to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "these crates lack `[lints] workspace = true`: {missing:?}"
    );
}

#[test]
fn every_crate_declares_the_workspace_rust_version() {
    let metadata = metadata();
    let versions: Vec<_> = members(&metadata)
        .into_iter()
        .map(|package| (package.name.to_string(), package.rust_version.clone()))
        .collect();
    for (name, version) in &versions {
        assert!(version.is_some(), "{name} has no rust-version");
    }
    let first = &versions[0].1;
    assert!(
        versions.iter().all(|(_, version)| version == first),
        "crates disagree on rust-version: {versions:?}"
    );
}

#[test]
fn every_crate_is_mit_licensed() {
    let metadata = metadata();
    for package in members(&metadata) {
        assert_eq!(
            package.license.as_deref(),
            Some("MIT"),
            "{} is not MIT licensed",
            package.name
        );
    }
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_inline_test_modules() {
    let metadata = metadata();
    let crates = metadata.workspace_root.as_std_path().join("crates");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(
        !files.is_empty(),
        "no sources found under {}",
        crates.display()
    );

    let offenders: Vec<_> = files
        .iter()
        .filter(|file| {
            fs::read_to_string(file)
                .unwrap()
                .lines()
                .any(|line| !line.trim_start().starts_with("//") && line.contains("mod tests {"))
        })
        .map(|file| {
            file.strip_prefix(&crates)
                .unwrap_or(file)
                .display()
                .to_string()
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "move these inline `mod tests {{ … }}` blocks to sibling *_tests.rs files: {offenders:?}"
    );
}
