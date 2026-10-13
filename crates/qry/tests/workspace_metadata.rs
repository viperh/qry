//! Structural checks on the workspace, read from `cargo metadata`:
//!
//! - `qry-core` never reaches a UI crate (or the keychain) through its normal
//!   dependencies, and no workspace crate depends on the `qry` binary;
//! - every crate shares the lint baseline, the MSRV and the license;
//! - unit tests live in sibling files rather than inline `mod tests { … }`.
//!
//! The CI `layering` job runs this file; `scripts/check-layering.py` is an
//! independent second implementation of the dependency rules, and
//! `deny.toml` bans the same crates a third time.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use cargo_metadata::{CargoOpt, DependencyKind, Metadata, MetadataCommand, Package, PackageId};

/// Crates `qry-core` must never depend on, directly or transitively.
const FORBIDDEN_IN_CORE: &[&str] = &[
    "ratatui",
    "crossterm",
    "clap",
    "ratatui-textarea",
    "keyring",
];

fn manifest_path() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")
}

/// The workspace, without resolving dependencies (fast, and works offline).
fn metadata() -> Metadata {
    MetadataCommand::new()
        .manifest_path(manifest_path())
        .no_deps()
        .exec()
        .expect("cargo metadata runs")
}

/// The workspace with the dependency graph resolved for every feature.
fn resolved_metadata() -> Metadata {
    MetadataCommand::new()
        .manifest_path(manifest_path())
        .features(CargoOpt::AllFeatures)
        .other_options(vec!["--locked".to_owned()])
        .exec()
        .expect("cargo metadata --all-features runs")
}

fn members(metadata: &Metadata) -> Vec<&Package> {
    let members = metadata.workspace_packages();
    assert!(!members.is_empty(), "the workspace has no members");
    members
}

fn member<'a>(metadata: &'a Metadata, name: &str) -> &'a Package {
    members(metadata)
        .into_iter()
        .find(|package| package.name.as_str() == name)
        .unwrap_or_else(|| panic!("{name} is not a workspace member"))
}

/// Package id -> the ids of its normal (non-dev, non-build) dependencies, on
/// every target platform.
fn normal_graph(metadata: &Metadata) -> HashMap<&PackageId, Vec<&PackageId>> {
    let resolve = metadata.resolve.as_ref().expect("a resolved graph");
    resolve
        .nodes
        .iter()
        .map(|node| {
            let deps = node
                .deps
                .iter()
                .filter(|dep| {
                    dep.dep_kinds
                        .iter()
                        .any(|kind| kind.kind == DependencyKind::Normal)
                })
                .map(|dep| &dep.pkg)
                .collect();
            (&node.id, deps)
        })
        .collect()
}

/// Every package reachable from `root`, each with the shortest path to it
/// (`root -> … -> package`, as crate names).
fn reachable(metadata: &Metadata, root: &PackageId) -> Vec<(String, String)> {
    let graph = normal_graph(metadata);
    let name = |id: &PackageId| metadata[id].name.to_string();
    let mut paths: HashMap<&PackageId, String> = HashMap::from([(root, name(root))]);
    let mut queue = VecDeque::from([root]);
    let mut found = Vec::new();
    while let Some(current) = queue.pop_front() {
        let path = paths[current].clone();
        for &next in graph.get(current).into_iter().flatten() {
            if !paths.contains_key(next) {
                let next_path = format!("{path} -> {}", name(next));
                found.push((name(next), next_path.clone()));
                paths.insert(next, next_path);
                queue.push_back(next);
            }
        }
    }
    found
}

#[test]
fn qry_core_has_no_ui_dependencies() {
    let metadata = resolved_metadata();
    let core = member(&metadata, "qry-core");
    let mut violations: Vec<_> = reachable(&metadata, &core.id)
        .into_iter()
        .filter(|(name, _)| FORBIDDEN_IN_CORE.contains(&name.as_str()))
        .map(|(_, path)| path)
        .collect();
    // Two versions of one crate give the same path twice.
    violations.sort();
    violations.dedup();
    assert!(
        violations.is_empty(),
        "qry-core must not depend on {FORBIDDEN_IN_CORE:?}, but it does:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn no_workspace_crate_depends_on_the_binary() {
    let metadata = metadata();
    // `qry` dev-depends on itself (for `test-util`); that edge is allowed.
    let offenders: Vec<_> = members(&metadata)
        .into_iter()
        .filter(|package| package.name.as_str() != "qry")
        .flat_map(|package| {
            package
                .dependencies
                .iter()
                .filter(|dep| dep.name == "qry")
                .map(|dep| format!("{} ({:?})", package.name, dep.kind))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "these crates depend on the qry binary: {offenders:?}"
    );
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
