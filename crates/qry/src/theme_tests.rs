//! Unit tests for `theme.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pretty_assertions::assert_eq;

use super::*;

fn hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02X}{g:02X}{b:02X}"),
        other => panic!("not an RGB colour: {other:?}"),
    }
}

/// Every colour field of `t`, by name.
fn fields(t: &Theme) -> [(&'static str, Color); 22] {
    [
        ("bg", t.bg),
        ("bg_alt_row", t.bg_alt_row),
        ("surface", t.surface),
        ("surface_raised", t.surface_raised),
        ("status_bg", t.status_bg),
        ("selection_bg", t.selection_bg),
        ("dialog_title_bg", t.dialog_title_bg),
        ("track", t.track),
        ("border", t.border),
        ("border_inner", t.border_inner),
        ("border_strong", t.border_strong),
        ("fg", t.fg),
        ("fg_muted", t.fg_muted),
        ("fg_dim", t.fg_dim),
        ("amber", t.amber),
        ("amber_bright", t.amber_bright),
        ("amber_tint", t.amber_tint),
        ("teal", t.teal),
        ("purple", t.purple),
        ("purple_tint", t.purple_tint),
        ("green", t.green),
        ("coral", t.coral),
    ]
}

/// Every field of `Theme::DARK` against the T201 table.
#[test]
fn dark_palette_exact_values() {
    let expected = [
        "#0E1116", "#10141A", "#131820", "#161C25", "#1A212B", "#1B2533", "#1E2632", "#1E2530",
        "#232A34", "#2C3542", "#3A4452", "#D3D9E0", "#A9B4C2", "#8A94A3", "#F2B04A", "#FFD08A",
        "#3A2F17", "#4FC1C9", "#C7A6FF", "#2A2340", "#9FD38A", "#F28B6B",
    ];
    for ((name, color), hex_value) in fields(&Theme::DARK).into_iter().zip(expected) {
        assert_eq!(hex(color), hex_value, "{name}");
    }
}

#[test]
fn color_support_precedence() {
    use ColorSupport::*;
    let cases = [
        (Some("1"), Some("truecolor"), NoColor),
        // An empty NO_COLOR doesn't count (no-color.org).
        (Some(""), Some("truecolor"), TrueColor),
        (None, Some("24bit"), TrueColor),
        (None, Some("TrueColor"), TrueColor),
        (None, Some("24BIT"), TrueColor),
        (None, None, Ansi256),
        (None, Some("xterm"), Ansi256),
        (None, Some(""), Ansi256),
    ];
    for (no_color, colorterm, expected) in cases {
        assert_eq!(
            ColorSupport::from_env(no_color, colorterm),
            expected,
            "NO_COLOR={no_color:?} COLORTERM={colorterm:?}"
        );
    }
}

#[test]
fn theme_env_maps_to_color_support() {
    assert_eq!(ThemeEnv::default().support(), ColorSupport::Ansi256);
    assert_eq!(ThemeEnv::truecolor().support(), ColorSupport::TrueColor);
    assert_eq!(ThemeEnv::no_color().support(), ColorSupport::NoColor);
    let both = ThemeEnv {
        no_color: true,
        ..ThemeEnv::truecolor()
    };
    assert_eq!(both.support(), ColorSupport::NoColor);
}

#[test]
fn by_name() {
    assert_eq!(Theme::by_name("dark"), Some(Theme::DARK));
    assert_eq!(Theme::by_name("light"), None);
    for name in Theme::NAMES {
        assert!(Theme::by_name(name).is_some(), "{name}");
    }
}

#[test]
fn truecolor_is_unchanged() {
    assert_eq!(Theme::DARK.adapt(ColorSupport::TrueColor), Theme::DARK);
}

// ---- xterm-256 mapping -------------------------------------------------

#[test]
fn ansi256_indices_match_tachy() {
    let t = Theme::DARK.adapt(ColorSupport::Ansi256);
    let cases = [
        ("bg", t.bg, 233),
        ("surface", t.surface, 234),
        ("status_bg", t.status_bg, 235),
        ("border", t.border, 236),
        ("fg", t.fg, 253),
        ("fg_dim", t.fg_dim, 246),
        ("amber", t.amber, 215),
        ("teal", t.teal, 74),
        ("purple", t.purple, 183),
        ("green", t.green, 150),
        ("coral", t.coral, 209),
    ];
    for (name, color, index) in cases {
        assert_eq!(color, Color::Indexed(index), "{name}");
    }
}

#[test]
fn xterm256_palette_values() {
    assert_eq!(xterm256_rgb(16), (0, 0, 0));
    assert_eq!(xterm256_rgb(215), (255, 175, 95));
    assert_eq!(xterm256_rgb(231), (255, 255, 255));
    assert_eq!(xterm256_rgb(232), (8, 8, 8));
    assert_eq!(xterm256_rgb(233), (18, 18, 18));
    assert_eq!(xterm256_rgb(255), (238, 238, 238));
}

#[test]
fn nearest_xterm256_extremes_and_greys() {
    assert_eq!(nearest_xterm256(0, 0, 0), 16); // cube 0,0,0 (exact)
    assert_eq!(nearest_xterm256(255, 255, 255), 231); // cube 5,5,5 (exact)
    assert_eq!(nearest_xterm256(0x80, 0x80, 0x80), 244); // ramp #808080 (exact)
    assert_eq!(nearest_xterm256(0x12, 0x12, 0x12), 233); // ramp #121212 (exact)
    assert_eq!(nearest_xterm256(0xEE, 0xEE, 0xEE), 255); // ramp #eeeeee (exact)
    assert_eq!(nearest_xterm256(0x05, 0x05, 0x05), 232); // ramp #080808: 81 vs 225 to black
}

/// Every palette colour, with the index tachy worked out by hand with the
/// redmean distance `(2 + r̄/256)·ΔR² + 4·ΔG² + (2 + (255 − r̄)/256)·ΔB²`.
/// The comment gives the chosen palette entry and the runner-up.
#[test]
fn nearest_xterm256_for_every_theme_colour() {
    let expected = [
        233, // bg: #121212 (vs 232 #080808)
        233, // bg_alt_row: #121212 (vs 234)
        234, // surface: #1c1c1c (vs 233)
        234, // surface_raised: #1c1c1c (vs 235)
        235, // status_bg: #262626 (vs 234)
        235, // selection_bg: #262626 (vs 236)
        235, // dialog_title_bg: #262626 (vs 236)
        235, // track: #262626 (vs 236)
        236, // border: #303030 (vs 235)
        237, // border_inner: #3a3a3a (vs 236)
        238, // border_strong: #444444 (vs 239)
        253, // fg: #dadada (vs 188 #d7d7d7)
        249, // fg_muted: #b2b2b2 (vs 145 #afafaf)
        246, // fg_dim: #949494 (vs 103 #8787af)
        215, // amber: cube 5,3,1 #ffaf5f (vs 179)
        222, // amber_bright: cube 5,4,2 #ffd787 (vs 223)
        235, // amber_tint: #262626 (vs 236)
        74,  // teal: cube 1,3,4 #5fafd7 (vs 80)
        183, // purple: cube 4,3,5 #d7afff (vs 147)
        236, // purple_tint: #303030 (vs 235)
        150, // green: cube 3,4,2 #afd787 (vs 114)
        209, // coral: cube 5,2,1 #ff875f (vs 210)
    ];
    for ((name, color), index) in fields(&Theme::DARK).into_iter().zip(expected) {
        let Color::Rgb(r, g, b) = color else {
            panic!("{name} is not RGB");
        };
        assert_eq!(nearest_xterm256(r, g, b), index, "{name}");
    }
}

#[test]
fn ansi256_only_indexed_16_to_255() {
    let t = Theme::DARK.adapt(ColorSupport::Ansi256);
    assert!(!t.monochrome);
    for (name, color) in fields(&t) {
        assert!(
            matches!(color, Color::Indexed(16..=255)),
            "{name}: {color:?}"
        );
    }
    // Striped rows collapse onto one index: accepted (see `Theme::adapt`).
    assert_eq!(t.bg_alt_row, t.bg);
}

#[test]
fn ansi256_keeps_selection_distinct_from_bg() {
    let t = Theme::DARK.adapt(ColorSupport::Ansi256);
    assert_ne!(t.selection_bg, t.bg);
    assert_ne!(t.status_bg, t.bg);
    // The cursor stays amber, distinct from the row backgrounds.
    assert_eq!(t.cursor_cell().bg, Some(Color::Indexed(215)));
    assert_ne!(t.cursor_cell().bg, t.row_selected().bg);
    assert_ne!(t.cursor_cell().bg, t.row(0).bg);
}

#[test]
fn collapsed_pairs_are_nudged_to_the_next_step() {
    // #121212 and #141414 both map to 233.
    let t = Theme {
        bg: Color::Rgb(0x12, 0x12, 0x12),
        selection_bg: Color::Rgb(0x14, 0x14, 0x14),
        status_bg: Color::Rgb(0x13, 0x13, 0x13),
        ..Theme::DARK
    };
    assert_eq!(nearest_xterm256(0x14, 0x14, 0x14), 233);
    assert_eq!(nearest_xterm256(0x13, 0x13, 0x13), 233);
    let a = t.adapt(ColorSupport::Ansi256);
    assert_eq!(a.bg, Color::Indexed(233));
    assert_eq!(a.selection_bg, Color::Indexed(234));
    assert_eq!(a.status_bg, Color::Indexed(234));

    // A darker partner moves down instead.
    let t = Theme {
        bg: Color::Rgb(0x14, 0x14, 0x14),
        selection_bg: Color::Rgb(0x12, 0x12, 0x12),
        ..Theme::DARK
    };
    let a = t.adapt(ColorSupport::Ansi256);
    assert_eq!(a.selection_bg, Color::Indexed(232));

    // At the end of the palette it steps back.
    let t = Theme {
        bg: Color::Rgb(0xEE, 0xEE, 0xEE),
        status_bg: Color::Rgb(0xEF, 0xEF, 0xEF),
        ..Theme::DARK
    };
    let a = t.adapt(ColorSupport::Ansi256);
    assert_eq!(a.bg, Color::Indexed(255));
    assert_eq!(a.status_bg, Color::Indexed(254));
}

// ---- NO_COLOR ----------------------------------------------------------

/// A style helper, called on a theme.
type Helper = fn(&Theme) -> Style;

/// Every style helper, by name. Helpers that take an argument are listed
/// once per value that matters.
fn helpers() -> Vec<(&'static str, Helper)> {
    vec![
        ("base", Theme::base),
        ("surface", Theme::surface),
        ("muted", Theme::muted),
        ("dim", Theme::dim),
        ("border", Theme::border),
        ("border_strong", Theme::border_strong),
        ("focused_border", Theme::focused_border),
        ("spinner", Theme::spinner),
        ("value_ok", Theme::value_ok),
        ("value_error", Theme::value_error),
        ("app_name", Theme::app_name),
        ("tab", Theme::tab),
        ("tab_active", Theme::tab_active),
        ("tab_running", Theme::tab_running),
        ("mode_pill(Normal)", |t| t.mode_pill(Pill::Normal)),
        ("mode_pill(Edit)", |t| t.mode_pill(Pill::Edit)),
        ("mode_pill(Command)", |t| t.mode_pill(Pill::Command)),
        ("pane_title", Theme::pane_title),
        ("pane_title_focused", Theme::pane_title_focused),
        ("divider", Theme::divider),
        ("divider_focused", Theme::divider_focused),
        ("tree_connection", Theme::tree_connection),
        ("tree_connection_selected", Theme::tree_connection_selected),
        ("tree_object", Theme::tree_object),
        ("tree_kind", Theme::tree_kind),
        ("session_dot(Open)", |t| {
            t.session_dot(&SessionStatus::Open { label: "x".into() }).0
        }),
        ("session_dot(Connecting)", |t| {
            t.session_dot(&SessionStatus::Connecting).0
        }),
        ("session_dot(Failed)", |t| {
            t.session_dot(&SessionStatus::Failed("x".into())).0
        }),
        ("driver_badge", |t| t.driver_badge(Driver::Postgres).0),
        ("tree_badge", |t| t.tree_badge(Driver::Postgres).0),
        ("tree_state(Open)", |t| {
            t.tree_state(Some(&SessionStatus::Open { label: "x".into() }), "⠋")
                .0
        }),
        ("tree_state(Connecting)", |t| {
            t.tree_state(Some(&SessionStatus::Connecting), "⠋").0
        }),
        ("tree_state(Failed)", |t| {
            t.tree_state(Some(&SessionStatus::Failed("x".into())), "⠋")
                .0
        }),
        ("tree_state(None)", |t| t.tree_state(None, "⠋").0),
        ("tree_folder", Theme::tree_folder),
        ("tree_count", Theme::tree_count),
        ("tree_column_type", Theme::tree_column_type),
        ("tree_pk", Theme::tree_pk),
        ("tree_fk", Theme::tree_fk),
        ("tree_null", Theme::tree_null),
        ("tree_error", Theme::tree_error),
        ("tree_gutter", Theme::tree_gutter),
        ("tree_filter_prompt", Theme::tree_filter_prompt),
        ("header", Theme::header),
        ("header_type", Theme::header_type),
        ("header_active", Theme::header_active),
        ("sort_indicator", Theme::sort_indicator),
        ("overflow_hint", Theme::overflow_hint),
        ("gutter", Theme::gutter),
        ("gutter_selected", Theme::gutter_selected),
        ("column_divider", Theme::column_divider),
        ("frozen_divider", Theme::frozen_divider),
        ("row(0)", |t| t.row(0)),
        ("row(1)", |t| t.row(1)),
        ("row_selected", Theme::row_selected),
        ("cursor_cell", Theme::cursor_cell),
        ("null_value", Theme::null_value),
        ("match_highlight", Theme::match_highlight),
        ("inspector_label", Theme::inspector_label),
        ("inspector_label_focused", Theme::inspector_label_focused),
        ("record_field", Theme::record_field),
        ("record_value", Theme::record_value),
        ("value_changed", Theme::value_changed),
        ("status_line", Theme::status_line),
        ("status_session", Theme::status_session),
        ("status_jobs", Theme::status_jobs),
        ("status_warning", Theme::status_warning),
        ("status_ok", Theme::status_ok),
        ("status_dim", Theme::status_dim),
        ("hint", Theme::hint),
        ("hint_key", Theme::hint_key),
        ("toast(Info)", |t| t.toast(ToastLevel::Info)),
        ("toast(Success)", |t| t.toast(ToastLevel::Success)),
        ("toast(Warning)", |t| t.toast(ToastLevel::Warning)),
        ("toast(Error)", |t| t.toast(ToastLevel::Error)),
        ("gauge", |t| t.gauge(t.teal)),
        ("sql_keyword", Theme::sql_keyword),
        ("sql_operator", Theme::sql_operator),
        ("sql_string", Theme::sql_string),
        ("sql_number", Theme::sql_number),
        ("sql_comment", Theme::sql_comment),
        ("sql_identifier", Theme::sql_identifier),
        ("sql_punct", Theme::sql_punct),
        ("sql_quoted_ident", Theme::sql_quoted_ident),
        ("sql_error", Theme::sql_error),
        ("editor_cursor_line", Theme::editor_cursor_line),
        ("editor_cursor", Theme::editor_cursor),
        ("editor_selection", Theme::editor_selection),
        ("line_number", Theme::line_number),
        ("line_number_current", Theme::line_number_current),
        ("dialog", Theme::dialog),
        ("dialog_border", |t| t.dialog_border(t.teal)),
        ("dialog_title", |t| t.dialog_title(t.teal)),
        ("dialog_heading", |t| t.dialog_heading(t.teal)),
        ("field", Theme::field),
        ("field_focused", Theme::field_focused),
        ("field_placeholder", Theme::field_placeholder),
        ("field_border", Theme::field_border),
        ("field_border_focused", Theme::field_border_focused),
        ("button", Theme::button),
        ("button_primary", Theme::button_primary),
        ("palette_border", Theme::palette_border),
        ("palette_item_selected", Theme::palette_item_selected),
        ("palette_match", Theme::palette_match),
        ("backdrop", Theme::backdrop),
        ("job(Query)", |t| t.job(JobKind::Query)),
        ("job(Export)", |t| t.job(JobKind::Export)),
        ("job_state(Running)", |t| {
            t.job_state(JobKind::Query, &JobState::Running { rows: 0 })
                .0
        }),
        ("job_state(Done)", |t| {
            t.job_state(JobKind::Query, &JobState::Done { rows: 0 }).0
        }),
    ]
}

/// No colour at all: `None`, or `Reset` from a field.
fn colourless(c: Option<Color>) -> bool {
    matches!(c, None | Some(Color::Reset))
}

#[test]
fn nocolor_is_reset_and_monochrome() {
    let t = Theme::DARK.adapt(ColorSupport::NoColor);
    assert!(t.monochrome);
    for (name, color) in fields(&t) {
        assert_eq!(color, Color::Reset, "{name}");
    }
    let cursor = t.cursor_cell();
    assert_eq!(cursor.fg, None);
    assert_eq!(cursor.bg, None);
    assert_eq!(cursor.add_modifier, Modifier::REVERSED | Modifier::BOLD);
}

#[test]
fn every_helper_has_mono_fallback() {
    let t = Theme::DARK.adapt(ColorSupport::NoColor);
    for (name, helper) in helpers() {
        let style = helper(&t);
        assert!(colourless(style.fg), "{name}: fg {:?}", style.fg);
        assert!(colourless(style.bg), "{name}: bg {:?}", style.bg);
        assert!(
            colourless(style.underline_color),
            "{name}: underline {:?}",
            style.underline_color
        );
        assert_eq!(style.sub_modifier, Modifier::empty(), "{name}");
    }
}

/// With colour, every helper sets some colour or modifier: none is a no-op.
#[test]
fn every_helper_styles_something_in_colour() {
    let t = Theme::DARK;
    for (name, helper) in helpers() {
        assert_ne!(helper(&t), Style::new(), "{name}");
    }
}

/// The things the user must still tell apart without colour.
#[test]
fn nocolor_keeps_cursor_selection_and_matches_distinct() {
    let t = Theme::DARK.adapt(ColorSupport::NoColor);
    let styles = [
        t.row(0),
        t.cursor_cell(),
        t.row_selected(),
        t.match_highlight(),
    ];
    for (i, a) in styles.iter().enumerate() {
        for b in &styles[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_ne!(t.mode_pill(Pill::Normal), t.status_line());
    assert_ne!(t.pane_title(), t.pane_title_focused());
    assert_ne!(t.field(), t.field_focused());
}

// ---- accents -----------------------------------------------------------

#[test]
fn mode_pill_accents() {
    let t = Theme::DARK;
    for (pill, bg) in [
        (Pill::Normal, t.teal),
        (Pill::Edit, t.amber),
        (Pill::Command, t.purple),
    ] {
        let style = t.mode_pill(pill);
        assert_eq!(style.bg, Some(bg), "{pill:?}");
        assert_eq!(style.fg, Some(t.bg), "{pill:?}");
        assert!(style.add_modifier.contains(Modifier::BOLD), "{pill:?}");
    }
}

#[test]
fn job_state_glyphs() {
    let t = Theme::DARK;
    let cases = [
        (JobState::Done { rows: 3 }, t.green, "✓"),
        (JobState::Failed("disk full".into()), t.coral, "✗"),
        (JobState::Cancelled, t.fg_dim, "–"),
        (JobState::Queued, t.fg_dim, "…"),
        (JobState::Running { rows: 10 }, t.amber, "▸"),
    ];
    for (state, fg, glyph) in cases {
        let (style, symbol) = t.job_state(JobKind::Export, &state);
        assert_eq!(style.fg, Some(fg), "{state:?}");
        assert_eq!(symbol, glyph, "{state:?}");
    }
    // Running takes the kind's colour.
    assert_eq!(
        t.job_state(JobKind::Query, &JobState::Running { rows: 0 })
            .0
            .fg,
        Some(t.teal)
    );
}

#[test]
fn session_dots_and_driver_badges() {
    let t = Theme::DARK;
    let open = SessionStatus::Open {
        label: "pg 16".into(),
    };
    assert_eq!(t.session_dot(&open), (t.spinner(), "●"));
    assert_eq!(t.session_dot(&SessionStatus::Connecting), (t.dim(), "◌"));
    assert_eq!(t.session_dot(&SessionStatus::Closed), (t.dim(), "◌"));
    assert_eq!(
        t.session_dot(&SessionStatus::Failed("refused".into())),
        (t.value_error(), "✗")
    );
    for (driver, label) in [
        (Driver::Postgres, "pg"),
        (Driver::Mysql, "my"),
        (Driver::Mariadb, "ma"),
        (Driver::Oracle, "ora"),
        (Driver::Sqlite, "lite"),
    ] {
        assert_eq!(t.driver_badge(driver), (t.dim(), label));
    }
}

#[test]
fn toast_levels() {
    let t = Theme::DARK;
    assert_eq!(t.toast(ToastLevel::Error).fg, Some(t.coral));
    assert_eq!(t.toast(ToastLevel::Warning).fg, Some(t.coral));
    assert_eq!(t.toast(ToastLevel::Info).fg, Some(t.fg));
    assert_eq!(t.toast(ToastLevel::Info).bg, Some(t.bg));
}

#[test]
fn sql_error_is_underlined_coral() {
    let t = Theme::DARK;
    let s = t.sql_error();
    assert_eq!(s.fg, Some(t.coral));
    assert_eq!(s.underline_color, Some(t.coral));
    assert!(s.add_modifier.contains(Modifier::UNDERLINED));
}

// ---- resolving ---------------------------------------------------------

#[test]
fn resolve_prefers_the_flag_then_the_config_then_dark() {
    let tc = ColorSupport::TrueColor;
    assert_eq!(resolve(None, None, tc).unwrap(), Theme::DARK);
    assert_eq!(resolve(None, Some("dark"), tc).unwrap(), Theme::DARK);
    // The flag wins, so a bad config value is never looked at.
    assert_eq!(
        resolve(Some("dark"), Some("nope"), tc).unwrap(),
        Theme::DARK
    );
    let err = resolve(Some("nope"), Some("dark"), tc).unwrap_err();
    assert!(err.to_string().contains("`nope`"), "{err}");
}

#[test]
fn resolve_adapts_to_the_terminal() {
    let t = resolve(None, None, ColorSupport::NoColor).unwrap();
    assert!(t.monochrome);
    let t = resolve(None, None, ColorSupport::Ansi256).unwrap();
    assert_eq!(t.bg, Color::Indexed(233));
}
