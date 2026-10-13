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
//! Until the reducer lands (T101), [`AppHarness`] wraps the [`Home`]
//! component and plays the part of `App`: keys go through the global keymap
//! first and then to `Home`, every action is fed back through
//! `Home::update`, and the actions `App` would turn into I/O are recorded as
//! [effects](Effect) instead of being carried out.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub use qry_core::testing::{Reply, grid, result};
use qry_core::{QueryResult, testing::FakeDatabase};
use ratatui::{Frame, Terminal, backend::TestBackend, buffer::Buffer};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub use crate::theme::ThemeEnv;
use crate::{
    action::Action,
    app::Mode,
    components::{Component, home::Home},
    config::{Config, parse_key_event},
    db,
    tui::Event,
};

/// Something the app asked the outside world to do. Before T101 this is the
/// [`Action`] itself (`Execute`, `Connect`, `Export`, …); T101 replaces the
/// alias with a real `Effect` enum, and the harness API stays the same.
pub type Effect = Action;

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

/// An answer the stand-in worker will deliver once the virtual clock reaches
/// `due`, in milliseconds since the harness was made.
#[derive(Debug)]
struct Pending {
    due: u64,
    actions: Vec<Action>,
}

/// Drives the main screen with scripted keys and actions on a virtual clock.
pub struct AppHarness {
    home: Home,
    config: Config,
    mode: Mode,
    action_tx: UnboundedSender<Action>,
    action_rx: UnboundedReceiver<Action>,
    /// Keys pressed since the last tick, for multi-key bindings, as in `App`.
    chord: Vec<KeyEvent>,
    effects: Vec<Effect>,
    /// Virtual time, in milliseconds since the harness was made.
    now: u64,
    /// Milliseconds between ticks, from the tick rate.
    tick_every: u64,
    next_tick: u64,
    ticks: u64,
    size: (u16, u16),
    theme_env: ThemeEnv,
    database: Option<FakeDatabase>,
    /// Worker answers not yet due, in the order they were scheduled.
    pending: Vec<Pending>,
    /// When the stand-in worker finishes what it already has: it runs one
    /// command at a time, like the real one.
    worker_free_at: u64,
}

/// Shows the mode, the clock and what is waiting; `Home` has no `Debug`.
impl std::fmt::Debug for AppHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHarness")
            .field("mode", &self.mode)
            .field("now_ms", &self.now)
            .field("ticks", &self.ticks)
            .field("size", &self.size)
            .field("effects", &self.effects)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

/// The tick rate the binary uses by default (`--tick-rate 4`).
const DEFAULT_TICK_RATE: f64 = 4.0;

