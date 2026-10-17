# T307 — Export redesign

**Phase:** 3 — Features · **Depends on:** T203, T305 · **Blocks:** —

## Description

### Current state

- **Form:** `ctrl-e` (`Mode::ExpoModal`) opens `ExportForm` (`crates/qry/src/components/exportform.rs`). Its fields are Path, Type `‹CSV|Excel|Text|JSON›`, Separator `‹COMMA|PIPE|TAB|SEMICOLON|OTHER›`, and Custom, which appears only for OTHER.
- **Submit:** submitting sends `Action::Export(ExportConfig)`. The worker (`db.rs`) writes `self.last`, the **single last result**, through `qry_core::Exporter` (`crates/qry-core/src/exporter.rs`).
- **Waiting:** while the worker writes, the modal waits on `Awaiting::Export` and shows "Exporting…". It closes on `Exported`, or keeps the path and shows the error on `ExportFailed`. These behaviours are tested in `home.rs` and must be kept.
- **Limitations:**
  - It exports only what was fetched. With T305 row limits that would silently be the first 1000 rows.
  - The header row is always written. There is no NULL representation option.
  - An existing file is overwritten without asking.
  - Formats are limited to CSV, Excel, Text and JSON.
  - `ExportType::with_extension` appends the extension.

### Target design

**Dialog.** An amber-accented `dialog_frame` (T203), 80 wide at most, 15 rows, over the backdrop. It is opened by `ctrl-e` from the Results context or the palette command "Export result…".

```
┌ Export · result 2 orders · 7 rows (all fetched) ──────────────────────────┐
│                                                                            │
│ › Path          ~/exports/orders.csv▏                                      │
│   Format        ‹ CSV ›                                                    │
│   Rows          ‹ fetched rows (1 000) ›   all rows (re-run)   selection  │
│   Separator     ‹ , comma ›                                                │
│   Header        ‹ yes ›                                                    │
│   NULL as       (empty)                                                    │
│   Encoding      ‹ UTF-8 ›                                                  │
│                                                                            │
│   → /home/viperh/exports/orders.csv  · exists, will ask before overwriting │
└─ Enter export · Tab next · ←→ change · Esc cancel ────────────────────────┘
```

**Dialog behaviour:**

- The current field is marked `› ` (T203 convention). Choices are drawn `‹ value ›`.
- Fields appear or disappear by format: Separator only for CSV/Text; Encoding only for text formats; Header for all but JSON and JSON lines.
- The last line is a live preview of the resolved path: `~` expanded, extension added, and a dim `exists` warning when the file is already there.

**Formats:**

| Format | Ext | Notes |
|---|---|---|
| CSV | csv | RFC 4180 quoting; separator choice incl. custom single char; `\r\n` optional |
| TSV | tsv | tabs/newlines in values escaped as `\t` `\n` |
| JSON | json | array of objects; NULL → `null`; numbers unquoted when `ValueKind` is numeric (T303) |
| JSON lines | jsonl | one object per line |
| Excel | xlsx | typed cells by `ValueKind` (numbers/dates/bools), bold frozen header, autofilter, column widths |
| Markdown | md | pipe table, pipes escaped |
| SQL INSERT | sql | `INSERT INTO <table>` with table name field (default from result tab name), batch size 100 rows/statement, dialect quoting (T303 `copy.rs`) |
| Text | txt | existing plain delimited (kept for compatibility) |

**Row scopes:**

- **Fetched rows:** writes what is in memory, the default. The header line shows `(all fetched)` or, when truncated, `(1 000 of more)` in amber, so truncation is never silent.
- **All rows (re-run):** starts a T305 `JobKind::Export` job that re-runs the result's SQL in its session with no row limit, streaming batches straight to the file (no full materialisation).
  - Progress shows in the jobs drawer and status line (`exporting · 120 000 rows · 3.1 s`), and the job is cancellable.
  - The dialog closes as soon as the job starts. Success or failure arrives as a toast (T205).
  - Re-running is warned about when the statement is not a `SELECT`/`WITH` (lexer check, T302): `re-running may change data` in coral, with a confirm.
- **Selection:** the selected rows or range, if the grid has one (T303), otherwise disabled.

**Overwrite.** If the target exists, Enter opens a coral confirm dialog, `orders.csv exists — overwrite?`, with `Overwrite` / `Cancel`. The write is atomic (temp file in the same dir, then rename), so a failed export never leaves a half-written file over an existing one.

**Completion feedback:**

- **Fetched-scope exports** keep today's in-dialog wait: "Exporting…", then close on success, or keep the path with an error on failure.
- **Success** shows a green toast, `✓ exported 1 000 rows to ~/exports/orders.csv`.
- **Remembered values:** the dialog remembers the last directory, format and separator per session.

