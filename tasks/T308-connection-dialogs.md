# T308 — Connection management dialogs

**Phase:** 3 — Features · **Depends on:** T203, T103 · **Blocks:** —

## Description

### Current state

**New/Edit connection form.** `ctrl-n` (`Mode::AddConnModal`) opens `ConnForm` (`crates/qry/src/components/connform.rs`). Edit is `e` in the tree, through `Action::EditConnection`.

- It is a fixed list of 11–12 fields: Name, Type `‹PostgreSQL|MySQL|MariaDB|SQLite|Oracle›`, Host, Port, User, Password (masked), Database, Schema, SSL mode, Wallet ("Oracle only: …" placeholder), Password in `‹KEYCHAIN|ASK EACH TIME|ENVIRONMENT›`, and Variable (ENVIRONMENT only).
- Most fields are shown whether or not they apply to the driver. SQLite still shows Host and Port.
- Submitting sends `Action::Connect` and the modal waits on `Awaiting::Connect`. It closes on `Connected`, or keeps everything on `ConnectFailed`.
- The form mints its `id` once, so retries do not duplicate keyring entries.
- A typed password goes through `secrets::stash`, **never in an action**.
- When the keyring refuses, `Action::SecretNotStored` falls back to prompting.

**Delete.** `Del` in the tree deletes immediately, with no confirmation. It sends `ForgetSecret` for keyring secrets.

**Import and export.**

- Import (`ctrl-o`, `ImportForm`) takes Path plus Format `‹Detect|qry backup|SQL Developer|DBeaver›` (`crates/qry/src/import.rs`). Passwords are never imported.
- The result is summarised in a status-bar message: added N, "already here", `skipped name: reason`, plus warnings.
- Export connections (`ctrl-s`, `ConnExportForm`) writes a qry backup.

**Visual and missing features:**

- All forms share `form.rs`: a bordered popup, with each field in its own bordered box (cyan when focused, dark grey otherwise).
- There is no "test connection", no duplicate, and no way to mark a dangerous connection.

### Target design

All dialogs use the T203 `dialog_frame`, the `› ` field marker, `‹ value ›` choices, and footer hints (`Enter save · Tab next · ←→ change · ctrl-t test · Esc cancel`). The existing guarantees are **kept and re-tested**:

- Password never in an action.
- id minted once.
- Wait for the worker before closing, and keep fields on failure.
- `SecretNotStored` fallback.
- `ForgetSecret` on delete or secret-store change (the `ConnTree::attempt` logic).

#### Connection dialog (new / edit / duplicate)

```
┌ New connection ─────────────────────────────────────────────── postgres ┐
│                                                                          │
│   Driver        ‹ PostgreSQL ›                                           │
│ › Name          local-app▏                                               │
│   Host          localhost                       Port   5432              │
│   User          app                                                      │
│   Password      ••••••••                         ‹ keychain ›            │
│   Database      app                              Schema  public          │
│   SSL           ‹ prefer ›                                               │
│   Label         ‹ none ›   (none · dev · staging · prod)                 │
│                                                                          │
│   ✓ connected in 42 ms · PostgreSQL 16.4                                 │  ← test result (green / coral)
└─ Enter save & connect · ctrl-t test · Tab next · ←→ change · Esc cancel ─┘
```

**Dynamic fields** per driver, defined by a table in qry-core so it is testable:

| Driver | Fields |
|---|---|
| PostgreSQL | Host, Port (5432), User, Password, Database, Schema, SSL mode (6 modes) |
| MySQL / MariaDB | Host, Port (3306), User, Password, Database, SSL mode (disable/prefer/require/verify-ca/verify-full) |
| SQLite | File path (with `~` expansion and an "exists" hint), Mode `‹ read-write / read-only / create ›` (`mode`: `rw`/`ro`/`rwc`). No password. |
| Oracle | Host, Port (1521), User, Password, Service/SID, Schema, SSL mode, Wallet dir (shown only when SSL ≠ disable) |
| any | "Use DSN instead" toggle → a single DSN field (`StoredConnection.dsn`) |

Further dialog behaviour:

