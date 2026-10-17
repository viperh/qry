//! Snapshots of the theme swatch in each colour mode.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::{
    testing::{buffer_to_string, render_with, styled},
    theme::ThemeEnv,
};

fn swatch(env: &ThemeEnv) -> String {
    let theme = env.adapt(Theme::DARK);
    styled(&render_with(80, 24, |frame| {
        draw(&theme, frame, frame.area())
    }))
}

#[test]
fn every_sample_fits_at_80x24() {
    let theme = Theme::DARK;
    let lines = samples(&theme);
    // 4 columns of 20 cells, the last one blank.
    assert!(lines.len() <= 4 * 24, "{} samples", lines.len());
    for line in &lines {
        assert!(line.width() <= 19, "too wide: {line:?}");
    }
    let text = buffer_to_string(&render_with(80, 24, |frame| {
        draw(&theme, frame, frame.area())
    }));
    for line in &lines {
        let sample: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(&sample), "{sample} is not drawn");
    }
}

#[test]
fn theme_swatch_truecolor() {
    insta::assert_snapshot!(
        "theme_swatch_truecolor_80x24",
        swatch(&ThemeEnv::truecolor())
    );
}

#[test]
fn theme_swatch_ansi256() {
    insta::assert_snapshot!("theme_swatch_ansi256_80x24", swatch(&ThemeEnv::default()));
}

#[test]
fn theme_swatch_nocolor() {
    insta::assert_snapshot!("theme_swatch_nocolor_80x24", swatch(&ThemeEnv::no_color()));
}
