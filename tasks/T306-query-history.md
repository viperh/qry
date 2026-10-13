# T306 — Query history and saved snippets

**Phase:** 3 — Features · **Depends on:** T302 · **Blocks:** —

## Description

### Current state

qry forgets every query. Once the editor buffer changes, a statement is gone. There is no history, no recall, and no way to keep a named query. The data directory (`config::get_data_dir()`, `QRY_DATA`, `.data/` in dev via `.envrc`) holds only `qry.log`. Connections are stored separately (`connections.rs`, `load_from`/`save_to`).

### Target design

#### History

**What is recorded.** Every statement that is actually sent to a backend (one entry per T302 statement, not per run) is appended to a per-connection history file:

```
<data dir>/history/<connection id>.jsonl
{"ts":"2026-10-13T09:12:44Z","sql":"SELECT * FROM users WHERE id = 3","ok":true,"rows":1,"ms":4}
{"ts":"2026-10-13T09:13:02Z","sql":"DROP TABLE tmp","ok":false,"error":"table \"tmp\" does not exist","ms":2}
```

- The file is named by `StoredConnection.id`, not by name, so renames keep history.
- The scratch `:memory:` connection (`ephemeral`) records to an in-memory history only. Config `history.ephemeral_to_disk = false`.
- Writes are appends done off the event loop (`spawn_blocking`, the same pattern as `App::save_connections`), and a write failure produces a coral toast once per session.
- **Consecutive duplicates** (the same normalised SQL: trimmed, with whitespace runs collapsed) only update `ts`/`ok`/`ms` of the last entry.

**Retention:**

- Capped at `history.max_entries` (default 1000) per connection.
- Compaction rewrites the file atomically (write to a temp file in the same dir, then rename) when it exceeds the cap by 10%.
- `history.enabled = false` disables recording entirely.

**Privacy.**

- History holds SQL text only, never connection passwords. The `secrets::Password` type is never serialisable, which T002/T005's compile-fail test proves.
- SQL can still contain secrets the user typed (`ALTER USER … PASSWORD '…'`). Statements matching a configurable redaction list are stored with the literal replaced by `'•••'`. The default list covers `PASSWORD`, `IDENTIFIED BY` and `SECRET`.
- The redaction is done through the T302 lexer: string literals after those keywords are redacted. It is not done by regex.
- On unix, files are created `0600`.

