# T101 — App state and reducer architecture

**Phase:** 1 — Architecture · **Depends on:** T002 · **Blocks:** T102, T103, T202, and everything in Phase 2 and 3 that draws or handles keys

## Description

### Current state

qry still follows the ratatui component template. State and side effects are tangled together:

- **Modes:** `crates/qry/src/app.rs` holds `App` with a `Mode` enum (`Home`, `AddConnModal`, `ExpoModal`, `PasswordModal`, `ImportModal`, `ConnExportModal`).
  - `handle_key_event` looks the key up in `config.keybindings` for the current mode.
  - `handle_actions` pumps an `mpsc` channel of `Action`s. It also starts blocking I/O directly (`spawn_blocking` for `connections::save`, `import::from_file`, `connections::export`, `secrets::delete`) and sends `DbCommand`s to the single worker.
- **Components:** every component implements `Component` (`crates/qry/src/components.rs`) and owns its own mutable focus flag (`set_focus`). Each holds a cloned `Config` and an `UnboundedSender<Action>`, and sends actions from inside `update`/`handle_key_event`.
- **Home:** `crates/qry/src/components/home.rs` is the de-facto controller. It holds:
  - the `Pane` focus ring (`Tree → Editor → Results`)
  - the open modal (`Option<Box<dyn Form>>`) and the `Awaiting` guard, so a late worker answer cannot act on a different modal
  - `asked` (whose password the worker wants) and `editing` (the record to open in the connection form)
  - the help visibility flag
- **Database worker:** `crates/qry/src/db.rs` spawns one `Worker` with one `qry_core::Driver`. It answers with `Action`s on the same channel.

As a result, tests have to build a `Home`, register a channel, poke keys and drain `rx`. Ordering bugs between `App::handle_actions` and child `update`s are only visible at runtime. Nothing is deterministic or replayable.

### Target design

The model is sverb's pure reducer (`crates/sverb-tui/src/app/`, driven by `AppHarness` in `crates/sverb-tui/src/testing.rs`). All decisions happen in a pure function, and a thin runtime performs the I/O.

```
crates/qry/src/app/
  mod.rs        App { state: State } and App::handle
  state.rs      State, Focus, Overlay, Awaiting, SessionState, ...
  event.rs      UiEvent
  effects.rs    Effect
  reducer/      one file per concern: keys.rs, db.rs, dialogs.rs, tree.rs, ...
crates/qry/src/runtime.rs   owns Tui, tokio tasks, workers; executes Effects
crates/qry/src/views/       fn draw(state: &State, theme: &Theme, frame, area)
```

```rust
pub enum UiEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick(Instant),              // the runtime supplies now; the reducer never reads a clock
    Timer(TimerId),
    Db(SessionId, DbAnswer),    // Connected, ConnectFailed, QueryDone, TablesLoaded, Exported, NeedPassword, ...
    File(FileAnswer),           // ConnectionsLoaded, Imported, ImportFailed, ConnectionsExported, SaveFailed, ...
    Paste(String),
}

pub enum Effect {
    Db(SessionId, DbCommand),
    OpenSession(SessionId, Box<StoredConnection>),   // T103
    CloseSession(SessionId),
    SaveConnections(Vec<StoredConnection>),
    LoadConnections,
    Import { path: String, format: Format },
    ExportConnections { path: String, connections: Vec<StoredConnection> },
    ForgetSecret(String),
    ScheduleTimer { id: TimerId, after: Duration },
    CancelTimer(TimerId),
    Suspend,
    Quit,
}

impl App {
    pub fn handle(&mut self, event: UiEvent) -> Vec<Effect>;
}
```

- **State:** `State` holds everything the views need:
  - focus (`Focus::{Tree, Editor, Results, Inspector, Jobs}`)
  - overlay stack (`Overlay::{Dialog(DialogState), Help(HelpState), Palette(PaletteState)}`)
  - sessions (T103), tree state, toasts (T205), terminal size, resolved `Keymap` (T102) and `Theme` (T201)
- **No owned focus in views:** views read `state.focus` instead of holding a `focus: bool`.
- **Retire the old template:** `Component`, `Action`-as-message-bus and `Mode` are removed. `Action` survives only as the keymap's command vocabulary (T102), and the reducer interprets it.
- **Runtime:** `runtime.rs` runs the loop below. Rendering happens on the frame-rate tick and after any batch of events that changed state, tracked with `state.dirty`.

  ```rust
  loop {
      let event = select!(tui event, worker answers, file answers, timers);
      for effect in app.handle(event) { self.execute(effect).await }
      if app.state.dirty { tui.draw(|f| views::draw(&app.state, &theme, f, f.area())) }
  }
  ```