### qry-core split

`crates/qry-core/src/exporter.rs` becomes a streaming writer:

```rust
pub enum ExportType { Csv, Tsv, Json, JsonLines, Excel, Markdown, SqlInsert, Text }
pub struct ExportOptions { pub separator: char, pub header: bool, pub null_as: String,
                           pub crlf: bool, pub table: Option<String>, pub dialect: Dialect, pub batch: usize }
pub trait RowSink { fn begin(&mut self, cols: &[ColumnMeta]) -> Result<()>;
                    fn rows(&mut self, rows: &[Vec<Option<String>>]) -> Result<()>;
                    fn finish(self: Box<Self>) -> Result<u64>; }
pub fn sink(etype: ExportType, path: &Path, opts: &ExportOptions) -> Result<Box<dyn RowSink>>;
pub fn resolve_path(input: &str, etype: ExportType, home: &Path) -> PathBuf; // ~, extension
```

- `ExportConfig` and `ExportType::with_extension` keep their current tests. Every existing exporter test is ported to `exporter_tests.rs`.
- `Excel` buffers in `rust_xlsxwriter`, which is in-memory by nature. When the row count exceeds 1 048 575, a clear error is returned before writing.
- The worker side (qry `db.rs` / session worker from T103) pipes `RowStream` batches (T305) into a `RowSink`.

### Edge cases

- The path is a directory, the parent is missing, or permission is denied: show a clear error naming the path, and keep the dialog open (existing behaviour, `a_failed_export_keeps_the_modal_and_the_path`).
- There is no result to export: the palette command is disabled, and `ctrl-e` gives a dim toast `nothing to export`.
- A custom separator must be a single non-quote, non-newline character.
- The result's session closes during an all-rows export: the job is Cancelled, the temp file is removed, and the original is untouched.
- Values with embedded NUL bytes in CSV are written as-is. Excel strips characters below 0x20 except tab/newline, with a warning in the toast.
- Windows paths and `~user` are not expanded (only `~` and `~/`).

### Out of scope

Exporting to clipboard (T303 copy covers it), Parquet, exporting schema DDL, scheduled exports.

## Acceptance criteria

- [x] The amber export dialog uses the T203 frame, `› ` focus marker, `‹ ›` choices, live path preview, and format-dependent fields.
- [x] All eight formats are written correctly. Excel cells are typed by `ValueKind`, with a bold frozen header and autofilter.
- [x] Header and NULL-as options apply to every format they make sense for.
- [x] Fetched-row exports of truncated results are visibly labelled. All-rows exports stream through a cancellable job, with no full materialisation (memory bounded by `fetch_size`).
- [x] An existing file prompts before overwrite. Writes are atomic: a failed or cancelled export leaves the original intact and no temp file behind.
- [x] Non-SELECT re-runs are warned about and confirmed.
- [x] The existing modal behaviours (wait, keep path on failure, close on success, ignore answers meant for other modals) still hold, with their tests ported.
- [x] Success and failure toasts show the row count and resolved path.

## Tests

**Unit tests** — `crates/qry-core/src/exporter_tests.rs`:

- The ported existing tests.
- Resolve path: `resolve_path_expands_tilde_and_adds_extension`, `resolve_path_keeps_existing_extension_case_insensitive`.
- Text formats:
  - `csv_rfc4180_quoting_and_crlf_option`
  - `csv_custom_separator_validation`
  - `tsv_escapes_tab_and_newline`
  - `json_numbers_unquoted_by_kind`
  - `jsonl_one_object_per_line`
  - `markdown_escapes_pipes`
  - `header_off_omits_first_line`
  - `null_as_applies_to_text_formats`
- SQL INSERT: `sql_insert_batches_and_quotes_per_dialect`.
- Excel: `excel_types_cells_and_freezes_header` (read back with `calamine` as a dev-dep), `excel_too_many_rows_errors_before_writing`.
- Atomicity: `atomic_write_leaves_original_on_failure` (a sink that fails mid-way).

**Proptest:** `prop_csv_export_round_trips` writes random tables with quotes, separators, newlines and NULLs, then reads them back with the `csv` crate and gets the same table, with `null_as` distinguishing NULL from empty.

**Unit tests** — `crates/qry/src/components/exportform_tests.rs`:

- Fields: `fields_change_with_format`, `preview_shows_resolved_path_and_exists_warning`.
- Scopes:
  - `truncated_result_labels_fetched_scope`
  - `all_rows_scope_emits_export_job_and_closes`
  - `non_select_rerun_requires_confirm`
