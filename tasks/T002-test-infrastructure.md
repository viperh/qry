# T002 — Test infrastructure: harness, snapshots, conventions

**Phase:** 0 — Foundation · **Depends on:** T001 · **Blocks:** T003, T004, T005, T101, every UI task

## Description

qry's UI tests today are hand-rolled per component:

- Each test module rebuilds its own `Terminal::new(TestBackend::new(w, h))` and joins `buffer[(x, y)].symbol()` into strings. Look for `render`, `row_text` and `shown` in the `home.rs` tests, `render_buffer` and `lines` in `results.rs`, and `render` and `lines` in `form.rs`.
- Assertions are substring checks such as `assert!(text.contains("Exporting"))`. They cannot catch layout regressions, and they say nothing about colour.
- The one colour assertion, in `results.rs` (`assert_eq!(buffer[(x, 3)].bg, Color::Green)`), will break as soon as the theme lands (T201).
- There is no fake database. The worker tests in `crates/qry/src/db.rs` run against a real in-memory SQLite. That is fine for SQLite, but they cannot simulate a slow query, a lost connection or a driver error.

This task builds the shared test toolkit every later task uses. It mirrors sverb's `crates/sverb-tui/src/testing.rs` (`AppHarness`, `buffer_to_string`, the `styled` snapshots in `shell_tests.rs`, `t08_every_view_renders_at_every_size`) and its `test-util` feature pattern for fakes (`sverb-conn` `mock`).

### 1. `crates/qry/src/testing.rs`

The harness module is gated so tests and other crates' dev-dependencies can use it, while it never ships in a release build:

```rust
// crates/qry/src/lib.rs (or main.rs, if there is no lib target yet; see note)
#[cfg(any(test, feature = "test-util"))]
pub mod testing;
```

```toml
# crates/qry/Cargo.toml
[features]
test-util = ["qry-core/test-util"]

[dev-dependencies]
qry = { path = ".", features = ["test-util"] }   # self dev-dep, as sverb-conn does
insta = { workspace = true }
proptest = { workspace = true }
tempfile = { workspace = true }
tokio = { workspace = true, features = ["test-util"] }
```

**Note.** `qry` is a binary-only crate today. Integration tests under `crates/qry/tests/` need a library target, so split `crates/qry/src/main.rs` into a thin `main.rs` plus `lib.rs` that exports `app`, `components`, `config`, `keymap`, `testing`, and so on. `main.rs` calls `qry::run(cli)`.

**`AppHarness` API** (the docs on the module carry this example):

```rust
//! let mut h = AppHarness::new(Config::embedded());
//! h.keys("ctrl-n");                       // chord list in display syntax
//! insta::assert_snapshot!(h.render(80, 24));
//! h.keys("esc");
//! assert!(h.take_effects().is_empty());
pub struct AppHarness { /* app, effects, clock, size, theme env */ }

impl AppHarness {
    pub fn new(config: Config) -> Self;
    pub fn with_database(self, db: FakeDatabase) -> Self;     // wired into the worker stand-in
    pub fn with_theme_env(self, env: ThemeEnv) -> Self;       // NO_COLOR / COLORTERM, used once T201 lands
    pub fn keys(&mut self, spec: &str) -> &mut Self;          // "ctrl-p j j enter", "shift-tab", "f8"
    pub fn key(&mut self, key: KeyEvent) -> &mut Self;
    pub fn type_str(&mut self, text: &str) -> &mut Self;      // one Char key per char
    pub fn send(&mut self, action: Action) -> &mut Self;      // inject a worker answer
    pub fn resize(&mut self, w: u16, h: u16) -> &mut Self;
    pub fn advance(&mut self, ms: u64) -> &mut Self;          // virtual clock: Tick events, due timers
    pub fn take_effects(&mut self) -> Vec<Effect>;            // or Vec<Action> before T101
    pub fn render(&mut self, w: u16, h: u16) -> String;       // buffer_to_string
    pub fn render_buffer(&mut self, w: u16, h: u16) -> Buffer;
}

pub fn buffer_to_string(buf: &Buffer) -> String;   // one line per row, trailing spaces kept
pub fn styled(buf: &Buffer) -> String;             // text, then "--- styles (x,y fg bg modifiers) ---" runs
pub fn parse_keys(spec: &str) -> Vec<KeyEvent>;    // shared with T102's chord display syntax
pub const SIZES: &[(u16, u16)] = &[(0,0),(1,1),(10,3),(80,24),(120,40),(160,48),(300,100)];
```

**Before T101 lands,** `AppHarness` wraps the current `Home` component:

- It creates an unbounded action channel and calls `register_action_handler`, `register_config_handler(Config::embedded())` and `init`.
- `keys` routes through `App::handle_key_event`-equivalent logic (the global keymap first, then `Home::handle_key_event`), so `<Ctrl-n>` really opens the modal.
- Every action it drains is fed back through `Home::update`, the same loop `App::handle_actions` runs. The actions that `App` itself would turn into I/O (`Execute`, `Connect`, `Export`, `SaveConnections`, `Import`, `WriteConnections`, `ForgetSecret`, `ListTables`) are not dispatched. They are recorded as effects instead.

**After T101,** the harness drives `App::handle(UiEvent) -> Vec<Effect>` directly. The public API above must not change, so tests written now keep working.

**Virtual clock.**

- `advance(ms)` delivers `Tick`s at the configured tick rate.
- Timers scheduled through `Effect::ScheduleTimer` (once T101 exists) fire in time order.
- No test may sleep in real time.

**Styled dump format.** Copy sverb's: the text grid, then one line per change of style run, in the form `{x},{y} {fg:?} {bg:?} {modifiers:?}`. A snapshot then shows both the layout and the theme.

### 2. Snapshots with insta

- Add `insta = "1.x"` to `[workspace.dependencies]` under `# --- dev ---`. Snapshots live next to their tests in `snapshots/`.
- `*.snap.new` is already in `.gitignore` (T001).
- Snapshot names include the terminal size: `home_empty_80x24`, `connection_form_new_80x24`, `help_120x40`.
- Use `insta::with_settings!({ prepend_module_to_snapshot => false }, …)` only where names would otherwise collide.
- Document `cargo insta review` / `cargo insta accept` in CONTRIBUTING (T401). CI runs with `INSTA_UPDATE=no` (insta's default when `CI` is set), so a missing or changed snapshot fails the build.

### 3. Fakes in `qry-core`

```toml
# crates/qry-core/Cargo.toml
[features]
test-util = []
```

```rust
#[cfg(any(test, feature = "test-util"))]
pub mod testing;   // crates/qry-core/src/testing.rs

/// A `Database` that answers from a script, for tests that must not touch a server.
#[derive(Debug, Clone, Default)]
pub struct FakeDatabase { /* kind, label, script: Arc<Mutex<VecDeque<Reply>>>, calls */ }

pub enum Reply { Rows(QueryResult), Affected(u64), Error(String), Delay(Duration, Box<Reply>), Hang }

impl FakeDatabase {
    pub fn new(kind: DatabaseType) -> Self;
    pub fn reply(self, r: Reply) -> Self;        // queue the next answer
    pub fn tables(self, names: &[&str]) -> Self;
    pub fn calls(&self) -> Vec<String>;          // every SQL text it received, in order
}
```

`Driver` (`crates/qry-core/src/lib.rs:127`) gets a `#[cfg(any(test, feature = "test-util"))] pub fn with_database(db: Box<dyn Database>) -> Self`, so the worker can be tested with a fake.

`QueryResult` builders for readable fixtures:

```rust
pub fn result(columns: &[&str], rows: &[&[Option<&str>]]) -> QueryResult;
pub fn grid(rows: usize, cols: usize) -> QueryResult;   // "r{i}c{j}" cells, like results.rs's helper
```

### 4. Conventions, written into CONTRIBUTING (T401 polishes the wording)

- **Unit tests** live in sibling files (T001). Every test file starts with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.
- **Integration tests** live in `crates/<crate>/tests/*.rs`, with shared helpers in `tests/common/mod.rs` that carry `#![allow(dead_code, unreachable_pub)]`.
- **Paths:** use `env!("CARGO_MANIFEST_DIR")` for fixtures and `env!("CARGO_TARGET_TMPDIR")` or `tempfile` for scratch space. Tests never write into the repo or the user's real config or data directories. Set `QRY_CONFIG` and `QRY_DATA` to a temporary directory.
- **Keychain:** tests set `QRY_KEYRING=off`. Add that switch to `crates/qry/src/secrets.rs` so the keychain backend is replaced by an in-memory map. No test may touch the developer's real keychain.
- **Fixtures:** shared fixtures go in `tests/fixtures/` at the repo root (`dbeaver/`, `sqldeveloper/`, `qry-backup/`, `sql/`). Existing inline JSON samples in `import.rs` tests move there.
- **Naming:** test names are descriptive snake_case sentences (`a_failed_export_keeps_the_modal_and_the_path`). The current style already does this, so keep it.

### 5. Port the existing hand-rolled helpers

- Replace the local `render`, `shown`, `row_text` and `lines` helpers in the `home`, `results`, `form`, `help` and `conntree` tests with `testing::buffer_to_string` and the harness.
- Convert substring assertions on whole screens into snapshots wherever the whole screen is the point. Keep targeted `contains` checks where only one fact matters.
- Replace the `Color::Green` assertion in the `results` tests with a styled snapshot. That snapshot will be re-accepted in T201.

### Out of scope

- Real-server integration tests (T004).
- Property tests and fuzzing (T005).

## Acceptance criteria

- [ ] `crates/qry` has a `lib.rs` exporting the app modules. `main.rs` is under ~30 lines and only parses the CLI, installs the hooks and calls into the library.
- [ ] `crates/qry/src/testing.rs` exists behind `#[cfg(any(test, feature = "test-util"))]` and provides every item in the API listing above, each with a doc comment.
- [ ] `crates/qry-core/src/testing.rs` provides `FakeDatabase`, `Reply`, `result()` and `grid()` behind the `test-util` feature.
- [ ] `cargo build -p qry --release` does not compile `testing.rs`. Check that `cargo tree -e features -p qry` in release has no `test-util`, and that `nm` or `strings` on the binary finds no `AppHarness`.
- [ ] `QRY_KEYRING=off` swaps the keychain for an in-memory store. With it set, `cargo test --workspace` never prompts or writes to the OS keychain.
- [ ] The existing tests in `home`, `results`, `form`, `help`, `conntree`, `editor` and the forms use the shared helpers. No test module defines its own `TestBackend` rendering helper (`grep -rn "TestBackend::new" crates/qry/src` only matches `testing.rs`).
- [ ] At least the snapshots listed under Tests exist and are committed.
- [ ] No test sleeps in real time (`grep -rn "thread::sleep\|tokio::time::sleep" crates/*/src crates/*/tests` matches nothing outside `start_paused` tests and `FakeDatabase`'s `Delay`, which uses `tokio::time` under a paused clock).
- [ ] CONTRIBUTING.md has a "Writing tests" section that documents the conventions in section 4 and the `cargo insta review` workflow.

## Tests

Harness self-tests in `crates/qry/src/testing_tests.rs`:

- **`parse_keys_reads_chords_and_named_keys`**:
  - `"ctrl-n esc shift-tab f8 enter j"` gives the expected `KeyEvent`s.
  - An unknown name such as `"ctrl-foo"` panics with a message naming the bad token.
- **`buffer_to_string_keeps_trailing_spaces_and_row_count`**: a 5x3 buffer with `"ab"` in row 0 gives `"ab   \n     \n     \n"`.
- **`styled_lists_one_run_per_style_change`**: a buffer with two coloured spans gives exactly one style line per run boundary.
- **`keys_open_the_new_connection_modal`**: `h.keys("ctrl-n")` and then `render(80, 24)` contains `New Connection`. The harness really routes the global keymap.
- **`io_actions_become_effects_not_dispatches`**: typing a path in the export modal and pressing `enter` leaves an `Export` effect from `take_effects()`. Nothing is written to disk; check with a temp dir that stays empty.
- **`advance_delivers_ticks_without_real_time`**: run under `#[tokio::test(start_paused = true)]`. `advance(1000)` at a 4 Hz tick rate delivers 4 `Tick`s, and the wall clock moves under 50 ms.
- **`every_view_renders_at_every_size`**: for each `(w, h)` in `SIZES`, for the empty main screen, each modal mode (`AddConnModal`, `ExpoModal`, `PasswordModal`, `ImportModal`, `ConnExportModal`) and the help overlay, `render(w, h)` does not panic and returns exactly `h` lines of `w` cells each. UI tasks extend this list with each new view.

`FakeDatabase` tests in `crates/qry-core/src/testing_tests.rs`:

- **`fake_answers_in_script_order_and_records_calls`**
- **`fake_delay_respects_a_paused_clock`**: `start_paused`, then `advance`.
- **`fake_error_surfaces_as_query_error`**

Worker tests in `crates/qry/src/db_tests.rs` (moved from `db.rs` in T001), using `FakeDatabase`:

- **`a_driver_error_comes_back_as_a_status_error`**
- **`commands_run_in_order_even_when_the_first_is_slow`**

Snapshots, under `crates/qry/src/components/snapshots/`:

- `home_empty_80x24`, `home_empty_160x48`
- `connection_form_new_80x24`
- `export_form_80x24`
- `password_prompt_80x24`
- `help_80x24`
- `results_grid_80x24` (styled), built from `grid(30, 8)`

CI checks:

- `cargo test --workspace --all-features --locked` passes with `CI=true`, so missing snapshots fail.
- `cargo test -p qry --no-default-features --locked` passes.
- `cargo build -p qry --release --locked` passes without `test-util`.
