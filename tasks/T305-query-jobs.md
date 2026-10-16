# T305 — Query jobs: progress, cancellation, timing, row limits

**Phase:** 3 — Features · **Depends on:** T103, T202 · **Blocks:** T307

## Description

### Current state

`crates/qry/src/db.rs` runs one worker task that owns one `qry_core::Driver`. Commands run strictly one at a time.

- `DbCommand::Query(String)` awaits `driver.query(sql)` to completion. Each backend then materialises **every row** into `QueryResult.rows`.
- `SELECT * FROM big_table` can exhaust memory and freezes the worker. Nothing else can run, including table listing and export.
- There is **no way to cancel** a running query, and no elapsed time is shown. The only feedback is the status bar message after completion.
- **Transactions:**
  - Oracle commits after every statement (`execute_and_commit`, `oracle.rs:301`).
  - The other backends run in their driver's default autocommit.
  - The user cannot see or control any of this.

### Target design

#### Jobs

Every database operation started by the user becomes a **job**: a statement run, "load more", an export re-run (T307), or a schema load (T301, hidden by default).

```rust
pub struct JobId(u64);
pub enum JobKind { Query, FetchMore, Export, Schema }
pub enum JobState { Queued, Running { rows: u64 }, Done { rows: u64 }, Failed(String), Cancelled }
pub struct Job { pub id: JobId, pub session: SessionId, pub kind: JobKind, pub sql: String,
                 pub started: Option<Instant>, pub finished: Option<Instant>, pub state: JobState }
```

- Jobs are held in a pure `Jobs` registry in app state (T101). The worker sends `JobUpdate` events. Elapsed time is computed against the T101 clock, so it is testable with a virtual clock.
- **One session runs one statement at a time** (a database connection is serial). Further jobs for that session are `Queued` in FIFO order.
- Different sessions run concurrently (T103).

#### Visual feedback

**While running:**

- The results pane title shows a teal braille spinner, ` ⠹ running · 2.4 s · 12 000 rows `.
- The status line shows the same, compactly.
- The hint bar shows `ctrl-c cancel`.

**When finished**, the result tab and status show `1 204 rows · 38 ms`. Number formatting uses a thin-space thousands separator (` `) and adaptive units (ms below 1 s, then `2.4 s`, then `1 m 12 s`).

**Jobs drawer** (`J` toggles it; T102 Jobs context). It is modelled on tachy's drawer: height `1 + min(n, 7)` rows, above the status line.

```
 JOBS · 2 running                                              ↑↓ select  c cancel  Enter show  J close
 ▸ local-app   SELECT * FROM events WHERE …         running  4.1 s   38 000 rows
 ▸ prod-db     SELECT count(*) FROM orders          running  0.8 s
 ✓ local-app   UPDATE users SET …                   done     12 ms   3 rows
 ✗ billing     SELECT * FROM nope                   failed   ORA-00942: table or view does not exist
 – local-app   SELECT * FROM huge                   cancelled 9.3 s
```

- The title is purple (`jobs_running`).
- State glyphs and styles come from `Theme::job_state` (T201): `▸` running in the kind colour, `✓` green, `✗` coral, `–` dim, `…` queued dim.
- At most 50 finished jobs are kept. Older ones are dropped.

#### Cancellation

`ctrl-c` in the Editor or Results context, or `c` in the drawer, cancels the session's running job. The job moves to `Cancelled` with a dim toast `query cancelled`. Queued jobs are removed with `c` in the drawer.

Per-backend support lives in qry-core. Each backend exposes a cancel handle, obtained **before** the query starts, so cancellation does not need the busy connection:

```rust
#[async_trait]
pub trait CancelHandle: Send + Sync { async fn cancel(&self) -> Result<()>; }
fn cancel_handle(&self) -> Box<dyn CancelHandle>;   // new method on Database
```

