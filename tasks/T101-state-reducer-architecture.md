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

- [ ] `crates/qry/src/app/` exists with `state.rs`, `event.rs`, `effects.rs` and `reducer/`; `App::handle(&mut self, UiEvent) -> Vec<Effect>` is the only entry point for input.
- [ ] `Component`, `Mode` and the `mpsc<Action>` bus are deleted; `grep -rn "trait Component\|enum Mode" crates/qry/src` returns nothing.
- [ ] `runtime.rs` is the only module that touches the terminal, spawns tasks, reads or writes files, or calls the keychain.
- [ ] Views are free functions over `&State`; no view struct stores a focus flag or a `Config` clone.
- [ ] The db worker answers with `DbAnswer`; `Action` no longer appears in `db.rs`.
- [ ] All seven preserved behaviours above have a passing harness test.
- [ ] No `Password` or password text appears in any `UiEvent`/`Effect` `Debug` output; the canary test passes.
- [ ] The app runs manually: connect to scratch, run `SELECT 1`, open and close every modal, quit. Behaviour matches the previous release.
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