- **Driver change:** switching drivers keeps values in fields that exist in both, and resets the default port only if the port was still the old default.
- **Password storage** `‹ keychain / ask each time / environment ›` sits next to the password field. "Environment" swaps the password field for a Variable field, `QRY_PW_LOCAL_APP` by default, generated from the name.
- **Label** (new field, stored in `StoredConnection.extra` → promoted to `label: Option<ConnLabel>`):
  - `prod` draws the session tab in the top bar (T202) and the tree row with a coral `[prod]` tag.
  - Running a non-SELECT statement on a `prod` connection asks for confirmation once per session (amber confirm). This can be turned off with config `confirm_writes_on_prod = false`.
  - `staging` is shown amber and `dev` teal.
- **Test connection** (`ctrl-t`, or a `[ Test ]` button reachable by Tab, drawn with `button` / `button_primary`):
  - Connects through a **throw-away** session that is not added to the tree and not saved, then disconnects.
  - It shows the result inline: green `✓ connected in 42 ms · <server version>` or coral `✗ <error>`.
  - The typed password is used through the stash and then **zeroized**. It is not stored.
- **Validation** runs inline, per field (coral message under the field), before any connect attempt:
  - Name required and unique among saved connections.
  - Port 1–65535.
  - SQLite path non-empty.
  - Environment variable name `[A-Z_][A-Z0-9_]*`.
- **Duplicate** (`c` in the tree, or the palette): opens the dialog pre-filled with `name (copy)` and a **new id**. The password is not copied; it is set to prompt.

#### Delete confirm

A coral confirm dialog (T203):

```
┌ Delete connection ─────────────────────────────┐
│ Delete "prod-db"? This cannot be undone.        │
│ [x] also remove its password from the keychain  │
│ [x] also delete its query history (T306)        │
│                                                 │
│            [ Delete ]   [ Cancel ]              │
└─ Enter confirm · Space toggle · Esc cancel ─────┘
```

- An open session for it is disconnected first, with the open-transaction check from T305.
- The keychain option is shown only for `Secret::Keyring`.

#### Import

The flow becomes **pick → preview → confirm**, instead of applying immediately:

1. **Pick:** a dialog with Path (`~` expansion) and Format `‹ Detect / qry backup / SQL Developer / DBeaver ›`. Enter parses the file (off the event loop, as today).
2. **Preview:** a teal dialog listing everything the file held:
   - `+` new connection, in green. These are selected by default.
   - `=` already here, dim. Matched with `same_target` or the id, as today.
   - `!` skipped, in coral, with its reason.
   - Format-level warnings below the list.
   - `Space` toggles inclusion of a new connection.
   - A note: `passwords are never imported; each connection will ask on first connect`.
3. **Confirm:** Enter adds the selected connections and saves once. A toast reports `imported 5 · 2 already here · 1 skipped`.

The long multi-part status message that `ConnTree::import` builds today is replaced by the toast and the preview.

#### Export connections

- Moved to the palette command "Export connections…", because `ctrl-s` goes to snippets (T306).
- Its dialog has Path and an "include" list (all selected by default) with Space toggles.
- The overwrite confirm and atomic write come from T307.
- Its note: `passwords are not included`.

### qry-core / qry split

**qry-core:**

- `qry_core::connspec`: the per-driver field table (`fn fields(driver) -> &'static [FieldSpec]`), defaults, and validation (`validate(&Draft) -> Vec<FieldError>`). It is pure and has no IO.
- `Database::server_version()` is added for the test-connection message.

**qry:**

- `StoredConnection` stays in qry with its serde shape. New fields (`label`) are added with `#[serde(default, skip_serializing_if…)]`, so files from older versions load and older versions keep unknown keys through the `extra` flatten (existing behaviour).
- Dialog state machines are pure reducers (T101). Effects are `TestConnection { draft_id }`, `Connect`, `SaveConnections`, `ForgetSecret`, `DeleteHistory`, `ReadImport`, `WriteConnections`.

### Edge cases