| Backend | Mechanism |
|---|---|
| Postgres | `tokio_postgres::Client::cancel_token()` → `cancel_query(NoTls)` |
| MySQL / MariaDB | open a short-lived second connection with the same config and run `KILL QUERY <connection_id>` (id read at connect via `SELECT CONNECTION_ID()`) |
| SQLite | `rusqlite::InterruptHandle` from `tokio_rusqlite` (`conn.get_interrupt_handle()`) |
| Oracle | `oracledb` `Connection::break_execution()`, or the driver's equivalent. If the beta driver lacks it, document the limitation: cancel then marks the job Cancelled and discards the late result, and the toast says `cancellation not supported by the Oracle driver; the server statement continues` |

#### Row limits and streaming

- **Config:** `max_rows` (default 1000; `0` means unlimited) and `fetch_size` (default 1000).
- A query streams rows from the backend and stops reading at `max_rows`. If more rows exist, `QueryResult.truncated = true`.
  - Postgres: `query_raw` row stream on the extended protocol, or a portal.
  - MySQL: `query_iter`.
  - SQLite: step the statement.
  - Oracle: the cursor iterator.
- **Truncated results** show a dim footer row in the grid, `… more rows — L load next 1000 · A fetch all`.
  - `L` starts a `FetchMore` job that continues the **same open cursor or stream**. It must not re-run the query, so results stay consistent.
  - `A` fetches all, with a confirm dialog when no row count estimate is available.
- **Keeping a cursor alive:**
  - It needs the session connection to stay dedicated to that cursor. A new query in the same session closes the pending cursor first; the old tab keeps its rows and loses its `L` option.
  - For backends where streams cannot be held across awaits safely (MySQL `query_iter` borrows the connection), the worker keeps the stream inside a task-local state machine owned by the session worker.
- **Progress:** `Running { rows }` is updated every `fetch_size` rows, so the spinner line shows a live row count.

#### Transactions

- The status line shows a transaction indicator per session:
  - `auto` (dim) in autocommit.
  - ` TX ` (amber pill) when a transaction is open, detected from `BEGIN`/`START TRANSACTION`/`SET autocommit=0`, or when autocommit is off.
- **Palette commands** (T206): `Commit`, `Rollback`, `Toggle autocommit`.
- **Oracle** follows the setting instead of always committing. It defaults to autocommit **on**, to keep today's behaviour.
- **Open transaction on exit or disconnect.** Quitting qry or disconnecting a session with an open transaction shows an amber confirm dialog, `prod-db has an open transaction`, with `Commit` / `Rollback` / `Cancel`.

### qry-core split

- `CancelHandle` and `cancel_handle()` are added to the `Database` trait.
- `query_stream(sql, fetch_size) -> Result<Box<dyn RowStream>>`, where `RowStream::next_batch(&mut self, n) -> Result<Option<Vec<Row>>>` and `columns() -> &[ColumnMeta]`.
- `set_autocommit(bool)`, `commit()`, `rollback()` and `in_transaction() -> bool` (best effort).
- The existing `query()` becomes `query_stream` plus collect-all, used by tests and small internal queries.
- A pure `Jobs` registry and formatting helpers (`format_rows`, `format_elapsed`) go in `crates/qry-core/src/jobs.rs`.

### Edge cases

- Cancel arrives after the query already finished: no-op, and the job stays `Done`.
- Cancel while queued: the job is removed and never sent.
- A session disconnects with running and queued jobs: all become `Cancelled`, with reason `session closed`.
- The backend errors mid-stream after 5 000 rows: the job becomes `Failed`. The partial rows are kept, with a coral footer `stopped after 5 000 rows: <error>`.
- `max_rows = 0` with a 10M-row table is allowed. The jobs drawer still updates the live count, and cancel works.
- Very fast queries (< 1 ms) show `0 ms`, not `0.0 s`.

### Out of scope

Query plans and EXPLAIN visualisation, server-side cursors for DML, scheduled jobs.

## Acceptance criteria

