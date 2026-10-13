//! Drives the app without a terminal, a runtime or a database server.
//!
//! Compiled for this crate's own tests and, through the `test-util` feature,
//! for other crates' dev-dependencies (this crate's integration tests get it
//! from the self dev-dependency in `Cargo.toml`). A release build never
//! contains it.
//!
//! ```ignore
//! let mut h = AppHarness::new(Config::embedded());
//! h.keys("ctrl-n");                       // chord list in display syntax
//! insta::assert_snapshot!(h.render(80, 24));
//! h.keys("esc");
//! assert!(h.take_effects().is_empty());
//! ```
//!
//! [`AppHarness`] drives [`App::handle`] the way the runtime does: keys,
//! ticks and answers go in as [`UiEvent`]s, and the [effects](Effect) that
//! come back are recorded instead of being carried out. With a
//! [`FakeDatabase`], the database effects are answered on a virtual clock.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub use qry_core::testing::{Reply, grid, result};
use qry_core::{QueryResult, testing::FakeDatabase};
use ratatui::{Frame, Terminal, backend::TestBackend, buffer::Buffer};

pub use crate::{app::Effect, theme::ThemeEnv};
use crate::{
    app::{App, DbAnswer, FileAnswer, SessionId, State, UiEvent},
    config::{Config, KeySection, parse_key_event},
    connections::StoredConnection,
    db::{self, DbCommand},
    theme::Theme,
    views,
};

/// Terminal sizes every view must survive, from nothing at all to a very
/// large screen. `every_view_renders_at_every_size` renders each view at each.
pub const SIZES: &[(u16, u16)] = &[
    (0, 0),
    (1, 1),
    (10, 3),
    (80, 24),
    (120, 40),
    (160, 48),
    (300, 100),
];

/// Answers the stand-in worker will deliver once the virtual clock reaches
/// `due`, in milliseconds since the harness was made.
#[derive(Debug)]
struct Pending {
    due: u64,
    answers: Vec<DbAnswer>,
}

/// Drives the app with scripted keys and events on a virtual clock.
#[derive(Debug)]
pub struct AppHarness {
    app: App,
    effects: Vec<Effect>,
    /// Events waiting to be handled, such as a worker's immediate answer.
    inbox: VecDeque<UiEvent>,
    /// The moment virtual time started, for the instants ticks carry.
    epoch: Instant,
    /// Virtual time, in milliseconds since the harness was made.
    now: u64,
    /// Milliseconds between ticks, from the tick rate.
    tick_every: u64,
    next_tick: u64,
    ticks: u64,
    theme_env: ThemeEnv,
    database: Option<FakeDatabase>,
    /// Worker answers not yet due, in the order they were scheduled.
    pending: Vec<Pending>,
    /// When the stand-in worker finishes what it already has: it runs one
    /// command at a time, like the real one.
    worker_free_at: u64,
}

/// The tick rate the binary uses by default (`--tick-rate 4`).
const DEFAULT_TICK_RATE: f64 = 4.0;

impl AppHarness {
    /// An app with this config, drawing [`Theme::DARK`] in truecolor, before
    /// [`UiEvent::Started`]: nothing is loaded from disk and no connection
    /// is opened until a test sends it.
    pub fn new(config: Config) -> Self {
        let theme_env = ThemeEnv::truecolor();
        let tick_every = tick_interval(DEFAULT_TICK_RATE);
        Self {
            app: App::new(config, theme_env.adapt(Theme::DARK), 0),
            effects: Vec::new(),
            inbox: VecDeque::new(),
            epoch: instant(0),
            now: 0,
            tick_every,
            next_tick: tick_every,
            ticks: 0,
            theme_env,
            database: None,
            pending: Vec::new(),
            worker_free_at: 0,
        }
    }