- **Behaviours that must be preserved (ported with their tests from `home.rs`):**
  1. A modal that starts a connect, export, import or connection export waits for the worker's answer. It shows a pending label (`Connecting…`, `Exporting…`, `Importing…`) and stays open with its fields intact on failure.
  2. A late answer meant for a modal that has since closed, or for a different modal, is ignored (`Awaiting`).
  3. A second submit while one is awaiting does nothing.
  4. Passwords never travel in an event or effect. The prompt writes into `secrets::stash`, and the effect carries only the connection id. The stash call lives in the runtime, behind an effect `StashPassword` that holds a `Password`. `Password` is zeroizing, not `Clone` and not `Debug`-printable, which keeps logs safe; the reducer receives the typed text through a key-by-key buffer that is itself a `Password`.
  5. The session starts with an ephemeral in-memory SQLite `scratch` connection. It is never saved.
  6. Connections load from disk at startup through `Effect::LoadConnections`.
  7. `QueryDone` is never logged in full.
- **No I/O in the reducer:** `app/` and `views/` must not import `tokio`, `std::fs`, `std::net`, `std::time::SystemTime`/`Instant::now`, `keyring` or `crate::secrets` I/O functions.

### Migration steps

1. Introduce `State`, `UiEvent` and `Effect` alongside the current code. Move `Home`'s fields into `State` one concern at a time: focus, then modals/awaiting, then help, then tree.
2. Convert each component's `draw` into a `views::*::draw(state, theme, frame, area)` free function. Convert each `handle_key_event` into a reducer function returning `Vec<Effect>`.
3. Replace `App::handle_actions` with `runtime.rs` executing effects. Move the `spawn_blocking` closures there unchanged.
4. Make the db worker answer with `DbAnswer` instead of `Action` (the `db.rs` `Worker::run` return type changes).
5. Delete `components.rs`, `Component`, `Mode` and the `action_tx` plumbing.
6. Wire the T002 `AppHarness` to `App::handle` and port the existing `home.rs` tests to it.

### Out of scope

- New keymap format (T102).
- Multiple sessions (T103 builds on the `SessionId` already present here; this task uses a single `SessionId(0)`).
- Any visual change (T201/T202).

Pixel output after this task should match the current UI. Any differences go in the PR.

## Acceptance criteria

- [x] `crates/qry/src/app/` exists with `state.rs`, `event.rs`, `effects.rs` and `reducer/`; `App::handle(&mut self, UiEvent) -> Vec<Effect>` is the only entry point for input.
- [x] `Component`, `Mode` and the `mpsc<Action>` bus are deleted; `grep -rn "trait Component\|enum Mode" crates/qry/src` returns nothing.
- [ ] `runtime.rs` is the only module that touches the terminal, spawns tasks, reads or writes files, or calls the keychain.
- [x] Views are free functions over `&State`; no view struct stores a focus flag or a `Config` clone.
- [x] The db worker answers with `DbAnswer`; `Action` no longer appears in `db.rs`.
- [x] All seven preserved behaviours above have a passing harness test.
- [x] No `Password` or password text appears in any `UiEvent`/`Effect` `Debug` output; the canary test passes.
- [x] The app runs manually: connect to scratch, run `SELECT 1`, open and close every modal, quit. Behaviour matches the previous release.
- [ ] `cargo test`, clippy `-D warnings` and docs pass in CI.

## Tests

All tests use the T002 `AppHarness` unless noted.