**History dialog** (`ctrl-r` in Editor, or the palette "History"). It is a purple-accented overlay, the same visual family as the palette (T206), because it is a command-like chooser.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ › sel usr▏                                         local-app · 238 entries   │
│ ✓ 09:12  SELECT * FROM users WHERE id = 3                      1 row  4 ms   │
│ ✓ 08:57  SELECT u.name, count(*) FROM users u JOIN orders …    42 rows 31 ms │
│ ✗ yday   SELECT * FROM user_s                                  error         │
└─ Enter insert  ctrl-enter run  tab all connections  ctrl-d delete  Esc close ┘
```

**Matching:**

- Fuzzy, subsequence-based, with the same scorer as the palette (shared `qry_core::fuzzy`).
- Matched characters are drawn with `palette_match` (amber bold). The selected item is `palette_item_selected`.

**Entry rows:**

- Status glyph: `✓` green or `✗` coral.
- Relative time: `HH:MM` today, `yday`, then `Oct 03`.
- First line of the SQL, truncated with `…`.
- Rows and duration, dim, right-aligned.

**Scope and actions:**

- `Tab` switches between the current connection and all connections. In "all" mode a dim connection name column is shown.
- `Enter` inserts the SQL at the cursor as a new statement (on its own lines).
- `ctrl-enter` inserts and runs it.
- `ctrl-d` deletes the entry, after an inline confirm (`press ctrl-d again`).
- `Esc` closes.

**Editor recall.** With the cursor on the first line of an otherwise empty buffer, `alt-↑`/`alt-↓` step through this connection's history, like a shell.

#### Saved snippets

Named, reusable queries, stored in the **config** dir because they are user-authored and should be easy to sync:

```
<config dir>/snippets.json
[{"name":"active users","sql":"SELECT * FROM users WHERE active","connection":null,"tags":["users"]}]
```

- `connection: null` means the snippet is global. Otherwise it is the connection id it belongs to.
- **Save:** `ctrl-s` in the Editor saves the current statement as a snippet through a small amber dialog, with Name (required, unique per scope) and Scope `‹ this connection / global ›`. (`ctrl-s` stops meaning "export connections" in the Editor context; T308 moves that action to the palette.)
- **Find:** the palette lists snippets under a "Snippets" group (`snippet: active users`). Choosing one inserts it, and `ctrl-enter` runs it.
- **Manage:** the palette command "Manage snippets" opens a list with rename (`r`), delete (`d`, coral confirm) and edit SQL (opens it in the editor; saving overwrites).
- **Placeholders** (optional; can be split out): `:name` / `${name}` parameters prompt for values in a dialog before running.

### qry-core split

`crates/qry-core/src/history.rs`:

```rust
pub struct Entry { pub ts: OffsetDateTime, pub sql: String, pub ok: bool, pub rows: Option<u64>, pub ms: u64, pub error: Option<String> }
pub struct History { entries: VecDeque<Entry>, cap: usize }
impl History {
    pub fn push(&mut self, e: Entry);                    // dedupe consecutive
    pub fn search(&self, q: &str) -> Vec<(usize, Score, Vec<usize>)>;
    pub fn to_jsonl(&self) -> String; pub fn from_jsonl(s: &str) -> (Self, Vec<ParseWarning>); // tolerant: bad lines skipped
}
pub fn redact(dialect: Dialect, sql: &str, words: &[&str]) -> String;
```

- `crates/qry-core/src/fuzzy.rs`: `score(query, candidate) -> Option<(Score, Vec<usize>)>`, shared with T206.
- `crates/qry-core/src/snippets.rs`: model, validation (unique names per scope) and JSON (de)serialisation.
- File IO stays in qry (`crates/qry/src/history_store.rs`), behind a small trait so tests use an in-memory store.

### Edge cases

- A corrupt or partially written last line (crash mid-append): it is skipped with a warning in the log, and the rest of the file still loads.
- Two qry instances appending to the same history: append-only writes of whole lines (one `write_all` per line) keep each line intact. Compaction takes an advisory lock file and skips compaction if the lock is held.
- A 100 KB statement is stored in full, and the dialog shows only its first line.
- Deleting a connection (T308) offers to delete its history too (a checkbox in the confirm dialog, default on).
- Clock skew or invalid `ts`: shown as `?`, sorted by file order.

### Out of scope

Syncing history across machines, full-text index, sharing snippets.

## Acceptance criteria

- [ ] Every executed statement is appended to `<data>/history/<connection id>.jsonl` with ts, sql, ok, rows, ms and error. Ephemeral connections stay in memory.
- [ ] Consecutive duplicates are merged. The cap (default 1000) is enforced with atomic compaction.
- [ ] Password literals after `PASSWORD`, `IDENTIFIED BY` and `SECRET` are redacted. Files are `0600` on unix.
- [ ] No `secrets::Password` value can ever reach a history or snippet file. A canary test proves it (below).
- [ ] `ctrl-r` opens a fuzzy history dialog with match highlighting, scope toggle, insert, insert-and-run and delete.
- [ ] `alt-↑`/`alt-↓` recall works on an empty buffer.
- [ ] Snippets can be saved (`ctrl-s`), listed and run from the palette, renamed and deleted, with global or per-connection scope.
- [ ] A corrupt history line never prevents loading the rest.
- [ ] `history.enabled = false` writes nothing (asserted by checking the directory stays empty).

## Tests

**Unit tests** — `crates/qry-core/src/history_tests.rs`:

- Recording: `push_dedupes_consecutive_normalised`, `push_enforces_cap_dropping_oldest`.
- JSONL: `jsonl_round_trip`, `from_jsonl_skips_corrupt_lines_with_warning`.
- Search: `search_scores_subsequence_and_returns_match_positions`.
- Redaction:
  - `redact_password_literal_postgres_alter_user`
  - `redact_identified_by_oracle`
  - `redact_ignores_password_word_inside_string`
  - `redact_keeps_non_secret_literals`

**Unit tests** — `crates/qry-core/src/fuzzy_tests.rs`:

- `prefix_beats_scattered`
- `case_insensitive`
- `no_match_is_none`
- `match_positions_are_char_indices_not_bytes`

**Unit tests** — `crates/qry-core/src/snippets_tests.rs`:

- `names_unique_per_scope`
- `global_and_connection_scopes_listed_for_connection`
- `round_trip_json`

**Proptests:**

- `prop_from_jsonl_never_panics(input in any::<String>())`
- `prop_redact_preserves_length_class_and_never_leaks_literal`: a random password inserted after `PASSWORD` never appears in the output.

**Fuzz:** `fuzz/fuzz_targets/history_jsonl.rs` (an untrusted file on disk), with a proptest twin.

**Integration tests** — `crates/qry/tests/history_store.rs`, in temp dirs under `CARGO_TARGET_TMPDIR`:

- `append_is_line_atomic_under_two_writers`
- `compaction_is_atomic_rename`
- `files_created_0600_on_unix`
- `disabled_writes_nothing`
- `ephemeral_connection_not_written`

**Canary test** — `crates/qry/tests/no_password_in_history.rs`, in the sverb canary style:

- Connect through the fake session with password `CANARY-PW-7f3a`.
- Run statements, save a snippet and quit.
- Scan every file under the temp data and config dirs and assert the canary appears nowhere.
- Also run `ALTER USER x PASSWORD 'CANARY-PW-2'` and assert that canary is redacted.

**Unit tests** — `crates/qry/src/components/history_dialog_tests.rs`:

- `ctrl_r_opens_and_esc_closes`
- `tab_toggles_scope`
- `enter_inserts_as_new_statement`
- `ctrl_enter_inserts_and_runs`
- `ctrl_d_twice_deletes`
- `alt_up_recall_only_on_empty_buffer`

**Snapshots** — plain and styled:

- `history_dialog_120x40`
- `history_dialog_all_connections_120x40`
- `history_dialog_filtered_80x24`
- `snippet_save_dialog_80x24`
- `palette_with_snippets_group_120x40`