- Overwrite: `existing_file_opens_overwrite_confirm`.
- Remembered values: `remembers_last_dir_and_format_per_session`.
- The ported `home.rs` export tests (`a_failed_export_keeps_the_modal_and_the_path`, `a_written_export_closes_the_modal`, `an_answer_meant_for_another_modal_is_ignored`).

**Snapshots** — plain and styled:

- `export_dialog_csv_80x24`
- `export_dialog_excel_80x24`
- `export_dialog_sql_insert_80x24`
- `export_dialog_truncated_warning_80x24`
- `export_overwrite_confirm_80x24`
- `export_job_in_drawer_120x40`

**DB integration tests** (T004 `db_or_skip!`):

- `export_all_rows_streams_without_materialising` (SQLite always; Postgres with `generate_series(1, 200000)`). Assert the file line count, and peak buffered rows ≤ `fetch_size` via a counting sink.
- `export_sql_insert_reimports_identically`, for each backend.
- `cancelled_export_removes_temp_and_keeps_original`

**Fuzz:** not applicable (no untrusted input parsed).

## Implementation notes

- **qry-core.** `ExportType` has the eight formats in the table's order, with
  `uses_separator`, `has_header`, `uses_null_as`, `is_text` and `row_limit`.
  `exporter.rs` is now the spec's streaming writer: `ExportOptions` (plus
  `bom`, for the Encoding row), `RowSink`, `sink`, `resolve_path`, and also
  `display_path` (`~/…` for toasts), `parse_separator`, `write_result` (the
  rows in hand; refuses a result over Excel's limit before opening
  anything) and `export_query` (every row: `Database::query_stream` feeds a
  writer on a blocking thread through a channel of one). Every sink writes
  to `.<name>.<pid>-<n>.part` beside the target and renames it over the
  target in `finish`; dropped before that (failure, cancel, the future
  dropped by a closed session) it removes the temporary file. The text
  writers reuse `copy.rs`'s field helpers (now `pub(crate)`). CSV quotes an
  empty string (`""`) so it reads apart from a NULL written as nothing.
  JSON writes numbers (valid JSON numbers only) and booleans unquoted by
  `ValueKind`. SQL INSERT batches `batch` rows per statement, quotes each
  part of a dotted table name, leaves out the column list with Header off,
  and uses `INSERT ALL … SELECT 1 FROM DUAL` for Oracle. Excel types numbers,
  booleans, dates, times and timestamps, bolds and freezes the header, adds
  an autofilter and column widths (first 1 000 rows), strips control
  characters below 0x20 but tab and newline (and cuts text at Excel's
  32 767), and says so through `RowSink::warning`.