`crates/qry/src/app/reducer_tests.rs`:
- `startup_opens_scratch_and_loads_connections`: the first `handle(Tick)` (or a `Started` event) emits `OpenSession(scratch, :memory:)` and `LoadConnections`, and never `SaveConnections` for scratch.
- `connect_modal_waits_for_answer`: submitting the connection form emits `Db(_, Connect)`, keeps the overlay open with pending `Connecting…`, and emits no focus or mode change.
- `connect_failure_keeps_fields_and_shows_error`: a `Db(ConnectFailed)` keeps the form text and shows the message.
- `second_submit_while_awaiting_is_ignored`: a second Enter emits no effect.
- `late_answer_for_closed_modal_is_ignored`: open export, submit, close with Esc, deliver `Exported`; state is unchanged and no overlay is open.
- `answer_for_other_modal_is_ignored`: export awaiting plus `Connected` and `ConnectFailed` leaves the export overlay pending (port of `an_answer_meant_for_another_modal_is_ignored`).
- `need_password_opens_prompt_and_submit_awaits_connect`: port of `a_request_for_a_password_opens_the_prompt`.
- `import_waits_and_keeps_path_on_failure`, `exporting_connections_waits_for_the_file`, `a_written_export_closes_the_modal`, `a_refused_modal_stays_open_with_its_error`: ports of the `home.rs` tests of the same names.
- `password_text_never_in_effects`: type `CANARY-PW-x1` into the prompt; assert no effect's `Debug` contains it.

`crates/qry/src/app/purity_tests.rs`:
- `reducer_modules_import_no_io`: a source scan of `src/app/**` and `src/views/**` fails on `tokio::`, `std::fs`, `std::net`, `Instant::now`, `SystemTime`, `keyring` and `secrets::{load,store,delete}`. This mirrors sverb `t02_reducer_modules_import_no_io`.

`crates/qry/src/app/determinism_tests.rs`:
- `reducer_is_deterministic` (proptest, 256 cases): two cloned `App`s fed the same 200 random `UiEvent`s (keys, resizes, ticks, canned db answers) produce identical effects and identical `State`. Requires `State: Clone + PartialEq`.
- `reducer_bounds_effects_per_event`: no single event emits more than 64 effects.

`crates/qry/src/runtime_tests.rs` (tokio):
- `runtime_executes_save_off_the_loop`: with a temp `QRY_DATA`, a `SaveConnections` effect writes the file and a failure comes back as `File(SaveFailed)`.
- `db_worker_answers_with_db_answer`: port of `query_result_comes_back_as_actions` in `db.rs`, now asserting `DbAnswer::Connected` then `DbAnswer::QueryDone`.

Snapshots (`crates/qry/src/views/snapshots/`):
- `home_unchanged_80x24` and `home_unchanged_160x48`: rendered from `State::default()` with scratch connected, as the refactor baseline. T202 later replaces them intentionally.

## Implementation notes