    /// Answers `Connect`, `OpenSession`, `Query` and `ListTables` from `db`,
    /// on the virtual clock: a [`Reply::Delay`] arrives once [`advance`]
    /// passes it, and a [`Reply::Hang`] never does. The requests are still
    /// recorded as effects. Without a database they are only recorded.
    ///
    /// [`advance`]: Self::advance
    #[must_use]
    pub fn with_database(mut self, db: FakeDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Draws as a terminal with this environment would: 256 colours, or no
    /// colour under `NO_COLOR`. The default is [`ThemeEnv::truecolor`], so
    /// styled snapshots show the palette's exact values.
    #[must_use]
    pub fn with_theme_env(mut self, env: ThemeEnv) -> Self {
        self.app.set_theme(env.adapt(Theme::DARK));
        self.theme_env = env;
        self
    }

    /// Ticks this many times a second instead of the default 4.
    #[must_use]
    pub fn with_tick_rate(mut self, per_second: f64) -> Self {
        self.tick_every = tick_interval(per_second);
        self.next_tick = self.now + self.tick_every;
        self
    }

    /// The app's state, for tests that check what the screen does not show.
    pub fn state(&self) -> &State {
        self.app.state()
    }

    /// The app's state, for this crate's tests that set up state no key
    /// reaches directly, such as which form field has focus.
    #[cfg(test)]
    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.app.state
    }

    /// The terminal environment set with [`with_theme_env`](Self::with_theme_env).
    pub fn theme_env(&self) -> &ThemeEnv {
        &self.theme_env
    }

    /// The theme the harness draws with, adapted to [`theme_env`](Self::theme_env).
    pub fn theme(&self) -> &Theme {
        &self.app.state().theme
    }

    /// The `keybindings` section the keys are looked up in: which modal, if
    /// any, is open.
    pub fn section(&self) -> KeySection {
        self.app.state().key_section()
    }

    /// How many `Tick`s [`advance`](Self::advance) has delivered so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// The virtual time since the harness was made.
    pub fn now(&self) -> Duration {
        Duration::from_millis(self.now)
    }

    /// Presses each whitespace-separated key in `spec`, such as
    /// `"ctrl-p j j enter"`; see [`parse_keys`] for the syntax.
    ///
    /// # Panics
    /// On a key it cannot read, naming it.
    pub fn keys(&mut self, spec: &str) -> &mut Self {
        for key in parse_keys(spec) {
            self.key(key);
        }
        self
    }

    /// Presses one key, and handles everything that follows from it.
    pub fn key(&mut self, key: KeyEvent) -> &mut Self {
        self.send(UiEvent::Key(key))
    }

    /// Types `text`, one `Char` key per character. Capitals carry Shift, as
    /// terminals report them.
    pub fn type_str(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            let modifiers = if c.is_uppercase() {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            self.key(KeyEvent::new(KeyCode::Char(c), modifiers));
        }
        self
    }

    /// Delivers an event, such as a worker's `QueryDone` or a file's
    /// `ImportFailed`, and everything that follows from it.
    pub fn send(&mut self, event: UiEvent) -> &mut Self {
        self.inbox.push_back(event);
        self.drain();
        self
    }

    /// Delivers an answer from the session's database worker.
    pub fn db(&mut self, answer: DbAnswer) -> &mut Self {
        self.send(UiEvent::Db(SessionId::FIRST, answer))
    }

    /// Delivers the outcome of a file the runtime read or wrote.
    pub fn file(&mut self, answer: FileAnswer) -> &mut Self {
        self.send(UiEvent::File(answer))
    }

    /// Resizes the terminal. The next [`render`](Self::render) still draws at
    /// the size it is given.
    pub fn resize(&mut self, w: u16, h: u16) -> &mut Self {
        self.send(UiEvent::Resize(w, h))
    }

    /// Moves the virtual clock on by `ms`, delivering every `Tick` and every
    /// worker answer that falls due, in time order. Nothing waits in real
    /// time.
    pub fn advance(&mut self, ms: u64) -> &mut Self {
        let target = self.now.saturating_add(ms);
        loop {
            let answer = self
                .pending
                .iter()
                .enumerate()
                .min_by_key(|(_, pending)| pending.due)
                .map(|(index, pending)| (index, pending.due));
            match answer {
                // An answer due at the same moment as a tick comes first.
                Some((index, due)) if due <= target && due <= self.next_tick => {
                    self.now = due;
                    let pending = self.pending.remove(index);
                    self.deliver(pending.answers);
                    self.drain();
                }
                _ if self.next_tick <= target => {
                    self.now = self.next_tick;
                    self.next_tick += self.tick_every;
                    self.ticks += 1;
                    let at = self.epoch + Duration::from_millis(self.now);
                    self.send(UiEvent::Tick(at));
                }
                _ => break,
            }
        }
        self.now = target;
        self
    }