- [x] Every user-started database operation is a job with id, session, kind, SQL, timing and state, visible in the jobs drawer (`J`).
- [x] Jobs in one session run in FIFO order. Jobs in different sessions run concurrently.
- [ ] A spinner, elapsed time and live row count show in the results title and status line while running. Completed results show `N rows · T`.
- [ ] `ctrl-c` cancels the running job of the focused session on Postgres, MySQL, MariaDB and SQLite, and the server statement actually stops (integration-tested with `pg_sleep` / `SLEEP` / a recursive CTE). The Oracle behaviour is implemented or documented as specified.
- [ ] `max_rows` (default 1000) truncates with a visible footer. `L` continues the same cursor without re-running the query. `A` fetches all, after confirmation.
- [ ] Memory: fetching the first 1000 rows of a 10M-row table never holds more than `max_rows + fetch_size` rows in memory (bench/integration assertion through a counting `RowStream`).
- [ ] The transaction indicator is correct after `BEGIN`, `COMMIT`, `ROLLBACK` and autocommit toggles. Commit and Rollback work from the palette.
- [ ] Quitting or disconnecting with an open transaction asks first. `Rollback` in that dialog really rolls back (integration test).
- [ ] Oracle no longer commits unconditionally when autocommit is off.
- [ ] Elapsed times in tests come from the virtual clock. No test sleeps.

## Tests

**Unit tests** — `crates/qry-core/src/jobs_tests.rs`:

- Queueing and lifecycle:
  - `fifo_per_session_parallel_across_sessions`
  - `cancel_queued_removes_without_sending`
  - `cancel_after_done_is_noop`
  - `session_close_cancels_running_and_queued`
  - `finished_jobs_capped_at_50`
- Formatting:
  - `format_elapsed_units` (0 ms, 999 ms, 2.4 s, 1 m 12 s)
  - `format_rows_thin_space_thousands`

**Unit tests** — `crates/qry/src/jobs_drawer_tests.rs`, `crates/qry/src/app/transaction_tests.rs`:

- Drawer: `drawer_toggle_and_height_formula`, `drawer_c_cancels_selected`.
- Keys: `ctrl_c_cancels_focused_session_job`, `truncated_footer_l_emits_fetch_more_not_rerun`.
- Transactions:
  - `quit_with_open_tx_shows_confirm`
  - `tx_indicator_after_begin_commit_rollback`

**Proptest:** `prop_jobs_registry_invariants`. Random enqueue, start, finish, cancel and close events must keep at most one `Running` job per session, and no job may leave a terminal state.

**Snapshots** — plain and styled:

- `results_running_spinner_120x20` (clock advanced 2 400 ms)
- `results_truncated_footer_120x20`
- `results_failed_mid_stream_120x20`
- `jobs_drawer_mixed_states_120x40`
- `status_tx_pill_120x2`
- `confirm_open_tx_on_quit_80x24`

**AppHarness flows:**

- `h.keys("ctrl-enter")` on a fake slow session, `h.advance(1500)`, then `h.keys("ctrl-c")`. Assert the `CancelJob` effect, the job state `Cancelled` and the toast.
- With two sessions, run in both and assert two `Running` jobs at once.

**DB integration tests** — `crates/qry-core/tests/cancel_<backend>.rs` and `stream_<backend>.rs`, using `db_or_skip!`:

- `cancel_stops_long_query`
  - Postgres: `SELECT pg_sleep(30)`, cancelled after 200 ms. Assert it returns within 2 s with a cancel error, and that the connection is still usable afterwards (`SELECT 1`).
  - MySQL/MariaDB: `SELECT SLEEP(30)`.
  - SQLite: a recursive CTE counting to 1e9.
- `stream_stops_at_max_rows_and_reports_truncated` (`generate_series(1, 1e6)` / recursive CTE)
- `fetch_more_continues_same_cursor`: the rows continue from 1001 with no duplicates, even after concurrent inserts.
- `autocommit_off_rollback_discards`
- `oracle_autocommit_off_does_not_commit` (`QRY_TEST_ORACLE_URL`)

**Benchmark** (T006): `stream_first_1000_of_1m_sqlite`, gated at ≤ 50 ms (CI ≤ 100 ms).

## Implementation notes

Done in this pass:

- **Registry.** `qry_core::jobs` holds `JobId`, `JobKind`, `JobState`, `Job`
  and a pure, generic `Jobs<S>` registry (FIFO per session, one running per
  session, cancel of queued vs running, close-session, 50 finished kept),
  with `format_elapsed` (`0 ms`, `2.4 s`, `1 m 12 s`) and `format_rows`
  (thin-space thousands; `status_line::thousands` now uses it). Unit tests
  and the `prop_jobs_registry_invariants` proptest are in `jobs_tests.rs`.
  `theme::JobKind`/`JobState` are now re-exports of these.
- **App.** Every statement a run queues is a job (`Queued.job`); it starts
  when sent and ends with the worker's own elapsed time. `State.now` is the
  time since the first tick, from the ticks' `Instant`s, so the harness's
  virtual clock drives it; a running job's time therefore moves in tick
  steps (250 ms). Schema loads are not jobs.
- **Cancel.** `CancelHandle` / `Database::cancel_handle()`: Postgres
  `cancel_token()`, MySQL/MariaDB `KILL QUERY <id>` on a short-lived second
  connection, SQLite `InterruptHandle`; Oracle has none. The worker leaves
  the handle in a slot the `SessionManager` holds, and
  `Effect::CancelQuery(session)` uses it on a task of its own. `ctrl-c` in
  the editor (copy when nothing runs) and the results, or `c` in the drawer,
  cancels the running job and drops the rest of its run, with a toast
  `query cancelled`. The worker's failure for a cancelled job, or the late
  result of a driver that cannot cancel, is dropped. Without a handle the
  runtime warns `cancellation not supported by this driver; the server
  statement continues`.
- **Row limit.** Config `max_rows` (default 1000, 0 = all).
  `Database::query_limited(sql, max_rows)` stops reading at the limit:
  SQLite stops stepping, Postgres stops reading the `simple_query_raw`
  stream, MySQL/MariaDB stop at `next()` (the rest is read off the wire and
  dropped), Oracle stops iterating the cursor. A truncated result has a dim
  footer `… more rows — showing the first 1 000 · A fetch all`, and the title
  says `(first rows only)` / `sorted … (fetched rows only)`.
- **Feedback.** Results title and status line show `⠹ running · 2.2 s`
  while the active session runs; the hint line leads with `ctrl-c cancel`.
- **Drawer.** `J` (Tree, Results) toggles it; height `1 + min(n, 7)`;
  j/k move, `c` cancels (a queued job is dropped unsent), Enter goes to the
  job's session, `J`/Esc close. Newest first.
- **Transactions.** The status line shows `auto` (dim) or an amber ` TX `
  pill; `SET autocommit=0/1` is also recognised. Palette `Commit` and
  `Rollback` send the statement on the active session. Quitting with an
  open transaction asks (`Open Transaction`, Cancel default, `Roll back &
  quit` runs ROLLBACK in each such session, then quits); closing such a
  session asks too (dropping the connection rolls it back).
- **Fixes on the way.** Statements a failure stops are now cancelled jobs
  rather than left queued.

Not done, and why:

- **`L` load more / keeping a cursor open.** Not built: `A` re-runs the
  statement with no limit (`DbCommand::QueryAll`) and has no confirm.
  `FetchMore` exists only as a kind. The memory box needs the streaming
  `RowStream`, which is not built; the limit is applied while reading, but
  there is no counting-stream assertion.
- **Live row counts.** No progress answers from the worker yet, so a
  running job shows time only.
- **Autocommit toggle and Oracle.** No `set_autocommit`; Oracle still
  commits after each non-query statement.
- **Integration tests** (`cancel_*`, `stream_*`, rollback-on-quit) against
  real servers are not written; SQLite's limit and interrupt are tested in
  `qry-core/src/lib_tests.rs` (the interrupt test waits 100 ms of real time
  for the statement to start, so the "no test sleeps" box stays open).
- **Snapshots.** `results_running_spinner` and `results_truncated_footer`
  are 120x24, as 120x20 is under the 80x24 minimum; there is no
  `results_failed_mid_stream` (no streaming errors yet).

