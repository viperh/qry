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
pub use qry_core::testing::{FakeSchema, Reply, grid, result};
use qry_core::{QueryResult, testing::FakeDatabase};
use ratatui::{Frame, Terminal, backend::TestBackend, buffer::Buffer};

pub use crate::{app::Effect, theme::ThemeEnv};
use crate::{
    app::{App, DbAnswer, FileAnswer, SessionId, State, Timer, UiEvent},
    config::Config,
    connections::StoredConnection,
    db::{self, DbCommand},
    keymap::{KeyContext, parse_key_event},
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

/// Events to deliver once the virtual clock reaches `due`, in milliseconds
/// since the harness was made: the stand-in worker's answers, and timers the
/// app asked for.
#[derive(Debug)]
struct Pending {
    due: u64,
    events: Vec<UiEvent>,
}

/// One stand-in worker per session: when it finishes what it already has.
/// Like the real ones, each runs one command at a time, and sessions do not
/// wait for each other.
type Busy = std::collections::HashMap<SessionId, u64>;

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
    /// Worker answers and timers not yet due, in the order they were
    /// scheduled.
    pending: Vec<Pending>,
    /// When each session's stand-in worker finishes what it already has.
    worker_free_at: Busy,
    /// The files the Export dialog's preview finds already there.
    existing: Vec<std::path::PathBuf>,
}

