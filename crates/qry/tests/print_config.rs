//! `qry --print-config`, run as a user would: it prints the keymap in
//! effect and exits, and a bad user file stops it with the file's name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Output};

/// Runs the binary with `--print-config`, with its config directory holding
/// `config_json` (or nothing) and its data directory a scratch one.
fn print_config(config_json: Option<&str>) -> (Output, tempfile::TempDir) {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    if let Some(json) = config_json {
        std::fs::write(config.path().join("config.json"), json).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_qry"))
        .arg("--print-config")
        .env("QRY_CONFIG", config.path())
        .env("QRY_DATA", data.path())
        .env("QRY_KEYRING", "off")
        .output()
        .unwrap();
    (output, config)
}

#[test]
fn prints_the_defaults_and_exits() {
    let (output, _dir) = print_config(None);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("{\n"), "{stdout}");
    assert!(stdout.contains(r#""ctrl-q": "Quit""#), "{stdout}");
    assert!(stdout.contains(r#""g g": "Top""#), "{stdout}");
    assert!(
        output.stderr.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn prints_a_user_override_and_warns_about_the_old_format() {
    let (output, _dir) = print_config(Some(r#"{"keybindings": {"Home": {"<f2>": "Quit"}}}"#));
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(r#""F2": "Quit""#), "{stdout}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("old keybinding format"), "{stderr}");
}

#[test]
fn an_unknown_action_fails_naming_the_file() {
    let (output, dir) = print_config(Some(r#"{"keybindings": {"Results": {"<x>": "Teleport"}}}"#));
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let file = dir.path().join("config.json").display().to_string();
    for part in [file.as_str(), "Results", "<x>", "Teleport"] {
        assert!(stderr.contains(part), "{part} missing from: {stderr}");
    }
}
