//! The query history's files, and the snippets file.
//!
//! Each connection's history is `<data dir>/history/<connection id>.jsonl`,
//! one JSON line per statement (`qry_core::history` reads and writes the
//! lines). A statement is one append of one whole line, with a single
//! `write_all`, so two qry instances appending at once interleave lines but
//! never break one. Compaction rewrites the file to a temporary one in the
//! same directory and renames it over, holding an advisory lock file while
//! it does; an instance that finds the lock taken skips compacting, as the
//! next one will do it. On unix every file is created `0600`.
//!
//! The runtime does the I/O off the event loop; the reducer only asks for it
//! with [`Effect::History`](crate::app::Effect::History). [`HistoryStore`]
//! is what it writes through, so tests can use the in-memory
//! [`MemoryStore`].

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use qry_core::{
    history::History,
    snippets::{self, Snippet},
};

/// A write the reducer asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryWrite {
    /// Add this line (with its newline) to the connection's file.
    Append { connection: String, line: String },
    /// Replace the connection's file with this: compaction, or an entry
    /// deleted from the dialog.
    Rewrite { connection: String, content: String },
}

/// One connection's history as read from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub connection: String,
    pub history: History,
    /// How many lines the file has, merged duplicates and skipped ones
    /// included: what compaction is judged by.
    pub lines: usize,
}

/// Where the history lives. Every method blocks; the runtime calls them
/// from `spawn_blocking`.
pub trait HistoryStore: std::fmt::Debug + Send + Sync {
    /// Every connection's file: its id and its text. No directory yet is no
    /// history, not an error.
    fn load_all(&self) -> io::Result<Vec<(String, String)>>;
    /// Appends one whole line to the connection's file.
    fn append(&self, connection: &str, line: &str) -> io::Result<()>;
    /// Replaces the connection's file. `Ok(false)` when another instance is
    /// rewriting it, so this one did not.
    fn rewrite(&self, connection: &str, content: &str) -> io::Result<bool>;
}

/// Carries out `write` on `store`.
pub fn perform(store: &dyn HistoryStore, write: &HistoryWrite) -> io::Result<()> {
    match write {
        HistoryWrite::Append { connection, line } => store.append(connection, line),
        HistoryWrite::Rewrite {
            connection,
            content,
        } => store.rewrite(connection, content).map(|_| ()),
    }
}

/// Reads every connection's history. A line that cannot be read is skipped
/// and logged; the rest of its file still loads.
pub fn load(store: &dyn HistoryStore) -> io::Result<Vec<Loaded>> {
    let mut loaded = Vec::new();
    for (connection, text) in store.load_all()? {
        let (history, warnings) = History::from_jsonl(&text);
        for warning in &warnings {
            tracing::warn!("history of {connection}: skipped {warning}");
        }
        loaded.push(Loaded {
            connection,
            history,
            lines: text.lines().filter(|l| !l.trim().is_empty()).count(),
        });
    }
    Ok(loaded)
}

/// The history files under one directory: `<data dir>/history`.
#[derive(Debug, Clone)]
pub struct FileStore {
    dir: PathBuf,
}

/// A lock file older than this was left by an instance that died while
/// compacting, and is taken over.
const STALE_LOCK: Duration = Duration::from_secs(60);