/// The home directory `~` stands for in the harness's export previews.
pub const TEST_HOME: &str = "/home/test";

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
            worker_free_at: Busy::new(),
            existing: Vec::new(),
        }
    }

    /// Answers `Connect`, `OpenSession`, `Query` and `LoadSchema` from `db`
    /// (schema loads from its [`FakeSchema`]),
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

    /// Has the Export dialog's preview find `path` (`~` being
    /// [`TEST_HOME`]) already there.
    pub fn with_existing_file(&mut self, path: impl Into<std::path::PathBuf>) -> &mut Self {
        self.existing.push(path.into());
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

    /// The contexts the next key is looked up in, innermost first.
    pub fn contexts(&self) -> Vec<KeyContext> {
        self.app.state().contexts()
    }

    /// The title of the open modal, if one is open.
    pub fn dialog_title(&self) -> Option<&'static str> {
        self.app
            .state()
            .dialog()
            .map(|dialog| dialog.form().title())
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

    /// Delivers an answer from the active session's database worker.
    pub fn db(&mut self, answer: DbAnswer) -> &mut Self {
        let id = self.app.state().sessions.active;
        self.send(UiEvent::Db(id, answer))
    }

    /// Delivers an answer from session `id`'s worker, open or not.
    pub fn db_for(&mut self, id: SessionId, answer: DbAnswer) -> &mut Self {
        self.send(UiEvent::Db(id, answer))
    }

    /// The active session, as the top bar shows it.
    pub fn active_session(&self) -> SessionId {
        self.app.state().sessions.active
    }

    /// The open sessions, in tab order.
    pub fn sessions(&self) -> Vec<SessionId> {
        self.app.state().sessions.order.clone()
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

    /// Moves the virtual clock on by `ms`, delivering every `Tick`, every
    /// worker answer and every `Effect::ScheduleTimer` timer that falls due,
    /// in time order. Nothing waits in real time.
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
                    self.inbox.extend(pending.events);
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

    /// Returns and clears the effects recorded so far. A waiting dialog's
    /// spinner timers, the tree's and the toasts' lifetimes are delivered on the
    /// virtual clock but not recorded.
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
                    Effect::Db(id, DbCommand::Connect(record))
                    | Effect::OpenSession(id, record) => {
                        self.answer_connect(*id, record);
                    }
                    Effect::Db(id, DbCommand::Query(sql)) => {
                        let max_rows = self.app.state().config.max_rows;
                        self.answer_query(*id, sql, max_rows);
                    }
                    Effect::Db(id, DbCommand::QueryAll(sql)) => self.answer_query(*id, sql, 0),
                    Effect::CancelQuery(id) => self.cancel_query(*id),
                    Effect::Db(id, DbCommand::LoadSchema { path, generation }) => {
                        let answer = self.database.as_ref().map(|db| {
                            let (outcome, delay) = db.schema_answer(path);
                            (db::schema_answer(path.clone(), *generation, outcome), delay)
                        });
                        match answer {
                            // The worker stays busy for good, as the real
                            // one would.
                            Some((_, u64::MAX)) => {
                                self.worker_free_at.insert(*id, u64::MAX);
                            }
                            Some((answer, delay)) => self.schedule(*id, delay, vec![answer]),
                            None => {}
                        }
                    }
                    // The preview's look at the disk: answered at once from
                    // the files the test says exist, and not recorded.
                    Effect::ProbeExportPath { input, etype } => {
                        let home = std::path::Path::new(TEST_HOME);
                        let path = qry_core::exporter::resolve_path(input, *etype, home);
                        let exists = self.existing.contains(&path);
                        self.inbox.push_back(UiEvent::File(
                            crate::app::event::FileAnswer::ExportProbed(
                                crate::app::event::ExportProbe {
                                    input: input.clone(),
                                    etype: *etype,
                                    resolved: path.display().to_string(),
                                    exists,
                                    is_dir: false,
                                },
                            ),
                        ));
                        continue;
                    }
                    // The fake connects in 42 ms and reports its version.
                    Effect::TestConnection { test, .. } => {
                        if let Some(db) = &self.database {
                            self.inbox.push_back(UiEvent::ConnectionTested {
                                test: *test,
                                outcome: Ok(crate::app::event::Tested {
                                    millis: 42,
                                    version: db.server_version_text(),
                                }),
                            });
                        }
                    }
                    Effect::ScheduleTimer { timer, after } => {
                        let after = u64::try_from(after.as_millis()).unwrap_or(u64::MAX);
                        self.pending.push(Pending {
                            due: self.now.saturating_add(after),
                            events: vec![UiEvent::Timer(*timer)],
                        });
                        // A spinner's frames and a toast's lifetime are
                        // pacing, not something a test asks for: delivered,
                        // but not recorded.
                        if matches!(timer, Timer::Spinner(_) | Timer::Toast(_) | Timer::Tree(_)) {
                            continue;
                        }
                    }
                    // A closed session's worker is dropped with whatever it
                    // still had to do.
                    Effect::CloseSession(id) => {
                        self.worker_free_at.remove(id);
                        let id = *id;
                        for pending in &mut self.pending {
                            pending
                                .events
                                .retain(|event| !matches!(event, UiEvent::Db(s, _) if *s == id));
                        }
                    }
                    _ => {}
                }
                self.effects.push(effect);
            }
        }
    }

    /// Answers a connection from the fake database, if there is one.
    fn answer_connect(&mut self, id: SessionId, record: &StoredConnection) {
        let answer = (self.database.as_ref()).map(|db| db::connected_answer(record, db.name()));
        if let Some(answer) = answer {
            self.schedule(id, 0, answer);
        }
    }

    /// Stops session `id`'s statement in progress, if any: its answer is
    /// dropped and the failure a cancelled statement gives arrives now.
    fn cancel_query(&mut self, id: SessionId) {
        let is_answer = |event: &UiEvent| matches!(event, UiEvent::Db(s, DbAnswer::QueryDone(_) | DbAnswer::QueryFailed { .. }) if *s == id);
        let before = self.pending.len();
        self.pending
            .retain(|pending| !pending.events.iter().any(is_answer));
        let hung = self.worker_free_at.get(&id) == Some(&u64::MAX);
        if self.pending.len() == before && !hung {
            return;
        }
        self.worker_free_at.insert(id, self.now);
        let answer =
            db::query_answer::<&str>(Err(qry_core::testing::CANCELLED), Duration::ZERO, None);
        self.schedule(id, 0, answer);
    }

    /// Answers a query from the fake database, if there is one, keeping
    /// at most `max_rows` rows.
    fn answer_query(&mut self, id: SessionId, sql: &str, max_rows: usize) {
        let Some(db) = &self.database else {
            return;
        };
        let mut reply = db.next_reply(sql);
        let mut after = 0;
        let outcome = loop {
            match reply {
                Reply::Rows(result) => break Ok(qry_core::shared::truncate(result, max_rows)),
                Reply::Affected(count) => {
                    break Ok(QueryResult::affected(count));
                }
                Reply::Error(message) => break Err(message),
                Reply::Delay(delay, inner) => {
                    after += u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
                    reply = *inner;
                }
                // The worker stays busy for good, as the real one would.
                Reply::Hang => {
                    self.worker_free_at.insert(id, u64::MAX);
                    return;
                }
            }
        };
        let answer = db::query_answer(outcome, Duration::from_millis(after), None);
        self.schedule(id, after, answer);
    }

    /// Delivers `answers` from session `id` once its stand-in worker has
    /// finished what it already had and then spent `after` ms on this. With
    /// nothing to wait for they arrive straight away, after the event being
    /// handled.
    fn schedule(&mut self, id: SessionId, after: u64, answers: Vec<DbAnswer>) {
        let free = self.worker_free_at.get(&id).copied().unwrap_or(0);
        let start = free.max(self.now);
        let due = start.saturating_add(after);
        self.worker_free_at.insert(id, due);
        let events = answers.into_iter().map(|answer| UiEvent::Db(id, answer));
        if due <= self.now {
            self.inbox.extend(events);
        } else if due != u64::MAX {
            self.pending.push(Pending {
                due,
                events: events.collect(),
            });
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
/// `"ctrl-n esc shift-tab f8 enter j"`, in the syntax of
/// [`parse_key_sequence`](crate::keymap::parse_key_sequence): `Enter`, `←`
/// and `PgDn` work as well as `enter`, `left` and `pagedown`. `shift-tab`
/// is the `BackTab` that terminals send, and a capital letter carries
/// Shift, as terminals report it.
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
    // A lone capital is Shift plus the letter, as a terminal sends it; the
    // keymap's chord drops the Shift.
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_uppercase()
    {
        return Ok(KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT));
    }
    parse_key_event(token)
}

#[cfg(test)]
#[path = "testing_tests.rs"]
mod testing_tests;