    /// Returns and clears the effects recorded so far.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Draws the screen into a `w`x`h` terminal and returns it as text; see
    /// [`buffer_to_string`].
    pub fn render(&mut self, w: u16, h: u16) -> String {
        buffer_to_string(&self.render_buffer(w, h))
    }

    /// Draws the screen into a `w`x`h` terminal and returns the cells, with
    /// their styles; see [`styled`].
    pub fn render_buffer(&mut self, w: u16, h: u16) -> Buffer {
        let state = self.app.state();
        render_with(w, h, |frame| {
            views::draw(state, &state.theme, frame, frame.area())
        })
    }

    /// Handles everything in the inbox, recording the effects and answering
    /// the database's from the fake, until nothing is left.
    fn drain(&mut self) {
        while let Some(event) = self.inbox.pop_front() {
            for effect in self.app.handle(event) {
                match &effect {
                    Effect::Db(_, DbCommand::Connect(record)) | Effect::OpenSession(_, record) => {
                        self.answer_connect(record);
                    }
                    Effect::Db(_, DbCommand::Query(sql)) => self.answer_query(sql),
                    Effect::Db(_, DbCommand::ListTables) => {
                        let answer = (self.database.as_ref())
                            .map(|db| db::tables_answer(Ok::<_, String>(db.table_names())));
                        if let Some(answer) = answer {
                            self.schedule(0, answer);
                        }
                    }
                    _ => {}
                }
                self.effects.push(effect);
            }
        }
    }

    fn deliver(&mut self, answers: Vec<DbAnswer>) {
        self.inbox.extend(
            answers
                .into_iter()
                .map(|answer| UiEvent::Db(SessionId::FIRST, answer)),
        );
    }

    /// Answers a connection from the fake database, if there is one.
    fn answer_connect(&mut self, record: &StoredConnection) {
        let answer = (self.database.as_ref()).map(|db| db::connected_answer(record, db.name()));
        if let Some(answer) = answer {
            self.schedule(0, answer);
        }
    }

    /// Answers a query from the fake database, if there is one.
    fn answer_query(&mut self, sql: &str) {
        let Some(db) = &self.database else {
            return;
        };
        let mut reply = db.next_reply(sql);
        let mut after = 0;
        let outcome = loop {
            match reply {
                Reply::Rows(result) => break Ok(result),
                Reply::Affected(count) => {
                    break Ok(QueryResult {
                        columns: Vec::new(),
                        rows: Vec::new(),
                        affected: Some(count),
                    });
                }
                Reply::Error(message) => break Err(message),
                Reply::Delay(delay, inner) => {
                    after += u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
                    reply = *inner;
                }
                // The worker stays busy for good, as the real one would.
                Reply::Hang => {
                    self.worker_free_at = u64::MAX;
                    return;
                }
            }
        };
        let answer = db::query_answer(outcome, Duration::from_millis(after));
        self.schedule(after, answer);
    }

    /// Delivers `answers` once the stand-in worker has finished what it
    /// already had and then spent `after` ms on this. With nothing to wait
    /// for they arrive straight away, after the event being handled.
    fn schedule(&mut self, after: u64, answers: Vec<DbAnswer>) {
        let start = self.worker_free_at.max(self.now);
        let due = start.saturating_add(after);
        self.worker_free_at = due;
        if due <= self.now {
            self.deliver(answers);
        } else if due != u64::MAX {
            self.pending.push(Pending { due, answers });
        }
    }
}

/// Milliseconds between ticks at `per_second` ticks a second, at least 1.
fn tick_interval(per_second: f64) -> u64 {
    let ms = (1000.0 / per_second).round();
    if ms.is_finite() && ms >= 1.0 {
        ms as u64
    } else {
        1
    }
}

/// The path of a shared fixture under `tests/fixtures/` at the repository
/// root, such as `fixture("dbeaver/data-sources.json")`. Tests read fixtures
/// from there and never write into the repository.
pub fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(relative)
}

