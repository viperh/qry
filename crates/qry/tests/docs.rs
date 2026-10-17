//! The hand-written docs hold together: every relative link and anchor
//! resolves, the README's images exist, and its CI badge names the real
//! workflow and branch. (The generated docs are checked in `src/docs.rs`.)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// README, CONTRIBUTING, CHANGELOG and every Markdown file under `docs/`.
fn markdown_files() -> Vec<PathBuf> {
    let root = root();
    let mut files = vec![
        root.join("README.md"),
        root.join("CONTRIBUTING.md"),
        root.join("CHANGELOG.md"),
    ];
    let mut docs: Vec<PathBuf> = std::fs::read_dir(root.join("docs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    docs.sort();
    files.extend(docs);
    files
}

/// The targets of `[text](target)` and `![alt](target)` links, outside
/// code blocks and inline code.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Inline code may hold `](` that is not a link.
        let line: String = line
            .split('`')
            .enumerate()
            .filter(|(i, _)| i % 2 == 0)
            .map(|(_, part)| part)
            .collect();
        let mut rest = line.as_str();
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            if let Some(end) = rest.find(')') {
                out.push(rest[..end].to_string());
                rest = &rest[end..];
            }
        }
    }
    out
}

/// GitHub's anchor for a heading: lower case, spaces to `-`, punctuation
/// other than `-` and `_` dropped.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

fn anchors(text: &str) -> Vec<String> {
    let mut fenced = false;
    text.lines()
        .filter(|line| {
            if line.trim_start().starts_with("```") {
                fenced = !fenced;
            }
            !fenced && line.starts_with('#')
        })
        .map(|line| slug(line.trim_start_matches('#')))
        .collect()
}

#[test]
fn relative_links_resolve() {
    let mut broken = Vec::new();
    for file in markdown_files() {
        let text = std::fs::read_to_string(&file).unwrap();
        let dir = file.parent().unwrap();
        for link in links(&text) {
            let link = link.split_whitespace().next().unwrap_or_default();
            if link.is_empty() || link.contains("://") || link.starts_with("mailto:") {
                continue;
            }
            let (path, anchor) = link.split_once('#').unwrap_or((link, ""));
            let target = if path.is_empty() {
                file.clone()
            } else {
                dir.join(path)
            };
            if !target.exists() {
                broken.push(format!("{}: {link}", file.display()));
                continue;
            }
            if !anchor.is_empty() && target.extension().is_some_and(|e| e == "md") {
                let headings = anchors(&std::fs::read_to_string(&target).unwrap());
                if !headings.iter().any(|h| h == anchor) {
                    broken.push(format!("{}: {link} (no such heading)", file.display()));
                }
            }
        }
    }
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

#[test]
fn readme_referenced_assets_exist() {
    let text = std::fs::read_to_string(root().join("README.md")).unwrap();
    let images: Vec<String> = links(&text)
        .into_iter()
        .filter(|l| l.starts_with("docs/assets/"))
        .collect();
    assert!(!images.is_empty(), "the README shows a screenshot");
    for image in images {
        assert!(root().join(&image).exists(), "{image} is missing");
    }
}

#[test]
fn readme_ci_badge_matches_workflow_and_branch() {
    let ci = std::fs::read_to_string(root().join(".github/workflows/ci.yml")).unwrap();
    assert!(
        ci.lines().any(|l| l.starts_with("name: CI")),
        "ci.yml is named CI"
    );
    // `on: push: branches:`, inline (`[master]`) or as a list.
    let mut lines = ci
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("push:"))
        .skip_while(|l| !l.trim_start().starts_with("branches:"));
    let inline = lines.next().unwrap().trim().trim_start_matches("branches:");
    let branch = match inline.trim().trim_matches(['[', ']']).trim() {
        "" => lines.next().unwrap().trim().trim_start_matches('-').trim(),
        b => b,
    }
    .to_string();
    let readme = std::fs::read_to_string(root().join("README.md")).unwrap();
    let badge = format!("actions/workflows/ci.yml/badge.svg?branch={branch}");
    assert!(
        readme.contains(&badge),
        "the README's badge should be {badge}"
    );
}

#[test]
fn slugs_follow_github() {
    assert_eq!(slug(" Key bindings"), "key-bindings");
    assert_eq!(slug("Results · Navigation"), "results--navigation");
    assert_eq!(slug("`max_rows` (rows)"), "max_rows-rows");
}
