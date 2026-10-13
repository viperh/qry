//! Unit tests for `config.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pretty_assertions::assert_eq;

use super::*;

#[test]
fn test_config() -> color_eyre::Result<()> {
    let c = Config::new()?;
    assert_eq!(
        c.keybindings
            .0
            .get(&KeySection::Home)
            .unwrap()
            .get(&parse_key_sequence("<ctrl-q>").unwrap_or_default())
            .unwrap(),
        &Action::Quit
    );
    Ok(())
}

#[test]
fn test_malformed_key_sequence_is_err() {
    let result = json5::from_str::<KeyBindings>(r#"{"Home": {"<ctrl-notakey>": "Quit"}}"#);
    assert!(result.is_err());
}

#[test]
fn test_simple_keys() {
    assert_eq!(
        parse_key_event("a").unwrap(),
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty())
    );

    assert_eq!(
        parse_key_event("enter").unwrap(),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())
    );

    assert_eq!(
        parse_key_event("esc").unwrap(),
        KeyEvent::new(KeyCode::Esc, KeyModifiers::empty())
    );
}

#[test]
fn test_with_modifiers() {
    assert_eq!(
        parse_key_event("ctrl-a").unwrap(),
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)
    );

    assert_eq!(
        parse_key_event("alt-enter").unwrap(),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    );

    assert_eq!(
        parse_key_event("shift-esc").unwrap(),
        KeyEvent::new(KeyCode::Esc, KeyModifiers::SHIFT)
    );
}

#[test]
fn test_multiple_modifiers() {
    assert_eq!(
        parse_key_event("ctrl-alt-a").unwrap(),
        KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )
    );

    assert_eq!(
        parse_key_event("ctrl-shift-enter").unwrap(),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT)
    );
}

#[test]
fn test_reverse_multiple_modifiers() {
    assert_eq!(
        key_event_to_string(&KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )),
        "ctrl-alt-a".to_string()
    );
}

#[test]
fn test_invalid_keys() {
    assert!(parse_key_event("invalid-key").is_err());
    assert!(parse_key_event("ctrl-invalid-key").is_err());
}

#[test]
fn test_case_insensitivity() {
    assert_eq!(
        parse_key_event("CTRL-a").unwrap(),
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)
    );

    assert_eq!(
        parse_key_event("AlT-eNtEr").unwrap(),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    );
}

/// Loads `json` the way [`Config::new`] loads a user's config file.
fn from_user_file(json: &str) -> Config {
    config::Config::builder()
        .add_source(config::File::from_str(json, config::FileFormat::Json))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap()
}

#[test]
fn the_embedded_config_names_the_dark_theme() {
    let c = Config::embedded();
    assert_eq!(c.theme.as_deref(), Some("dark"));
    assert!(c.warnings().is_empty(), "{:?}", c.warnings());
}

#[test]
fn theme_key_is_read() {
    assert_eq!(
        from_user_file(r#"{"theme": "dark"}"#).theme.as_deref(),
        Some("dark")
    );
    assert_eq!(from_user_file("{}").theme, None);
}

#[test]
fn unknown_theme_is_an_error_listing_names() {
    let c = from_user_file(r#"{"theme": "neon"}"#);
    let err = crate::theme::resolve(
        None,
        c.theme.as_deref(),
        crate::theme::ColorSupport::TrueColor,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("`neon`"), "{err}");
    for name in crate::theme::Theme::NAMES {
        assert!(err.contains(name), "{name} missing from: {err}");
    }
}

#[test]
fn legacy_styles_section_warns_and_loads() {
    let c = from_user_file(
        r#"{
            "keybindings": {"Home": {"<ctrl-q>": "Quit"}},
            "styles": {"Home": {"title": "bold red on blue"}}
        }"#,
    );
    assert_eq!(
        c.keybindings.0[&KeySection::Home][&parse_key_sequence("<ctrl-q>").unwrap()],
        Action::Quit
    );
    let warnings = c.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("\"styles\""), "{}", warnings[0]);
    assert!(!warnings[0].contains('\n'), "one line: {}", warnings[0]);
}