/// A moment `ms` milliseconds after a fixed one, for tests that build
/// `UiEvent::Tick`s themselves: the reducer's own modules may not read the
/// clock, not even in their tests.
pub fn instant(ms: u64) -> Instant {
    static EPOCH: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    *EPOCH + Duration::from_millis(ms)
}

/// Every `.rs` file under these directories of this crate's `src/` (and
/// `src/<dir>.rs` itself where there is one), with its text, for tests that
/// scan the source.
///
/// # Panics
/// On a directory or file it cannot read.
#[cfg(test)]
pub(crate) fn sources(dirs: &[&str]) -> Vec<(std::path::PathBuf, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut stack = Vec::new();
    for dir in dirs {
        let file = root.join(format!("{dir}.rs"));
        if file.is_file() {
            files.push(file);
        }
        stack.push(root.join(dir));
    }
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            panic!("cannot read {}", dir.display());
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|path| match std::fs::read_to_string(&path) {
            Ok(text) => (path, text),
            Err(e) => panic!("cannot read {}: {e}", path.display()),
        })
        .collect()
}

/// Draws whatever `draw` draws into a `w`x`h` test terminal and returns the
/// cells. A view's own tests use this with that view's `draw`; whole-screen
/// tests use [`AppHarness::render_buffer`].
pub fn render_with(w: u16, h: u16, draw: impl FnOnce(&mut Frame<'_>)) -> Buffer {
    let Ok(mut terminal) = Terminal::new(TestBackend::new(w, h));
    let Ok(_) = terminal.draw(draw);
    terminal.backend().buffer().clone()
}

/// The screen as text: one line per buffer row, each ending in `\n`, with
/// trailing spaces kept so a snapshot shows the size too.
pub fn buffer_to_string(buf: &Buffer) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

/// The rows of the screen as separate strings, for tests that look at one
/// row; [`buffer_to_string`] without the newlines.
pub fn buffer_lines(buf: &Buffer) -> Vec<String> {
    buffer_to_string(buf).lines().map(str::to_string).collect()
}

/// The screen as text, then a `--- styles (x,y fg bg modifiers) ---` line,
/// then one `{x},{y} {fg:?} {bg:?} {modifiers:?}` line wherever the style
/// changes along a row (and at the start of each row). A snapshot of this
/// shows both the layout and the colours.
pub fn styled(buf: &Buffer) -> String {
    let mut out = buffer_to_string(buf);
    out.push_str("--- styles (x,y fg bg modifiers) ---\n");
    let area = buf.area;
    for y in area.top()..area.bottom() {
        let mut last = None;
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            let style = (cell.fg, cell.bg, cell.modifier);
            if last != Some(style) {
                out.push_str(&format!(
                    "{x},{y} {:?} {:?} {:?}\n",
                    cell.fg, cell.bg, cell.modifier
                ));
                last = Some(style);
            }
        }
    }
    out
}

/// Reads keys written the way the help shows them, separated by spaces:
/// `"ctrl-n esc shift-tab f8 enter j"`. Modifiers are `ctrl-`, `alt-` and
/// `shift-`; named keys are `esc`, `enter`, `tab`, `backspace`, `delete`,
/// `insert`, `space`, `up`/`down`/`left`/`right`, `home`/`end`,
/// `pageup`/`pagedown` and `f1`…`f12`. `shift-tab` is the `BackTab` that
/// terminals send, and a capital letter carries Shift.
///
/// # Panics
/// On a key it cannot read, naming that key.
pub fn parse_keys(spec: &str) -> Vec<KeyEvent> {
    spec.split_whitespace()
        .map(|token| match parse_key(token) {
            Ok(key) => key,
            Err(e) => panic!("parse_keys: cannot read key `{token}` in {spec:?}: {e}"),
        })
        .collect()
}

/// One key of [`parse_keys`].
fn parse_key(token: &str) -> Result<KeyEvent, String> {
    // A lone capital is Shift plus the letter; the config parser would
    // lower-case it first.
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_uppercase()
    {
        return Ok(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT));
    }
    let key = parse_key_event(token)?;
    Ok(match key.code {
        KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
            KeyEvent::new(KeyCode::BackTab, key.modifiers)
        }
        _ => key,
    })
}

#[cfg(test)]
#[path = "testing_tests.rs"]
mod testing_tests;