- Editing a connection while its session is open: saving reconnects that session, after confirming if a transaction is open.
- Switching password storage from keychain to prompt sends `ForgetSecret` (existing behaviour). Switching to keychain stores it on the next successful connect.
- The keychain is unavailable (no Secret Service on headless Linux): `SecretNotStored` → a dim toast `keychain unavailable; qry will ask each time`, and the record becomes `Prompt`.
- A test connection is still in flight when the dialog is cancelled: its answer is ignored (a generation id, like `Awaiting` today).
- Import of a 500-connection DBeaver file: the preview list scrolls, and `a`/`A` select or deselect all.
- An import file that is not valid JSON or XML: coral error in the pick dialog, keeping the path (existing behaviour, `an_import_waits_for_the_file_and_keeps_the_path_on_failure`).

### Out of scope

SSH tunnels, cloud IAM auth, importing passwords, connection groups/folders (a follow-up).

## Acceptance criteria

- [x] The connection dialog shows only the fields relevant to the selected driver. Switching drivers keeps shared values and handles the default port as specified.
- [x] Inline validation blocks submit and names the field, before any connect attempt.
- [x] `ctrl-t` tests a connection without saving or adding it to the tree, and shows the latency and server version or the error inline. The password is zeroized afterwards.
- [x] Label `prod`/`staging`/`dev` is saved and shown coral/amber/teal in the tree and top-bar tab. `prod` asks before the first write statement per session (configurable).
- [x] Duplicate creates a new id, with the password not copied.
- [x] Delete asks in a coral confirm, with keychain and history options, and disconnects an open session first.
- [x] Import shows a preview with new, already-here and skipped entries, selectable. Confirm saves once and toasts a summary.
- [x] Export connections lets you choose which to include, confirms overwrite, writes atomically, and never includes passwords.
- [x] All existing guarantees hold and keep their tests: a password never appears in any `Action` or `Effect` (a test that scans `Debug` of every emitted effect for the canary), id minted once across retries, modal waits and keeps fields on failure, `SecretNotStored` fallback, `ForgetSecret` on delete or storage change.
- [x] Old `connections.json` files load unchanged. A round trip through the new version preserves unknown keys.

## Tests

**Unit tests** — `crates/qry-core/src/connspec_tests.rs`:

- Field table: `fields_per_driver_match_table`, `sqlite_has_no_password_or_host`, `oracle_wallet_only_with_ssl`.
- Validation:
  - `validate_name_required_and_unique`
  - `validate_port_range`
  - `validate_env_var_name`
  - `validate_sqlite_path_required`
- Driver switching: `driver_switch_keeps_shared_fields`, `driver_switch_updates_default_port_only_if_untouched`.

**Unit tests** — `crates/qry/src/components/conndialog_tests.rs` (ported from `connform.rs` tests plus new):

- Guarantees:
  - `password_never_in_emitted_actions` (canary `CANARY-PW-…` scanned in the `Debug` output of every effect)
  - `id_minted_once_across_retries`
  - `failed_connect_keeps_fields`
  - `secret_not_stored_falls_back_to_prompt_with_toast`
  - `switch_keyring_to_prompt_forgets_secret`
- Test connection: `ctrl_t_emits_test_effect_not_connect`, `late_test_answer_after_cancel_ignored`.
- Labels and duplication: `prod_label_confirms_first_write_once`, `duplicate_gets_new_id_and_prompt_secret`.

**Unit tests** — `crates/qry/src/components/deleteconfirm_tests.rs`:

- `cancel_is_noop`
- `confirm_forgets_keyring_and_deletes_history_when_checked`
- `unchecked_keeps_history`
- `open_session_disconnected_first`

**Unit tests** — `crates/qry/src/components/importdialog_tests.rs`:

- `preview_classifies_new_existing_skipped`
- `space_toggles_selection`
- `a_selects_all`
- `confirm_adds_selected_and_saves_once`
- `parse_error_keeps_path`
- The ported `an_import_waits_for_the_file_and_keeps_the_path_on_failure`.

**Integration tests** — `crates/qry/tests/connections_compat.rs`:

- `old_files_load` (fixtures from the current version in `tests/fixtures/connections/v0_1.json`)
- `unknown_keys_round_trip`
- `label_round_trip`

**Snapshots** — plain and styled:

- `conndialog_postgres_100x30`
- `conndialog_sqlite_100x30`
- `conndialog_oracle_tls_100x30`
- `conndialog_env_password_100x30`
- `conndialog_validation_errors_100x30`
- `conndialog_test_ok_100x30`
- `conndialog_test_failed_100x30`
- `delete_confirm_80x24`
- `import_preview_120x40`
- `export_connections_dialog_100x30`
- `tree_and_tab_prod_label_120x40`

**AppHarness flow:** `h.keys("ctrl-n")`, type the fields for the fake backend, `h.keys("ctrl-t")`. Assert the inline ✓. Then `h.keys("enter")`: the session opens and the tree shows the new connection with a green dot.

**DB integration test** (T004): `test_connection_reports_server_version`, for each backend.

**Fuzz:** the import parsers already parse untrusted files. T005 owns the `import_qry`, `import_sqldeveloper` and `import_dbeaver` targets; this task must keep them building.

## Implementation notes

- **What is not verified against a real server.** The per-backend DB integration test `test_connection_reports_server_version` waits for T004's service containers; `runtime_tests::test_connection_reports_server_version` covers it on a real SQLite `:memory:` through the executor, `a_test_takes_its_password_from_the_stash_and_leaves_none_behind` a refused Postgres connect, and the reducer side runs on the harness's fake (which answers `Effect::TestConnection` in 42 ms with `FakeDatabase::version`). `Database::server_version()`'s default queries (`current_setting('server_version')`, `VERSION()`, `v$version`) were not run against Postgres, MySQL, MariaDB or Oracle. No manual pty run was made.
- **Out-of-scope parts left as hooks.** The delete confirm's "also delete its query history" emits `Effect::DeleteHistory(id)`, which the runtime only debug-logs until T306 keeps a history. Closing a session on delete does not run T305's open-transaction check (there is no such confirm yet), and neither does editing a connection whose session is open (it reconnects, as before). The overwrite confirm and atomic write are this task's own, since T307 has not landed: the runtime answers `FileAnswer::ConnectionsExportExists` when the target exists and `overwrite` is false, the reducer opens `Confirm::overwrite_connections`, and its yes sends the export again with `overwrite: true`; `connections::export` still refuses to replace a file that is not a qry backup. The SQLite "file exists" hint is not shown: checking would be I/O in the view or the reducer (`purity_tests`). `~` is expanded in `StoredConnection::to_config`.
- **qry-core.** `connspec.rs`: `Engine` (with `default_port`, `needs_password`, `LABELS`), `FieldKind` (the table's fields plus `Name`, `Dsn` and `EnvVar`, so errors can name any row), `FieldSpec { kind, default }`, `fields(engine)`, `ssl_modes(engine)` (MySQL and MariaDB have no `allow`), `SQLITE_MODES`, `PasswordStore`, `Draft`, `visible(draft)` (the DSN alone, the Variable in place of the Password, the wallet only when SSL ≠ disable), `switch_engine` (shared values stay, a choice the new engine lacks resets, the port moves only if it was empty or the old default), `default_env_var` (`QRY_PW_LOCAL_APP`), `is_env_var_name` and `validate(draft, taken)`. The spec's `validate(&Draft)` takes the other names as a second argument. Validation also requires the host, the user, Postgres's database and Oracle's service, which the worker required anyway. `Database::server_version()` (a default method per `DatabaseType`, overridden by `FakeDatabase`) and `Driver::server_version()`. `sql::is_read_only` decides what the prod confirm counts as a write: `SELECT`, `SHOW`, `EXPLAIN`, `DESCRIBE`, `VALUES`, `WITH` without `INSERT`/`UPDATE`/`DELETE`/`MERGE`, and transaction control are reads; anything else is a write.
- **The dialog.** `app/dialogs/conndialog.rs` replaces `connform.rs` (the spec's `components/…` paths are `app/dialogs/…` since T101). Rows are `Key::{Driver, Name, UseDsn, Conn(FieldKind), Store, Label, Test}`, rebuilt by `relayout` after every change. Rows a choice hides keep their values, so turning the DSN off brings the fields back. One field per row: the mockup's second column (Port beside Host, the store beside Password) is not drawn, as the T203 form view has one column; `Password in ‹ keychain / ask each time / environment ›` follows the Password row. A `[ Test ]` button (`Field::Button`, `button` / `button_primary`) is the last row, and Enter on it tests. The record keeps an edited record's `extra` keys, leaves out the port when it is the driver's default, writes SQLite's mode only when it is not `rw`, and stores `label`. The id is minted when the dialog opens, so retries reuse it. Duplicate copies the record under a new id as `name (copy)`; a keychain password becomes "ask each time" and nothing is stashed.
- **Form framework changes (T203's `form.rs` and `views/form.rs`).** `Form::labels()` returns `&[&'static str]` borrowed from the form, for rows that change. New: `field_error(i)` (drawn coral on its own row under the field; the view scrolls so the focused field and its error show), `note()` (`Note { text, tone: Ok | Error | Busy }`, a line under the fields: the test's outcome) and `check(&mut self)` (the reducer calls it before `submit`; the connection dialog marks every bad field, moves to the first and puts `Field: message` in the footer). `Field::Checkbox`'s caption is a `String` (the export list's names), `Field::apply` is `pub(crate)`, and `Field::Button` is new. The `allow(dead_code)`s naming T308 are gone.
- **Test connection.** `Action::TestConnection` (Dialog `ctrl-t`, key-only, so it joins the palette test's exclusions). `reducer::connections::test` validates everything but the name, says "type the password" without any effect when there is nothing to connect with, and otherwise bumps `State::tests_made`, sets `TestState::Running(n)` and emits `StashPassword { connection: test_stash_key(id) }` (`test:<id>`, so a connect cannot take it) then `Effect::TestConnection { test: n, record }`. The executor takes the stash at once and spawns `db::test_connection`, which resolves the password (typed, environment, keychain, never a prompt), connects a fresh `Driver`, times it, asks the version and drops both; the password (`Zeroizing`) is cleared as it drops, and the error is redacted and cut to its first line. `UiEvent::ConnectionTested { test, outcome: Result<Tested, String> }` only lands on an open connection dialog whose test is still `Running(test)`, so an answer after Esc, or after a newer test, is ignored.
- **Labels.** `ConnLabel { Dev, Staging, Prod }` and `StoredConnection.label` (`serde(default, skip_serializing_if)`). `Theme::conn_label` (coral, amber, teal; bold) draws `[prod]` after the name in the tree and in the session's top-bar tab (`SessionState::label`, also refreshed by `retarget`). `run::enqueue` holds a run with a write on a prod session in `SessionState::held` behind `Confirm::run_on_prod` (amber, starts on Run): yes sets `writes_confirmed` and runs it, anything else drops it. Config `confirm_writes_on_prod` (default `true`) is in `.config/config.json`, `Config` and `qry --print-config`.
- **Delete.** `deleteconfirm::delete_connection` builds a destructive `Confirm` with checkboxes (`Confirm::with_check`, `Check::{ForgetSecret, DeleteHistory}`; the keychain one only for `Secret::Keyring`). With checkboxes, Tab / ↑ / ↓ move between them, Space ticks, ← / → pick the button, and `Enter confirm · Space toggle · Esc cancel` goes in the bottom border (`views::confirm::draw` takes the keymap now). `OnConfirm::DeleteConnection { id, forget_secret, delete_history }`; `connections::delete` closes every session on the connection first, then forgets it, emits `ForgetSecret` and `DeleteHistory` as ticked, and saves. `tree.rs` lost `delete`, `import` and `export_connections` (now in `reducer/connections.rs`), and `save` and `forget_node` became `pub(super)`; its import/export tests went with them.
- **Import.** `FileAnswer::Imported` now opens `DialogState::ImportPreview` (`importdialog.rs`, drawn by `views/import_preview.rs`, teal): `+` new (selected), `=` already here (same id or `same_target`, or a repeat within the file), `!` skipped with its reason, then the warnings and the passwords note. ↑ / ↓ / PgUp / PgDn / Home / End move (new Dialog bindings for ↑ ↓ PgUp PgDn), Space toggles a new one, `a` / `A` select all or none, and the list scrolls. Enter adds the selection, saves once and toasts `imported N · M already here · K skipped` (a warning when something was skipped). The old multi-part status message is gone.
- **Export.** `ConnExportForm::new(&[(id, name)])`: the file, then a checkbox per saved connection, all ticked; subtitle `passwords are not included`; `Submission::ExportConnections { path, ids }`. Tree `ctrl-s` is unbound (T306's snippets get it), and the palette calls it `Export connections…`.
- **SecretNotStored.** The worker no longer sends its own error notice; the reducer sets the record to `Prompt`, saves, and shows an info toast `keychain unavailable; qry will ask each time`.
- **Actions and keys.** New `DuplicateConnection` (Tree `c`, palette `Duplicate connection`) and `TestConnection` (Dialog `ctrl-t`); Dialog also gained ↑ ↓ PgUp PgDn; Tree `ctrl-s` was removed. Re-accepted after review: `print_config_default`, the help snapshots, the palette snapshots, every New Connection snapshot, `confirm_delete_80x24` and `screen_toast_over_dialog_120x40`.
- **Test names that differ from the list.** `purity_tests` rejects the word `keyring` anywhere under `src/app/`, tests included, so `switch_keyring_to_prompt_forgets_secret` is `switch_keychain_to_prompt_forgets_secret` and `confirm_forgets_keyring_and_deletes_history_when_checked` is `confirm_forgets_keychain_secret_and_deletes_history_when_checked`. The guarantee, test-connection, label and duplicate tests, the harness flow and the conndialog snapshots (plus `tree_and_tab_prod_label_120x40`) are in `app/dialogs/conndialog_tests.rs`; the delete tests and `delete_confirm_80x24` in `deleteconfirm_tests.rs`; the import tests and `import_preview_120x40` in `importdialog_tests.rs`; `export_connections_dialog_100x30` in `connexportform_tests.rs`; the export and overwrite flow in `app/reducer/connections_tests.rs`; the executor's side in `runtime_tests.rs`. `crates/qry/tests/connections_compat.rs` reads `tests/fixtures/connections/v0_1.json` (the 0.1 shape, with a file-level and a record-level unknown key); `connections::{parse, load_from, save_to}` became `pub` for it. `conndialog::test_support::{fill, type_into, dialog}` fill the dialog by row for other modules' tests.
- **Fuzz.** There is no `fuzz/` directory yet (T005), and `import.rs` is unchanged.
- **Checks run locally:** `cargo fmt --all`, clippy with `-D warnings`, `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked` (435 `qry` unit tests, 104 in `qry-core`, 3 in `tests/connections_compat.rs`) and the CI docs command. The shared disk was full, so builds ran with `CARGO_PROFILE_DEV_DEBUG=0`.

Verification (later):
- **`server_version` on servers:** `crates/qry-core/tests/db_runtime.rs::server_version_names_the_server` passed against PostgreSQL 16.15 (`initdb`, scram-sha-256), MariaDB 10.11.14 (`mariadb-install-db`) and MySQL 8.0.46 (Ubuntu's `mysql-server-core-8.0` package unpacked with `dpkg -x`, `--initialize-insecure`, `caching_sha2_password` over TCP), all started by hand under `/tmp/qry-verify-*` and removed afterwards. No Oracle: there is no Docker daemon, and Oracle Free has no package that runs without one here. The answers were `PostgreSQL 16.15 (Ubuntu 16.15-0ubuntu0.24.04.1)`, `MySQL 8.0.46-0ubuntu0.24.04.4`, `MariaDB 10.11.14-MariaDB-0ubuntu0.24.04.1` and `SQLite 3.53.2`. Oracle's `v$version` query has not run here.
- **Terminal run:** a temp `connections.json` held a Postgres record with `secret: {"mode": "env"}`, and the real binary was driven under tmux.
  - It connected with Enter (a second session tab, `postgres · qry@127.0.0.1`) and browsed the schema.
  - `SELECT pg_sleep(20)` was cancelled with `ctrl-c` (`query cancelled`), and the connection kept working.
  - Help (F1), the palette (`ctrl-p`), the jobs drawer (`J`) and quitting (`ctrl-q`, exit 0) behaved, and nothing was written to stderr.
  - The connection dialog's Test button was not exercised there.
