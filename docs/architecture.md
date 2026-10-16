# Architecture

## Crates

```
crates/
  qry-core/   the databases and everything that needs no terminal
  qry/        the terminal app: state, reducer, views, runtime
fuzz/         cargo-fuzz targets (its own workspace)
```

**`qry-core`** holds the drivers (`postgres.rs`, `mysql.rs`, `mariadb.rs`,
`sqlite.rs`, `oracle.rs`) behind one `Database` trait, and the pure logic
the app draws on: the SQL lexer and statement splitter (`sql/`), result
grid sorting, searching and sizing (`grid.rs`), the copy and export writers
(`copy.rs`, `exporter.rs`), the schema catalogs and cache (`schema.rs`), the
value inspector (`inspect.rs`), the jobs registry (`jobs.rs`), history and
snippets (`history.rs`), and the fuzzy matcher (`fuzzy.rs`).

The layering rule: **`qry-core` never depends on `ratatui`, `crossterm` or
`clap`** (a workspace test checks the manifests). Everything in it is tested
without a terminal.

## The app: state, events, reducer, effects

The `qry` crate is built around one pure function:

```
App::handle(UiEvent) -> Vec<Effect>
```

- **`State`** (`app/state.rs`) is everything the screen shows, as plain
  data: the sessions, the tree, the overlays, the toasts, the jobs.
- **`UiEvent`** (`app/event.rs`) is everything that can happen: a key, a
  mouse event, a tick, a database worker's answer, a file read finishing.
- **The reducer** (`app/reducer/`, one file per concern) turns an event
  into a new state and a list of **`Effect`s** (`app/effects.rs`): run this
  SQL on that session, save the connections, copy this text, quit.
- **The views** (`views/`) are free functions that draw a `State` with a
  `Theme`. They never change the state.
- **The runtime** (`runtime.rs`) owns the terminal, the tasks and the
  files. It turns terminal input into events, carries out effects, and
  turns their outcomes back into events.

The reducer and the views do no I/O, read no clock and touch no
environment; `app/purity_tests.rs` scans their source to keep it that way.
That is what makes the whole UI testable: `testing::AppHarness` feeds events
to the reducer, answers database effects from a `FakeDatabase` on a virtual
clock, and renders to an in-memory buffer.

## Sessions and workers

Each open connection is a *session* (`SessionId`). The runtime's
`SessionManager` gives each one a database worker (`db.rs`): a task owning
one `Driver`, running its commands one at a time and answering with
`DbAnswer`s tagged with the session. Sessions never wait for each other.

## Jobs

Every statement a run queues is a *job* in `State.jobs`
(`qry_core::jobs::Jobs`): queued, running, then done, failed or cancelled,
timed on the ticks' clock. A session sends its next statement only when
the last one answered, so a failure can stop the rest of a run. `ctrl-c`
marks the running job cancelled and emits `Effect::CancelQuery`; the
runtime uses the connection's cancel handle (taken when it connected, so it
never needs the busy connection) to stop the statement on the server.
Queries keep at most `max_rows` rows; the drivers stop reading there.

## Keys and actions

Keys never reach the reducer as keys for long: the keymap
(`keymap.rs`, built from `.config/config.json` and the user's config) turns
them into `Action`s for the key contexts in effect (the focused pane, an
open dialog, the palette). The help overlay, the hint line, the palette and
[keybindings.md](keybindings.md) are all built from the same keymap through
`help_model.rs`.