impl FileStore {
    /// The store for the data directory `data_dir`.
    pub fn new(data_dir: &Path) -> Self {
        Self {
            dir: data_dir.join("history"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, connection: &str, extension: &str) -> PathBuf {
        self.dir
            .join(format!("{}.{extension}", file_stem(connection)))
    }

    /// Takes the connection's compaction lock: `None` if another instance
    /// holds it.
    fn lock(&self, connection: &str) -> io::Result<Option<LockFile>> {
        let path = self.path(connection, "lock");
        for _ in 0..2 {
            match private(OpenOptions::new().write(true).create_new(true)).open(&path) {
                Ok(_) => return Ok(Some(LockFile(path))),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let age = fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok());
                    if age.is_none_or(|age| age < STALE_LOCK) {
                        return Ok(None);
                    }
                    let _ = fs::remove_file(&path);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

/// Removes the lock file when the rewrite is done, however it ended.
struct LockFile(PathBuf);

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl HistoryStore for FileStore {
    fn load_all(&self) -> io::Result<Vec<(String, String)>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut files = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Some(connection) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(connection_of)
            else {
                continue;
            };
            // Not UTF-8 in places: read what can be, line by line.
            let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
            files.push((connection, text));
        }
        files.sort();
        Ok(files)
    }

    fn append(&self, connection: &str, line: &str) -> io::Result<()> {
        create_dir(&self.dir)?;
        let mut file = private(OpenOptions::new().append(true).create(true))
            .open(self.path(connection, "jsonl"))?;
        // One write of the whole line, so a concurrent append cannot land
        // inside it.
        file.write_all(line.as_bytes())
    }

    fn rewrite(&self, connection: &str, content: &str) -> io::Result<bool> {
        create_dir(&self.dir)?;
        let Some(_lock) = self.lock(connection)? else {
            return Ok(false);
        };
        let target = self.path(connection, "jsonl");
        let temp = self.path(connection, &format!("jsonl.tmp{}", std::process::id()));
        let written = (|| {
            let mut file =
                private(OpenOptions::new().write(true).create(true).truncate(true)).open(&temp)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temp, &target)
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temp);
        }
        written.map(|()| true)
    }
}

/// History kept in memory, for tests: what each connection's file would
/// hold.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    files: Arc<Mutex<BTreeMap<String, String>>>,
}

impl MemoryStore {
    /// What connection's file holds, if it was written.
    pub fn file(&self, connection: &str) -> Option<String> {
        self.files().get(connection).cloned()
    }

    fn files(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, String>> {
        self.files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl HistoryStore for MemoryStore {
    fn load_all(&self) -> io::Result<Vec<(String, String)>> {
        Ok(self
            .files()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }

    fn append(&self, connection: &str, line: &str) -> io::Result<()> {
        self.files()
            .entry(connection.to_string())
            .or_default()
            .push_str(line);
        Ok(())
    }

    fn rewrite(&self, connection: &str, content: &str) -> io::Result<bool> {
        self.files()
            .insert(connection.to_string(), content.to_string());
        Ok(true)
    }
}

/// A connection id as a file name: letters, digits, `-` and `_` stay, and
/// any other byte becomes `%XX`, so no id can climb out of the directory.
pub fn file_stem(connection: &str) -> String {
    let mut out = String::with_capacity(connection.len());
    for b in connection.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The connection id a file name stands for; `None` for one
/// [`file_stem`] would not have written.
pub fn connection_of(stem: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(stem.len());
    let mut rest = stem.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        if b == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(b);
            rest = tail;
        }
    }
    String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

/// `options` creating files only the user can read, on unix.
fn private(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn create_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir)
}

/// `<config dir>/snippets.json`.
pub fn snippets_path(config_dir: &Path) -> PathBuf {
    config_dir.join("snippets.json")
}

/// The saved snippets; none without a file.
pub fn load_snippets(config_dir: &Path) -> color_eyre::Result<Vec<Snippet>> {
    let path = snippets_path(config_dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(color_eyre::eyre::eyre!(
                "cannot read {}: {e}",
                path.display()
            ));
        }
    };
    snippets::from_json(&text)
        .map_err(|e| color_eyre::eyre::eyre!("cannot read {}: {e}", path.display()))
}

/// Writes the snippets, through a temporary file renamed over the old one.
pub fn save_snippets(config_dir: &Path, list: &[Snippet]) -> io::Result<()> {
    create_dir(config_dir)?;
    let target = snippets_path(config_dir);
    let temp = config_dir.join(format!("snippets.json.tmp{}", std::process::id()));
    let mut file: File =
        private(OpenOptions::new().write(true).create(true).truncate(true)).open(&temp)?;
    file.write_all(snippets::to_json(list).as_bytes())?;
    file.sync_all()?;
    fs::rename(&temp, &target)
}

#[cfg(test)]
#[path = "history_store_tests.rs"]
mod history_store_tests;
