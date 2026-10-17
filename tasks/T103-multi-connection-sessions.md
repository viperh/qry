# T103 — Multiple open connections (sessions)

**Phase:** 1 — Architecture · **Depends on:** T101 · **Blocks:** T301, T305, T308

## Description

### Current state

`crates/qry/src/db.rs` spawns exactly one `Worker`, which owns one `qry_core::Driver` and a `last: Option<Arc<QueryResult>>`. Connecting to a second database silently replaces the first. In the tree (`components/conntree.rs`), `active: Option<usize>` marks the one live connection; `attempting` and `expand_when_connected` track the pending one. The editor (`components/editor.rs`) and results grid (`components/results.rs`) are global, so switching connections keeps the old SQL and the old result on screen with no hint of where they came from. The scratch `:memory:` SQLite connection from `app.rs` is lost as soon as the user connects anywhere else.

### Target design

A **session** is one open connection plus everything bound to it:
- its worker task
- its editor buffer (text, cursor, undo history)
- its result sets (T303), job list (T305) and schema tree cache (T301)

Several sessions can be open at once. They are shown as tabs in the top bar (drawn by T202): ` 1 scratch `, ` 2 prod `, ` 3 analytics `. The active tab is underlined amber and a session whose job is running shows a spinner.

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(u32);            // monotonically allocated; never reused within a run

pub struct SessionState {             // in app/state.rs, pure data
    pub id: SessionId,
    pub connection_id: String,        // StoredConnection.id; scratch has a fixed id
    pub title: String,
    pub status: SessionStatus,        // Connecting | Open { label } | Failed(String) | Closed
    pub editor: EditorState,
    pub results: Vec<ResultTab>,      // T303
    pub jobs: Vec<JobState>,          // T305
    pub schema: SchemaCache,          // T301
}