- **Layout.** `app/mod.rs` (`App { state }`, `App::new(config, theme, seed)`, `handle`, `state`, `set_theme`, `take_dirty`), `app/state.rs`, `app/event.rs`, `app/effects.rs`, `app/reducer/` (`mod.rs` dispatch and `act`, `keys.rs`, `db.rs`, `files.rs`, `dialogs.rs`, `tree.rs`, `editor.rs`, `results.rs`, `help.rs`), and `app/dialogs/` for the forms' state and key handling (`form.rs`, `textinput.rs`, one file per form). Drawing is `views.rs` (`views::draw(state, theme, frame, area)`, the old `Home::draw` layout) plus `views/{tree,editor,results,status,help,form}.rs`. `runtime.rs` holds `Runtime` (the loop) and `Executor` (the effects). `components.rs`, `components/`, `app.rs`, `Component`, `Home`, `Mode` and the action channel are gone.
- **What the enums hold.** `UiEvent::{Started, Key, Resize, Tick(Instant), Db(SessionId, DbAnswer), File(FileAnswer)}`. `Effect::{Db, OpenSession, SaveConnections, LoadConnections, Import, ExportConnections, ForgetSecret, StashPassword, ClearScreen, Suspend, Quit}`. Left out because nothing uses them yet: `Effect::{CloseSession, ScheduleTimer, CancelTimer}` and `UiEvent::{Timer, Paste}` (paste was ignored before too); T102 and T103 add them with their first user. Added: `UiEvent::Started` (the runtime sends it first; it emits `LoadConnections` then `OpenSession(SessionId(0), scratch)`), `Effect::StashPassword` and `Effect::ClearScreen`. `DbAnswer` also has `Status(StatusCode)` (the worker's status lines) and `WorkerStopped` (the executor sends it when the worker's channel is closed; it replaces `App::send_db`'s message). `FileAnswer` has `LoadFailed` and `SaveFailed`, which the runtime logs and the reducer ignores, as `Action::Error` was only logged before; T205 can toast them. `ConnectionsExported { path, count }`: the reducer now writes the `Exported N connections to …` line.
- **State.** `Focus` has only `Tree`, `Editor` and `Results`, and `Overlay` only `Dialog(DialogState)` and `Help(HelpState)`; `Inspector`, `Jobs` and `Palette` come with their tasks. `State::overlays` holds at most one dialog and one help, the help always last (on top, and it gets the keys first, as before). There is no `SessionState` yet: `State::session` is `SessionId::FIRST` (`SessionId(0)`), and a `Db` answer for any other id is dropped. `StatusCode` moved from `action.rs` to `app/state.rs` and lost its serde derives. `State: Clone + PartialEq + Debug`; `EditorState`'s `PartialEq` compares the text, the cursor and the selection only.
- **Scroll positions are `Cell`s** (`TreeState::offset`, `ResultsState::{row_offset, col_offset, page_rows}`, `HelpState::{scroll, page, max_scroll}`). What fits depends on the terminal, so the view writes them as it draws, as the components did, and the reducer reads them back for PageUp/PageDown. The tree's `selected` is a plain field, clamped by the reducer before each command and by the view.
- **The editor is styled by the reducer** (`reducer::restyle` after every event, in `App::new` and in `App::set_theme`): the `TextArea` keeps its scroll position as it draws, so the view must draw the stored one rather than a styled clone. `views::editor::draw` takes the theme parameter and ignores it.
- **Theme.** It is `State::theme`; the runtime calls `views::draw(state, &state.theme, …)`. `Runtime::new` does what `App::new` did before: `Config::new`, the config warnings, `theme::resolve`.
- **`Mode` became `config::KeySection`** (same variants and serde names, so `.config/config.json` is unchanged). The section follows from the open dialog (`State::key_section`). `Action` is now only what keys are bound to: `Quit, Suspend, ClearScreen, Help, FocusNext, FocusPrev, ChangeMode(KeySection), Execute(String)`. A user config binding one of the removed variants (`Tick`, `Render`, `Resume`, `Error`, …) no longer loads; no default did. T102 replaces both.
- **Key routing** (`reducer/keys.rs`) keeps the old order: the section's binding is looked up first (with `State::pending_keys` for sequences), then the key goes to the help, else the dialog, else the focused pane, and the bound action runs last. `pending_keys` is cleared on every `Tick`, as before; T102 replaces that with a timer.
- **Passwords.** `Password` is `pub`, not `Clone` (`duplicate()` is the explicit copy), and gained `insert`/`remove`/`chars` so a prompt is typed key by key into it (`Field::Secret(SecretInput)`, which replaced `TextInput::masked`). `SecretInput`'s `Clone` duplicates on purpose, so `State` can be cloned. Forms return a `Submission` (in `app/dialogs/mod.rs`) instead of calling `secrets::stash`; the reducer turns a typed password into `Effect::StashPassword` followed by `Db(Connect)` or `Db(PasswordEntered)`, and the runtime stashes it. `Effect` is not `Clone` because of it. `DbCommand::PasswordEntered` still names no connection (T103 keys the stash by id).
- **Ids without the clock.** `StoredConnection::with_id` was added. The reducer mints form ids from `IdGen`, seeded by the runtime with the new `connections::id_seed()` (the old `mint_id` hash); the harness seeds 0. The scratch connection has the fixed id `scratch` (it is ephemeral, never saved and has no secret).
- **Runtime.** `Executor::new(data_dir)` starts the worker and returns its answers' receiver and an event receiver for file outcomes. `execute(effect) -> Executed::{Terminal(effect), Spawned(JoinHandle), Done}`: `Quit`, `Suspend` and `ClearScreen` go back to the loop, which owns the `Tui`. The `spawn_blocking` closures are the old ones, sending `FileAnswer`s. The data directory is passed in (the runtime passes `get_data_dir()`), so `runtime_executes_save_off_the_loop` uses a temp directory instead of a temp `QRY_DATA`: `QRY_DATA` is read once per process through a `LazyLock`. `connections::load()` and `save()` were dropped for `load_from`/`save_to`. The loop draws on every frame-rate tick, as before, and after any event that set `dirty` (every event but `Tick`). It logs every event at debug except ticks and `QueryDone` (`runtime::log_line`), and every effect; `DbAnswer`'s `Debug` prints a result as `QueryDone(N rows × M columns)` besides. `OpenSession` connects the one worker; T103 gives each session its own.
- **"`runtime.rs` is the only module that …" is left unticked.** `runtime.rs` is the only caller of that I/O, but the worker it spawns (`db.rs`) still spawns its own task, looks passwords up in the keychain and writes result exports itself, which T103 expects it to keep doing; `connections`, `import`, `secrets` and `tui` are helpers that only `runtime.rs` and the worker call. Moving the worker's keychain and export I/O into `runtime.rs` would contradict T103's "`Worker` keeps its current logic", so the box needs rewording rather than more code. `app/` and `views/` do none of it, which `purity_tests` checks: besides the listed words it rejects `std::env`, `crate::runtime`, `db::spawn` and any `secrets::` path other than `secrets::Password`.
- **Bug found by the determinism test**, fixed in its own commit: deleting a tree connection while it was being opened left `attempting` pointing past the end, so the late answer panicked, and deleting the active one moved the active mark to its neighbour. Indexes to a deleted node are now cleared. The proptest regression seed is committed under `crates/qry/proptest-regressions/`.
- **Harness API changes.** `mode()` is now `section() -> KeySection`; `send` takes a `UiEvent`, with `db(DbAnswer)` and `file(FileAnswer)` for the session's worker and the files; `state()` is public and `state_mut()` is `#[cfg(test)] pub(crate)` (they replace `home()`/`home_mut()`); `Effect` is `app::Effect`, re-exported. `AppHarness::new` does not send `Started`, so the tree starts empty as before. With a `FakeDatabase` it answers `OpenSession`, `Db(Connect)`, `Db(Query)` and `Db(ListTables)`. New helpers: `testing::instant(ms)` (tests under `src/app/` may not call `Instant::now`) and the `#[cfg(test)]` `testing::sources(dirs)` for source scans. A query is now run by typing it in the editor and pressing F8.
- **Tests.** 234 before, 254 after: 212 `qry` unit tests, 2 in `tests/harness.rs`, 3 in `tests/no_raw_colors.rs`, 6 in `tests/workspace_metadata.rs`, 31 in `qry-core`. The `home_tests` became `app/reducer_tests.rs` (behaviour) and `views/views_tests.rs` (snapshots and the focus highlight). `a_failed_connection_keeps_the_modal_and_everything_typed` was split into `connect_modal_waits_for_answer`, `second_submit_while_awaiting_is_ignored` and `connect_failure_keeps_fields_and_shows_error`. Renamed: `an_import_waits_for_the_file_…` → `import_waits_and_keeps_path_on_failure`, `an_answer_meant_for_another_modal_is_ignored` → `answer_for_other_modal_is_ignored`, `a_request_for_a_password_opens_the_prompt` → `need_password_opens_prompt_and_submit_awaits_connect`, `update_is_forwarded_to_children` → `a_status_answer_reaches_the_status_bar`, `an_export_hands_the_saved_list_to_the_app` → `…_to_the_runtime`, `what_is_typed_goes_to_the_stash_and_not_to_the_action` → `what_is_typed_is_kept_as_a_password_and_never_printed`, and `db.rs`'s `query_result_comes_back_as_actions` → `…_as_db_answers` (kept; `runtime_tests::db_worker_answers_with_db_answer` checks the same through the executor). The `conntree`, `results`, `editor` and `help` tests now go through the reducer functions or `App::handle` and draw through the views.
- **Snapshots.** Every existing snapshot moved to `crates/qry/src/views/snapshots/` under its new module path, with a body identical to the old one (diffed before the old files were deleted). `home_unchanged_80x24` and `home_unchanged_160x48` are new: the harness after `Started` and the scratch database's `Connected` and status answers.
- **Manual run.** Through a pty (`script`), with temporary `QRY_DATA`/`QRY_CONFIG` and `QRY_KEYRING=off`: scratch connected, `SELECT 1 AS one` gave `Fetched 1 row`, New Connection, Export, Import Connections and Export Connections opened and closed with Esc, a Postgres connection with no stored password opened the password prompt (`for pg`) and Esc closed it, and Ctrl-q quit with exit status 0. The old binary could not be built side by side (the temp filesystem was full), so "matches the previous release" rests on the identical snapshots and the ported tests.
- **Checks run locally:** `cargo fmt --all` (stable, then nightly rustfmt for the import grouping), clippy with `-D warnings`, `QRY_KEYRING=off cargo test --workspace --all-features --locked`, the CI docs command, and `cargo check --workspace --all-features --locked` with Rust 1.96. The CI box stays unticked: nothing was pushed, so GitHub has not run it.
- README's layout block and CONTRIBUTING's harness paragraph were updated for the new modules.
