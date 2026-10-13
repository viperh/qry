//! Passwords: where they come from, and how they are kept while in use.
//!
//! Two rules shape this module. A password is never written to a file qry
//! controls — the OS keychain, an environment variable or a prompt are the
//! only sources. And a password never travels inside an event, and only
//! redacted inside an effect, so it cannot reach the screen, the log or a
//! panic message: the runtime leaves what the prompt took in [`stash`] and
//! the database worker takes it from there.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
};

use zeroize::Zeroizing;

/// The keychain service name; the account is the connection's id.
const SERVICE: &str = "qry";

/// A password. Cleared when dropped, and it cannot be printed: its `Debug`
/// shows `[redacted]` and it has no `Display`. It is not `Clone` either, so a
/// copy is always an explicit [`Password::duplicate`] rather than a stray
/// `.clone()`.
#[derive(Default, PartialEq, Eq)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A second copy, for the one place that has to keep one too (the
    /// in-memory keychain, and a prompt's buffer when the state is cloned).
    pub(crate) fn duplicate(&self) -> Self {
        Self::new(self.as_str())
    }

    /// Length in characters, for a cursor that moves by character.
    pub(crate) fn chars(&self) -> usize {
        self.0.chars().count()
    }

    /// Inserts `c` before the character at `at`, typing one key into a
    /// prompt. The text moves to a fresh buffer before it can outgrow this
    /// one, so a reallocation never leaves a copy behind uncleared.
    pub(crate) fn insert(&mut self, at: usize, c: char) {
        if self.0.len() + c.len_utf8() > self.0.capacity() {
            let mut grown = Zeroizing::new(String::with_capacity((self.0.capacity() * 2).max(16)));
            grown.push_str(&self.0);
            // The old buffer is cleared as it drops.
            self.0 = grown;
        }
        let at = self.byte_index(at);
        self.0.insert(at, c);
    }

    /// Removes the character at `at`, if there is one. Removing never
    /// reallocates, so nothing is left behind.
    pub(crate) fn remove(&mut self, at: usize) {
        if at < self.chars() {
            let at = self.byte_index(at);
            self.0.remove(at);
        }
    }

    fn byte_index(&self, at: usize) -> usize {
        self.0
            .char_indices()
            .nth(at)
            .map_or(self.0.len(), |(i, _)| i)
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Where a prompted password waits between the modal and the worker, so that
/// it never has to ride in a message to the worker. Keyed by connection id:
/// two sessions can be asking at once, and each worker takes only the
/// password typed for its own connection.
static STASH: Mutex<BTreeMap<String, Password>> = Mutex::new(BTreeMap::new());

/// Leaves `password` for the worker opening connection `id`, in place of any
/// left for it before.
pub(crate) fn stash(id: &str, password: Password) {
    if let Ok(mut held) = STASH.lock() {
        held.insert(id.to_string(), password);
    }
}

/// The stash is process-wide, so tests that use it take this first rather
/// than stealing each other's passwords.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sets an environment variable for a test.
///
/// This is the one place in the workspace allowed to use `unsafe` (the
/// workspace lints deny it everywhere else). `std::env::set_var` is unsafe
/// since edition 2024 because another thread could be reading the
/// environment through libc at the same moment. Tests only set `QRY_TEST_*`
/// names that no other code reads, and std serialises its own environment
/// access, so a concurrent read can only come from a C library, which the
/// unit tests do not call while setting these.
#[cfg(test)]
#[allow(unsafe_code, reason = "edition 2024 makes set_var unsafe; see above")]
pub(crate) fn set_test_var(name: &str, value: &str) {
    debug_assert!(
        name.starts_with("QRY_TEST_"),
        "test variables are QRY_TEST_*"
    );
    // SAFETY: see the function documentation.
    unsafe { std::env::set_var(name, value) };
}

/// Takes what the prompt left for connection `id`, if anything.
pub(crate) fn take_stash(id: &str) -> Option<Password> {
    STASH.lock().ok().and_then(|mut held| held.remove(id))
}

/// Set `QRY_KEYRING=off` to keep passwords in memory instead of the OS
/// keychain, for the life of the process. Tests run this way so they never
/// prompt for, read or write the developer's real keychain; this crate's own
/// unit tests always do, whatever the variable says.
pub const KEYRING_ENV: &str = "QRY_KEYRING";

/// The stand-in keychain used when the real one is switched off, keyed by
/// connection id.
static MEMORY: Mutex<Option<HashMap<String, Password>>> = Mutex::new(None);

/// Whether passwords go to the OS keychain rather than [`MEMORY`].
fn keychain_enabled() -> bool {
    !cfg!(test) && !std::env::var(KEYRING_ENV).is_ok_and(|value| value.eq_ignore_ascii_case("off"))
}

/// Runs `f` on the in-memory keychain. A poisoned lock is taken anyway: a
/// single insert or remove cannot leave the map half-changed.
fn with_memory<T>(f: impl FnOnce(&mut HashMap<String, Password>) -> T) -> T {
    let mut held = MEMORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(held.get_or_insert_with(HashMap::new))
}

/// Stores a password in the OS keychain under a connection's id. Blocking:
/// call it from `spawn_blocking`.
pub(crate) fn store(id: &str, password: &Password) -> color_eyre::Result<()> {
    if !keychain_enabled() {
        with_memory(|memory| memory.insert(id.to_string(), password.duplicate()));
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, id)?;
    entry.set_password(password.as_str())?;
    Ok(())
}

/// Reads a connection's password from the OS keychain. `Ok(None)` means the
/// keychain works but has no entry; an error means it could not be reached,
/// which is the cue to prompt instead.
pub(crate) fn fetch(id: &str) -> color_eyre::Result<Option<Password>> {
    if !keychain_enabled() {
        return Ok(with_memory(|memory| {
            memory.get(id).map(Password::duplicate)
        }));
    }
    let entry = keyring::Entry::new(SERVICE, id)?;
    match entry.get_password() {
        Ok(password) => Ok(Some(Password::new(password))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Removes a connection's password. A missing entry is not an error: the end
/// state is the same.
pub(crate) fn delete(id: &str) -> color_eyre::Result<()> {
    if !keychain_enabled() {
        with_memory(|memory| memory.remove(id));
        return Ok(());
    }
    let entry = keyring::Entry::new(SERVICE, id)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A password from a named environment variable, if it is set and not empty.
pub(crate) fn from_env(var: &str) -> Option<Password> {
    match std::env::var(var) {
        Ok(value) if !value.is_empty() => Some(Password::new(value)),
        _ => None,
    }
}

/// Replaces the password in anything that looks like a DSN, so an error or a
/// status message cannot carry one to the screen or the log.
pub(crate) fn redact(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;

    // `scheme://user:password@host`: everything between the first colon after
    // `//` and the `@` that ends the credentials.
    while let Some(start) = rest.find("://") {
        let (before, after) = rest.split_at(start + 3);
        out.push_str(before);
        let end = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
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
#[path = "secrets_tests.rs"]
mod secrets_tests;
