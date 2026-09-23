//! Passwords: where they come from, and how they are kept while in use.
//!
//! Two rules shape this module. A password is never written to a file qry
//! controls — the OS keychain, an environment variable or a prompt are the
//! only sources. And a password never travels inside an
//! [`Action`](crate::action::Action), so it cannot reach the screen, the log
//! or a panic message: the prompt leaves it in [`stash`] and the database
//! worker takes it from there.

use std::sync::Mutex;

use zeroize::Zeroizing;

/// The keychain service name; the account is the connection's id.
const SERVICE: &str = "qry";

/// A password. Cleared when dropped, and it cannot be printed: its `Debug`
/// shows `[redacted]` and it has no `Display`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Where a prompted password waits between the modal and the worker, so that
/// it never has to ride in an action. One prompt is open at a time.
static STASH: Mutex<Option<Password>> = Mutex::new(None);

pub fn stash(password: Password) {
    if let Ok(mut held) = STASH.lock() {
        *held = Some(password);
    }
}

/// The stash is process-wide, so tests that use it take this first rather
/// than stealing each other's passwords.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Takes what the prompt left, if anything.
pub fn take_stash() -> Option<Password> {
    STASH.lock().ok().and_then(|mut held| held.take())
}

/// Stores a password in the OS keychain under a connection's id. Blocking:
/// call it from `spawn_blocking`.
pub fn store(id: &str, password: &Password) -> color_eyre::Result<()> {
    let entry = keyring::Entry::new(SERVICE, id)?;
    entry.set_password(password.as_str())?;
    Ok(())
}

/// Reads a connection's password from the OS keychain. `Ok(None)` means the
/// keychain works but has no entry; an error means it could not be reached,
/// which is the cue to prompt instead.
pub fn fetch(id: &str) -> color_eyre::Result<Option<Password>> {
    let entry = keyring::Entry::new(SERVICE, id)?;
    match entry.get_password() {
        Ok(password) => Ok(Some(Password::new(password))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Removes a connection's password. A missing entry is not an error: the end
/// state is the same.
pub fn delete(id: &str) -> color_eyre::Result<()> {
    let entry = keyring::Entry::new(SERVICE, id)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A password from a named environment variable, if it is set and not empty.
pub fn from_env(var: &str) -> Option<Password> {
    match std::env::var(var) {
        Ok(value) if !value.is_empty() => Some(Password::new(value)),
        _ => None,
    }
}

/// Replaces the password in anything that looks like a DSN, so an error or a
/// status message cannot carry one to the screen or the log.
pub fn redact(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;

    // `scheme://user:password@host`: everything between the first colon after
    // `//` and the `@` that ends the credentials.
    while let Some(start) = rest.find("://") {
        let (before, after) = rest.split_at(start + 3);
        out.push_str(before);
        let end = after.find(|c: char| c.is_whitespace()).unwrap_or(after.len());
        let (authority, tail) = after.split_at(end);
        match (authority.find(':'), authority.find('@')) {
            (Some(colon), Some(at)) if colon < at => {
                out.push_str(&authority[..=colon]);
                out.push_str("[redacted]");
                out.push_str(&authority[at..]);
            }
            _ => out.push_str(authority),
        }
        rest = tail;
    }
    out.push_str(rest);

    // `password=secret` as libpq and friends write it.
    redact_key_value(&out, "password=")
}

fn redact_key_value(message: &str, key: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.to_lowercase().find(key) {
        let (before, after) = rest.split_at(start + key.len());
        out.push_str(before);
        out.push_str("[redacted]");
        let end = after
            .find(|c: char| c.is_whitespace() || c == ';' || c == '&')
            .unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_cannot_be_printed() {
        let password = Password::new("hunter2");
        assert_eq!(format!("{password:?}"), "[redacted]");
        assert_eq!(format!("{:?}", Some(password.clone())), "Some([redacted])");
        assert_eq!(password.as_str(), "hunter2");
    }

    #[test]
    fn the_stash_hands_a_password_over_once() {
        let _guard = test_lock();
        stash(Password::new("hunter2"));
        assert_eq!(take_stash().map(|p| p.as_str().to_string()), Some("hunter2".into()));
        assert_eq!(take_stash(), None, "taken twice");
    }

    #[test]
    fn an_environment_variable_is_read_only_when_it_has_a_value() {
        // SAFETY: single-threaded within this test, and the names are its own.
        unsafe {
            std::env::set_var("QRY_TEST_PASSWORD", "hunter2");
            std::env::set_var("QRY_TEST_EMPTY", "");
        }
        assert_eq!(from_env("QRY_TEST_PASSWORD").map(|p| p.as_str().to_string()), Some("hunter2".into()));
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
}
