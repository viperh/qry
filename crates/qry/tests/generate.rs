//! `qry generate` run as the real binary, the way the release jobs and the
//! packages run it: no TTY, and no config or data directory of its own.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};

fn run_generate(args: &[&str]) -> std::process::Output {
    let scratch = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_qry"))
        .arg("generate")
        .args(args)
        // Pipes, not a terminal: stdin is empty, stdout is captured.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("QRY_CONFIG", scratch.path().join("config"))
        .env("QRY_DATA", scratch.path().join("data"))
        .env("QRY_KEYRING", "off")
        .output()
        .unwrap()
}

#[test]
fn generate_never_touches_the_terminal() {
    let scratch = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_qry"))
        .args(["generate", "man"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("QRY_CONFIG", scratch.path().join("config"))
        .env("QRY_DATA", scratch.path().join("data"))
        .env("QRY_KEYRING", "off")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "exit {:?}, stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(".SH ENVIRONMENT"), "{stdout}");
    // No escape sequence at all: no alternate screen (\x1b[?1049h), no
    // cursor hiding, no colour.
    assert!(!stdout.contains('\x1b'), "escape sequence in: {stdout:?}");
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    // Nothing was set up: no log file in the data directory.
    assert!(!scratch.path().join("data").exists());
}

#[test]
fn generate_completions_from_the_binary() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let output = run_generate(&["completions", shell]);
        assert!(output.status.success(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(script.contains("qry"), "{shell}: {script}");
        assert!(!script.contains('\x1b'), "{shell}: escape sequence");
    }
}

#[test]
fn generate_rejects_an_unknown_shell() {
    let output = run_generate(&["completions", "tcsh"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}
