//! CHANGELOG.md keeps the Keep a Changelog shape the release notes are cut
//! from (the `awk` in `.github/workflows/cd.yml`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

fn changelog() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md"))
        .unwrap()
}

/// The `## [...]` headings, in order.
fn headings(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("## [")).collect()
}

/// What cd.yml's awk prints for `version`: the lines after its heading, up
/// to the next `## [`, without the trailing link definitions.
fn release_notes(text: &str, version: &str) -> String {
    let mut on = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with("## [") {
            if on {
                break;
            }
            on = line.contains(&format!("[{version}]"));
            continue;
        }
        if on && !(line.starts_with('[') && line.contains("]: http")) {
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn is_iso_date(d: &str) -> bool {
    let b = d.as_bytes();
    d.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && d.bytes()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

#[test]
fn changelog_has_unreleased_and_semver_headings() {
    let text = changelog();
    let headings = headings(&text);
    assert_eq!(headings.first(), Some(&"## [Unreleased]"));
    for heading in &headings[1..] {
        let (version, _) = heading.trim_start_matches("## [").split_once(']').unwrap();
        assert!(is_semver(version), "{heading}");
    }
    assert!(headings.iter().any(|h| h.starts_with("## [0.1.0]")));
}

#[test]
fn changelog_dates_are_iso() {
    let text = changelog();
    for heading in headings(&text).into_iter().skip(1) {
        let date = heading.rsplit(" - ").next().unwrap();
        assert!(is_iso_date(date), "{heading}");
    }
}

#[test]
fn release_notes_extraction_finds_section() {
    let notes = release_notes(&changelog(), "0.1.0");
    assert!(!notes.is_empty());
    assert!(notes.contains("### "), "{notes}");
    assert!(!notes.contains("## ["), "stops at the next version");
    assert!(release_notes(&changelog(), "9.9.9").is_empty());
}
