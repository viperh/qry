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

- [ ] Two connections can be open at once; queries in session 2 do not affect session 1's driver, buffer or results.
- [ ] Every `DbAnswer` is tagged with a `SessionId`; answers for closed sessions are ignored without panicking.
- [ ] Each session keeps its own editor text, cursor, undo history and results across switches.
- [ ] Scratch opens at startup as session 1, is never written to `connections.json`, and is reopened when the last session closes.
- [ ] Enter on a connection with an open session switches to it; `shift-Enter` opens another.
- [ ] Closing a session with a running job asks for confirmation; confirming drops the worker and the job is reported cancelled.
- [ ] Two simultaneous password prompts each reach the right connection; a canary test proves no cross-delivery.
- [ ] Tree indicators reflect session status.
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
