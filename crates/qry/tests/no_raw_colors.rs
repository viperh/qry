//! Drawing code must use `Theme` style helpers, never raw colours (T201).
//!
//! Scans `src/**/*.rs` except `theme.rs` (where the palette lives) and its
//! own tests, `theme_tests.rs`. Ported from tachy's `tests/no_raw_colors.rs`,
//! which only looks for `Color::…`; this one also catches ratatui's
//! `Stylize` colour shorthands such as `.cyan()` and `.on_green()`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    fs,
    path::{Path, PathBuf},
};

const EXEMPT: &[&str] = &["theme.rs", "theme_tests.rs"];

/// ratatui's named colours.
const NAMED: &[&str] = &[
    "Reset",
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Magenta",
    "Cyan",
    "Gray",
    "DarkGray",
    "LightRed",
    "LightGreen",
    "LightYellow",
    "LightBlue",
    "LightMagenta",
    "LightCyan",
    "White",
];

/// ratatui's `Stylize` colour methods; each also has an `on_` form that
/// sets the background.
const SHORTHANDS: &[&str] = &[
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "gray",
    "dark_gray",
    "light_red",
    "light_green",
    "light_yellow",
    "light_blue",
    "light_magenta",
    "light_cyan",
    "white",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The identifier at the start of `s`.
fn ident(s: &str) -> &str {
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    &s[..end]
}

/// Every raw colour in `line`: a `Color::<named | Rgb | Indexed>`, or a
/// `.<colour>()` / `.on_<colour>()` shorthand.
fn raw_colors(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (i, _) in line.match_indices("Color::") {
        let name = ident(&line[i + "Color::".len()..]);
        if name == "Rgb" || name == "Indexed" || NAMED.contains(&name) {
            found.push(format!("Color::{name}"));
        }
    }
    for (i, _) in line.match_indices('.') {
        let rest = &line[i + 1..];
        let method = ident(rest);
        let colour = method.strip_prefix("on_").unwrap_or(method);
        if SHORTHANDS.contains(&colour) && rest[method.len()..].starts_with("()") {
            found.push(format!(".{method}()"));
        }
    }
    found
}

/// `(line number, offence)` for every raw colour in `text`, numbered from 1.
fn scan(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .flat_map(|(n, line)| raw_colors(line).into_iter().map(move |c| (n + 1, c)))
        .collect()
}

#[test]
fn no_raw_colors_outside_theme() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(files.len() > 5, "no sources found in {}", src.display());

    let mut offences = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_str().unwrap();
        if file.parent() == Some(src.as_path()) && EXEMPT.contains(&name) {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for (n, color) in scan(&text) {
            offences.push(format!("{}:{n}: {color}", file.display()));
        }
    }
    assert!(
        offences.is_empty(),
        "use Theme helpers instead of raw colours:\n{}",
        offences.join("\n")
    );
}

#[test]
fn scanner_catches_raw_color_fixture() {
    let text = fs::read_to_string(qry::testing::fixture("raw_color.rs.txt")).unwrap();
    let found = scan(&text);
    let found: Vec<(usize, &str)> = found.iter().map(|(n, c)| (*n, c.as_str())).collect();
    assert_eq!(
        found,
        [
            (6, "Color::Rgb"),
            (7, "Color::Indexed"),
            (8, "Color::Red"),
            (8, "Color::DarkGray"),
            (9, ".cyan()"),
            (10, ".black()"),
            (10, ".on_green()"),
        ]
    );
}

#[test]
fn detects_raw_colors() {
    assert_eq!(
        raw_colors("Style::new().fg(Color::Rgb(1, 2, 3))"),
        ["Color::Rgb"]
    );
    assert_eq!(raw_colors("bg(Color::Indexed(4))"), ["Color::Indexed"]);
    assert_eq!(
        raw_colors("fg(Color::Red).bg(Color::DarkGray)"),
        ["Color::Red", "Color::DarkGray"]
    );
    assert_eq!(raw_colors("line.dark_gray()"), [".dark_gray()"]);
    assert_eq!(raw_colors("cell.on_light_blue()"), [".on_light_blue()"]);
    assert!(raw_colors("theme.dialog_border(theme.teal)").is_empty());
    assert!(raw_colors("fn gauge(fill: Color) -> Style").is_empty());
    assert!(raw_colors("line.bold().reversed()").is_empty());
    // A field or method that only starts like a colour is fine.
    assert!(raw_colors("theme.green; x.redraw(); y.cyan_ish()").is_empty());
}