impl AppHarness {
    /// A main screen set up the way `App::run` sets it up: an action channel,
    /// this config, and `init` at 80x24. Nothing is loaded from disk and no
    /// connection is opened.
    pub fn new(config: Config) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let mut home = Home::new();
        let size = (80, 24);
        let setup = home
            .register_action_handler(action_tx.clone())
            .and_then(|()| home.register_config_handler(config.clone()))
            .and_then(|()| home.init(ratatui::layout::Size::new(size.0, size.1)));
        if let Err(e) = setup {
            panic!("AppHarness::new: setting up the main screen failed: {e:?}");
        }
        let tick_every = tick_interval(DEFAULT_TICK_RATE);
        Self {
            home,
            config,
            mode: Mode::Home,
            action_tx,
            action_rx,
            chord: Vec::new(),
            effects: Vec::new(),
            now: 0,
            tick_every,
            next_tick: tick_every,
            ticks: 0,
            size,
            theme_env: ThemeEnv::default(),
            database: None,
            pending: Vec::new(),
            worker_free_at: 0,
        }
    }

    /// Answers `Connect`, `Execute` and `ListTables` from `db`, on the
    /// virtual clock: a [`Reply::Delay`] arrives once [`advance`] passes it,
    /// and a [`Reply::Hang`] never does. The requests are still recorded as
    /// effects. Without a database they are only recorded.
    ///
    /// [`advance`]: Self::advance
    #[must_use]
    pub fn with_database(mut self, db: FakeDatabase) -> Self {
        self.database = Some(db);
        self
    }

    /// Uses this terminal environment for the theme. Stored only, until the
    /// theme system lands (T201).
    #[must_use]
    pub fn with_theme_env(mut self, env: ThemeEnv) -> Self {
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

    /// The main screen, for this crate's tests that check state the screen
    /// does not show.
    #[cfg(test)]
    pub(crate) fn home(&self) -> &Home {
        &self.home
    }

    /// The main screen, for this crate's tests that set up state no key
    /// reaches directly, such as which form field has focus.
    #[cfg(test)]
    pub(crate) fn home_mut(&mut self) -> &mut Home {
        &mut self.home
    }

    /// The terminal environment set with [`with_theme_env`](Self::with_theme_env).
    pub fn theme_env(&self) -> &ThemeEnv {
        &self.theme_env
    }

    /// The mode `App` would be in: which modal, if any, has the keys.
    pub fn mode(&self) -> Mode {
        self.mode
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

    /// Presses one key the way `App` handles it: the global keymap for the
    /// current mode first, then `Home`, then every action that follows.
    pub fn key(&mut self, key: KeyEvent) -> &mut Self {
        if let Some(action) = self.global_binding(key) {
            self.queue(action);
        }
        match self.home.handle_events(Some(Event::Key(key))) {
            Ok(Some(action)) => self.queue(action),
            Ok(None) => {}
            Err(e) => panic!("AppHarness::key: Home failed on {key:?}: {e:?}"),
        }
        self.drain();
        self
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

    /// Delivers an action as if the worker (or `App`) had sent it, such as a
    /// `QueryDone` or a `ConnectFailed`, and everything that follows from it.
    pub fn send(&mut self, action: Action) -> &mut Self {
        self.queue(action);
        self.drain();
        self
    }

    /// Resizes the terminal. The next [`render`](Self::render) still draws at
    /// the size it is given.
    pub fn resize(&mut self, w: u16, h: u16) -> &mut Self {
        self.size = (w, h);
        self.send(Action::Resize(w, h))
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
                    for action in pending.actions {
                        self.queue(action);
                    }
                    self.drain();
                }
                _ if self.next_tick <= target => {
                    self.now = self.next_tick;
                    self.next_tick += self.tick_every;
                    self.ticks += 1;
                    self.send(Action::Tick);
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

    /// Draws the main screen into a `w`x`h` terminal and returns it as text;
    /// see [`buffer_to_string`].
    pub fn render(&mut self, w: u16, h: u16) -> String {
        buffer_to_string(&self.render_buffer(w, h))
    }

    /// Draws the main screen into a `w`x`h` terminal and returns the cells,
    /// with their styles; see [`styled`].
    pub fn render_buffer(&mut self, w: u16, h: u16) -> Buffer {
        let home = &mut self.home;
        render_with(w, h, |frame| {
            if let Err(e) = home.draw(frame, frame.area()) {
                panic!("AppHarness::render: drawing failed: {e:?}");
            }
        })
    }

    /// The action the global keymap binds `key` to in the current mode,
    /// counting the keys pressed since the last tick for multi-key bindings.
    fn global_binding(&mut self, key: KeyEvent) -> Option<Action> {
        let keymap = self.config.keybindings.0.get(&self.mode)?;
        if let Some(action) = keymap.get(&vec![key]) {
            return Some(action.clone());
        }
        self.chord.push(key);
        keymap.get(&self.chord).cloned()
    }

    fn queue(&self, action: Action) {
        // The harness holds the receiver, so the channel cannot be closed.
        let _ = self.action_tx.send(action);
    }

    /// Plays `App::handle_actions`: handles what `App` itself would, records
    /// I/O as effects, and feeds every action through `Home::update` until
    /// nothing is left.
    fn drain(&mut self) {
        while let Ok(action) = self.action_rx.try_recv() {
            match &action {
                Action::Tick => self.chord.clear(),
                Action::ChangeMode(mode) => self.mode = *mode,
                Action::Execute(sql) if !sql.trim().is_empty() => {
                    self.effects.push(action.clone());
                    self.answer_query(sql);
                }
                Action::Connect(record) => {
                    self.effects.push(action.clone());
                    let answer =
                        (self.database.as_ref()).map(|db| db::connected_answer(record, db.name()));
                    if let Some(answer) = answer {
                        self.schedule(0, answer);
                    }
                }
                Action::ListTables => {
                    self.effects.push(action.clone());
                    let answer = (self.database.as_ref())
                        .map(|db| db::tables_answer(Ok::<_, String>(db.table_names())));
                    if let Some(answer) = answer {
                        self.schedule(0, answer);
                    }
                }
                Action::Export(_)
                | Action::PasswordEntered
                | Action::SaveConnections(_)
                | Action::Import { .. }
                | Action::WriteConnections { .. }
                | Action::ForgetSecret(_) => self.effects.push(action.clone()),
                _ => {}
            }
            match self.home.update(action.clone()) {
                Ok(Some(next)) => self.queue(next),
                Ok(None) => {}
                Err(e) => panic!("AppHarness: Home::update({action}) failed: {e:?}"),
            }
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

    /// Delivers `actions` once the stand-in worker has finished what it
    /// already had and then spent `after` ms on this. With nothing to wait
    /// for they arrive straight away, as part of the current drain.
    fn schedule(&mut self, after: u64, actions: Vec<Action>) {
        let start = self.worker_free_at.max(self.now);
        let due = start.saturating_add(after);
        self.worker_free_at = due;
        if due <= self.now {
            for action in actions {
                self.queue(action);
            }
        } else if due != u64::MAX {
            self.pending.push(Pending { due, actions });
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

/// Draws whatever `draw` draws into a `w`x`h` test terminal and returns the
/// cells. Component tests use this with the component's own `draw` or
/// `render`; whole-screen tests use [`AppHarness::render_buffer`].
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