pub struct Sessions { pub order: Vec<SessionId>, pub active: SessionId, pub by_id: BTreeMap<SessionId, SessionState> }
```

The runtime side lives in `crates/qry/src/sessions.rs`, the only place that holds senders:
```rust
pub struct SessionManager { workers: HashMap<SessionId, UnboundedSender<DbCommand>> }
impl SessionManager {
    pub fn open(&mut self, id: SessionId, record: StoredConnection, answers: UnboundedSender<(SessionId, DbAnswer)>);
    pub fn send(&self, id: SessionId, cmd: DbCommand) -> Result<(), WorkerGone>;
    pub fn close(&mut self, id: SessionId);   // drops the sender, so the worker disconnects and ends
}
```
- `db::spawn` becomes `db::spawn(id, answers)`, and every answer is tagged with its `SessionId`. `Worker` keeps its current logic: connect, ask for a password, query, export, list tables.
- The reducer emits `Effect::OpenSession(id, record)`, `Effect::Db(id, cmd)` and `Effect::CloseSession(id)`.
- Answers for an unknown or closed `SessionId` are dropped and logged at debug level.
- A dead worker produces `DbAnswer::WorkerStopped`, which sets `SessionStatus::Failed("database worker has stopped")`. This replaces the status message in `App::send_db`.

**Rules:**
- **Startup.** Session 1 is the ephemeral `scratch` in-memory SQLite and opens at startup (`StoredConnection { ephemeral: true }`). It can be closed. When the last session closes, a fresh scratch is opened, so the editor always has a target.
- **Opening.** Enter on a tree connection that has no open session opens a new session and focuses it. If a session is already open for that connection, it switches to it. `shift-Enter` always opens an additional session to the same connection, which is useful for a second transaction.
- **Switching.** `ctrl-1`…`ctrl-9`, `ctrl-PgDn`/`ctrl-PgUp` and the palette switch sessions. Switching swaps the editor buffer and results shown. Focus stays on the same pane.
- **Closing.** `CloseSession` (palette, and `ctrl-w` in Global except while the editor is focused) closes the active session. If a job is running there, a confirm dialog appears first (T203).
- **Tree indicators.** The tree marks connections with open sessions: `●` teal = open, `◌` dim = connecting, `✗` coral = failed.
- **Passwords.** The password prompt (`NeedPassword`) names the session's connection. The stash is keyed by connection id, so two sessions prompting at once cannot swap passwords. This changes `secrets::stash`/`take_stash` to take an id.
- **Status line.** It shows the active session's label (`alice@db.local:5432/app`).
- **Limit.** At most 9 sessions are addressable by digit. More are allowed; tabs overflow with `…` as in tachy `top_bar.rs`.

### Migration steps

1. Add `SessionId` to `DbCommand`/`DbAnswer` routing and change `db::spawn`.
2. Move per-connection fields out of the tree into `SessionState`. The tree keeps only stored records and expansion state.
3. Key `secrets::stash` by connection id.
4. Move the editor and results into `SessionState`.
5. Add `sessions.rs` and wire it into `runtime.rs`.

### Out of scope

- Tab drawing details (T202).
- Schema tree contents (T301).
- Running several queries concurrently inside one session: one worker runs commands in order, and T305 handles cancellation.

## Acceptance criteria

- [x] Two connections can be open at once; queries in session 2 do not affect session 1's driver, buffer or results.
- [x] Every `DbAnswer` is tagged with a `SessionId`; answers for closed sessions are ignored without panicking.
- [x] Each session keeps its own editor text, cursor, undo history and results across switches.
- [x] Scratch opens at startup as session 1, is never written to `connections.json`, and is reopened when the last session closes.
- [x] Enter on a connection with an open session switches to it; `shift-Enter` opens another.
- [x] Closing a session with a running job asks for confirmation; confirming drops the worker and the job is reported cancelled.
- [x] Two simultaneous password prompts each reach the right connection; a canary test proves no cross-delivery.
- [x] Tree indicators reflect session status.
- [ ] Manual check: open scratch plus a real Postgres, switch with `ctrl-1`/`ctrl-2`, run different queries in each, close one.

## Tests

`crates/qry/src/app/sessions_tests.rs` (harness with a `FakeSessions` that records effects and lets tests inject `DbAnswer`s):
- `startup_has_scratch_session_one`.
- `enter_on_connection_opens_new_session_and_focuses_it`: the effect is `OpenSession(SessionId(2), record)` and the active session is 2.
- `enter_on_open_connection_switches_instead`.
- `shift_enter_opens_second_session_same_connection`.
- `switching_sessions_swaps_editor_and_results`: type `select 1` in 1, switch to 2, type `select 2`, switch back; the buffer is `select 1` with the cursor at its end.
- `answer_for_closed_session_is_dropped`: close 2, then deliver `Db(2, QueryDone)`; state is unchanged.
- `closing_last_session_reopens_scratch`.
- `close_with_running_job_asks_first`: emits no `CloseSession` until confirmed.
- `worker_stopped_marks_session_failed`.
- `ctrl_digit_out_of_range_is_ignored`: `ctrl-7` with 3 sessions does nothing.

`crates/qry/src/sessions_tests.rs` (tokio, real workers on SQLite):
- `two_sqlite_sessions_are_independent`: session A creates a table in `:memory:`, and session B (another `:memory:`) does not see it.
- `close_drops_worker`: after `close`, `send` returns `WorkerGone`.
- `answers_are_tagged_with_session_id`.

`crates/qry/src/secrets_tests.rs`:
- `stash_is_keyed_by_connection`: stash `CANARY-PW-a` for `a` and `CANARY-PW-b` for `b`; `take_stash("b")` returns only b's password.

Snapshots (render only; T202 owns the final look):
- `top_bar_three_sessions_120x40` and `top_bar_overflow_80x24`: tabs with the active one marked, and overflow `…`.

## Implementation notes

- **The manual check is still open.** No Postgres was reachable where this was built. A pty smoke run of the real binary covered scratch only: run `select 42` (`Fetched 1 row`), close the only session with `ctrl-w` from the tree, and see a fresh scratch open as session 2 (`OpenSession(SessionId(2), …)` in the debug log). Exit status was 0.
- **State.** `app/state.rs` has `SessionId` (`FIRST` is now `SessionId(1)`), `SessionState { id, connection_id, title, status, editor, results, running, expand_when_connected }` and `Sessions { order, active, by_id, next }`. `Sessions::default()` already holds the scratch session as 1, so the editor has a target before `Started`. `Started` only starts its worker. `SessionStatus` stays in `theme.rs`, which drew it first, and `app::state` re-exports it. `jobs` (T305) and `schema` (T301) are not fields yet. `running` counts queries sent and not yet answered, and feeds the tab spinner (`◐`) and the close confirmation until T305 brings jobs. `State::editor()/editor_mut()/results()/results_mut()` reach the active session's. `TreeState` lost `active`, `attempting` and `expand_when_connected`, and gained `position(id)`.
- **Reducer (`reducer/sessions.rs`).** `open` adds a session, makes it active and emits `OpenSession`. `open_or_switch` (tree Enter) switches to the session already on that connection (the active one if it is on it, else the first in tab order) and reconnects it if it had failed. `reconnect` sends `Db(id, Connect)`. `close` emits `CloseSession`; it reports `Closed <title>: its running query was cancelled` when something was running, drops that session's queued password prompts and open prompt, and opens a fresh scratch when none are left. `ctrl-1`…`9` switch by tab position, `ctrl-PgDn`/`ctrl-PgUp` wrap, and an out-of-range digit does nothing. Submitting the New Connection modal goes through `connect`: the session already on that connection id reconnects with the edited record, or a new one opens. Its id is what the modal waits for.
- **Answers.** `UiEvent::Db(id, _)` for an id not in `Sessions` is dropped in the reducer. The runtime already drops and debug-logs answers for closed sessions. `Awaiting::Connect(SessionId)` and `Awaiting::Export(SessionId)` carry the session, so another session's answer cannot close or fail the modal. `Connected` sets `Open { label }`. `ConnectFailed` sets `Failed`, and for a connection added this run and never saved it also forgets the node and closes the session (the modal stays open with its fields). `WorkerStopped` sets `Failed("database worker has stopped")`, clears `running` and fails a modal waiting on that session; it no longer writes the status bar.
- **Passwords.** `PasswordForm` carries the asking session, and `Submission::PasswordEntered { session, connection, password }` sends `PasswordEntered` back to that session. A `NeedPassword` from another session while a prompt is open waits in `State::password_queue`. Closing a dialog opens the next prompt in the queue.
- **New action and keys.** `Action::OpenNewSession` (Tree `shift-Enter`, "Open another session on it"). Global `ctrl-w` is `CloseSession`; the editor's own `ctrl-w` (DeleteWordBack) shadows it. `print_config_default` was re-accepted. Terminals without the kitty keyboard protocol send plain Enter for shift-Enter. There the second session is reachable once the palette (T206) lists `OpenNewSession`.
- **Confirm dialog.** `dialogs/confirmform.rs` (`DialogState::Confirm`, `Submission::CloseSession(id)`) is a one-choice form: Enter closes, Esc keeps. `views::form` cannot draw a form with no fields, so it has one choice field. T203 can restyle it.
- **Views.** A one-row top bar (`views/topbar.rs`) draws the tabs ` N title ` with `tab_active` / `tab_running` / `tab`. When they overflow, tabs before the active one give way to a leading `…`, and a trailing `…` marks more to the right (tachy `top_bar.rs` behaviour; T202 owns the final look). The status panel became a one-row status line. It shows the active session (`status_session` label when open, `connecting to <title>…` dim, `<title>: <error>` as a warning) and then the last message. The tree draws `●`/`◌`/`✗` from `theme.session_dot` after the expand marker. A connection with no session gets blank space there, and the active session's connection is bold. Every home/dialog snapshot moved down a row for the top bar and was re-accepted.
- **Harness.** `AppHarness` keeps one virtual worker clock per session, answers each `OpenSession`/`Db` effect for the session that sent it, and drops pending answers of a closed session. `db()` answers for the active session, and `db_for(id, …)`, `active_session()` and `sessions()` are new. `app/sessions_tests.rs` has the listed tests plus `queries_run_in_the_active_session_only`, `next_and_previous_session_wrap_around`, `ctrl_w_in_the_editor_deletes_a_word_instead`, `two_password_prompts_each_reach_their_own_session` (the canary), `closing_a_session_drops_its_waiting_password_prompt`, `a_failed_saved_connection_reconnects_on_enter` and `the_status_line_shows_the_active_sessions_label`. Its `FakeSessions` wraps `AppHarness`, so no second fake runtime exists. The two top-bar snapshots live in `app/snapshots/`. `reducer_tests::ctrl_digit_switches_session` now switches for real, and `tree_tests` drives connects through the reducer's `connect` and `db::answer`.
- **Checks run locally:** `cargo fmt --all`, clippy with `-D warnings`, `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked` (267 `qry` unit tests, 31 in `qry-core`), and the CI docs command with `RUSTDOCFLAGS=-D warnings`. Nightly rustfmt was not available, so import grouping was not re-run.

