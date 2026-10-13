//! Unit tests for `secrets.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_password_cannot_be_printed() {
    let password = Password::new("hunter2");
    assert_eq!(format!("{password:?}"), "[redacted]");
    assert_eq!(
        format!("{:?}", Some(password.duplicate())),
        "Some([redacted])"
    );
    assert_eq!(password.as_str(), "hunter2");
}

#[test]
fn a_password_is_typed_and_erased_by_character() {
    let mut password = Password::default();
    for (at, c) in "hélo".chars().enumerate() {
        password.insert(at, c);
    }
    password.insert(3, 'l');
    assert_eq!(password.as_str(), "héllo");
    assert_eq!(password.chars(), 5);

    password.remove(1);
    password.remove(9);
    assert_eq!(password.as_str(), "hllo");
    // Long enough to outgrow its first buffer.
    for _ in 0..40 {
        password.insert(0, 'x');
    }
    assert_eq!(password.chars(), 44);
}

#[test]
fn the_stash_hands_a_password_over_once() {
    let _guard = test_lock();
    stash(Password::new("hunter2"));
    assert_eq!(
        take_stash().map(|p| p.as_str().to_string()),
        Some("hunter2".into())
    );
    assert_eq!(take_stash(), None, "taken twice");
}

#[test]
fn an_environment_variable_is_read_only_when_it_has_a_value() {
    set_test_var("QRY_TEST_PASSWORD", "hunter2");
    set_test_var("QRY_TEST_EMPTY", "");
    assert_eq!(
        from_env("QRY_TEST_PASSWORD").map(|p| p.as_str().to_string()),
        Some("hunter2".into())
    );
    assert_eq!(from_env("QRY_TEST_EMPTY"), None);
    assert_eq!(from_env("QRY_TEST_UNSET"), None);
}

#[test]
fn dsn_passwords_are_redacted() {
    assert_eq!(
        redact("error connecting to postgres://alice:hunter2@db.local/app"),
        "error connecting to postgres://alice:[redacted]@db.local/app"
    );
    assert_eq!(
        redact("mysql://root:s3cret@127.0.0.1:3306/app failed"),
        "mysql://root:[redacted]@127.0.0.1:3306/app failed"
    );
    // No credentials in the authority: left as it was.
    assert_eq!(redact("postgres://db.local/app"), "postgres://db.local/app");
}

#[test]
fn key_value_passwords_are_redacted() {
    assert_eq!(
        redact("host=db.local user=alice password=hunter2 dbname=app"),
        "host=db.local user=alice password=[redacted] dbname=app"
    );
    assert_eq!(
        redact("Server=db;Password=hunter2;Database=app"),
        "Server=db;Password=[redacted];Database=app"
    );
}

#[test]
fn unit_tests_keep_passwords_in_memory_not_the_keychain() {
    assert!(!keychain_enabled());
    // A made-up id, so a real keychain entry could never be read here.
    let id = "qry-test-in-memory-keychain";
    assert_eq!(fetch(id).unwrap(), None);
    store(id, &Password::new("hunter2")).unwrap();
    assert_eq!(fetch(id).unwrap().unwrap().as_str(), "hunter2");
    delete(id).unwrap();
    assert_eq!(fetch(id).unwrap(), None);
    // Deleting what is not there is not an error, as with the keychain.
    delete(id).unwrap();
}