- **Streaming.** `Database::query_stream(sql, fetch_size, RowSender)` and
  `StreamItem::{Columns, Rows}`. SQLite steps on its connection thread with
  `blocking_send`; Postgres reads the `simple_query_raw` stream. MySQL and
  MariaDB share `mysql::stream`, which reads `query_iter` a row at a time
  and, once the reader has gone, drains the rest off the wire (dropping
  it) so the connection stays usable. Oracle walks its cursor inside
  `with_conn` on the blocking pool, with `fetch_array_size` set to the
  fetch size, sending through `shared::blocking_send`; once the reader has
  gone the cursor is dropped with the rest unread. All three gather rows
  in `shared::Batches` (one batch at most) and write cells with the same
  `column_meta` / `cell` as `query_limited`. So every backend holds at
  most a batch plus the bounded channel's one; only the trait's default
  (used by the test fake) still reads the whole result first. A dropped
  receiver ends every backend's stream with `the rows' reader stopped`.
  Tests: `shared_tests.rs` (batching, a gone reader, `send_result`);
  `db_contract::a_streamed_result_matches_the_query` (streamed columns and
  rows equal `query`'s, in batches of 2); and in `db_export.rs`
  `a_failure_part_way_comes_after_the_rows_before_it` (a row that errors
  late, so a materialising backend would deliver nothing first) and
  `a_reader_that_goes_stops_the_stream_and_the_connection_works_on`. Only
  SQLite ran here; the server backends run in CI's containers.
- **The dialog** (`app/dialogs/exportform.rs`, the spec's
  `components/exportform.rs`): amber, 80 wide, rows Path, Format, Rows,
  Separator, Custom (for `other`), Line ends (CSV), Header, NULL as, Table
  (SQL INSERT), Encoding, laid out again on every change with hidden rows
  keeping their values. The subtitle is `result N table · R rows (all
  fetched)`, or `(R of more)` in amber (`Form::subtitle_warns`,
  `Theme::subtitle_warning`). The preview is the form's note
  (`Note::then`, `Tone::{Plain, Dim}`): the reducer emits
  `Effect::ProbeExportPath` once per path and format, the runtime resolves
  it with the real home directory and answers `FileAnswer::ExportProbed`, so
  neither the reducer nor the view touches the disk. A Rows choice of `all
  rows (re-run)` on a statement `sql::is_read_only` does not pass shows
  `re-running may change data` in coral under the row. The session
  remembers the folder of the last path (prefilled), the format and the
  separator (`SessionState::export`).
- **Flow** (`app/reducer/export.rs`). Enter asks the coral `Run Again`
  confirm first when needed, then the coral `File Exists` (`orders.csv
  exists — overwrite?`, Overwrite / Cancel) when the preview found the file;
  either one's Cancel (Esc, `n`) goes back to the dialog as it was, and its
  yes resubmits. The worker checks again (`overwrite: false` and the file
  there answers `ExportExists`, which opens the same confirm). Fetched rows
  go as `DbCommand::Export(ExportJob { source: Fetched(Arc<QueryResult>) })`
  (the tab's own result, so the worker no longer keeps `last`) and the
  dialog waits as before. All rows queue a `Queued { export }` in the
  session as a `JobKind::Export` job; the dialog closes, the worker sends
  `ExportProgress` per batch, the status line and drawer read `exporting ·
  120 000 rows · 2.8 s`, `ctrl-c` / drawer `c` cancel it (`export
  cancelled`), and closing the session cancels it before dropping the
  worker. Toasts: `exported 1 000 rows to ~/exports/orders.csv` (a warning
  with Excel's note), `could not export 7 rows to …: …` and `export to …
  failed after 3 000 rows: …`. With no result `ctrl-e` toasts `nothing to
  export` and the palette leaves `Export result…` out. The palette's
  `export csv|tsv|json|xlsx <path>` write the active tab's rows at once
  (`export tsv` is now TSV) and do not replace a file that is there.
- **Keys.** `ExportResults` is Results `ctrl-e` (was `ctrl-s`); re-accepted
  after review: the help, hints, inspector, palette and `print_config_default`
  snapshots and `dialog_export_120x40`. `export_form_80x24` (views_tests) is
  gone: `export_dialog_csv_80x24` replaces it.
- **Tests.** `exporter_tests.rs` (the ported five, every listed test,
  `prop_csv_export_round_trips` reading back with the `csv` crate, Excel
  read back with `calamine` and its XML through `zip`, all three new
  dev-dependencies; zip was in the tree already). `tests/db_export.rs` has
  the three DB tests per backend (SQLite ran here; the servers did not).
  `exportform_tests.rs` has the listed form and harness tests, the ported
  `home.rs` ones, cancel, close-session, failure and warning toasts, and the
  six snapshots plain and styled (the harness answers probes itself from
  `with_existing_file`, `~` being `/home/test`). `db_tests` and
  `runtime_tests` cover the worker and the probe.
- **Not done.** The `selection` scope: the grid has a cell cursor and no
  row or range selection (T303), so the choice is not offered. The Rows
  choice shows its value only (`‹ fetched rows ›`), not the mockup's three
  options side by side, since the T203 form draws one value. The footer
  reads `→ change` (one action per hint). `\r\n` is a Line ends row for CSV
  only. No pty run of the real binary was made.
- **Checks run locally:** `cargo fmt --all`, clippy with `-D warnings`,
  `QRY_KEYRING=off QRY_DB_TESTS=skip CI=true cargo test --workspace
  --all-features --locked` and the CI docs command.

Verification (later):
- **Servers:** `tests/db_export.rs` (all rows streamed, SQL INSERT re-imported, cancel) passed against PostgreSQL 16.15 (`initdb`, scram-sha-256), MariaDB 10.11.14 (`mariadb-install-db`) and MySQL 8.0.46 (Ubuntu's `mysql-server-core-8.0` package unpacked with `dpkg -x`, `--initialize-insecure`, `caching_sha2_password` over TCP), all started by hand under `/tmp/qry-verify-*` and removed afterwards. No Oracle: there is no Docker daemon, and Oracle Free has no package that runs without one here.
- **Terminal run:** the real binary was driven under tmux, 120×40, with `QRY_CONFIG`/`QRY_DATA` in temp dirs and `QRY_KEYRING=off`. In the scratch SQLite:
  - `ctrl-e` on a 3-row result opened the dialog, and exporting to `/tmp/…/out.csv` wrote `a,b` and the three rows, with NULL as empty.
  - The second open prefilled the folder.
  - Excel with `all rows (re-run)` wrote `all.xlsx` through a job and toasted `exported 3 rows to …`.
  - Nothing was broken. One behaviour to note: with a `.xlsx` path and the JSON lines format, the preview reads `all.xlsx.jsonl`, because the extension is appended rather than replaced.
- The fourth box is unchanged: MySQL, MariaDB and Oracle still use the materialising default `query_stream`, which another task is replacing.
