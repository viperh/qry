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

- [ ] The connection dialog shows only the fields relevant to the selected driver. Switching drivers keeps shared values and handles the default port as specified.
- [ ] Inline validation blocks submit and names the field, before any connect attempt.
- [ ] `ctrl-t` tests a connection without saving or adding it to the tree, and shows the latency and server version or the error inline. The password is zeroized afterwards.
- [ ] Label `prod`/`staging`/`dev` is saved and shown coral/amber/teal in the tree and top-bar tab. `prod` asks before the first write statement per session (configurable).
- [ ] Duplicate creates a new id, with the password not copied.
- [ ] Delete asks in a coral confirm, with keychain and history options, and disconnects an open session first.
- [ ] Import shows a preview with new, already-here and skipped entries, selectable. Confirm saves once and toasts a summary.
- [ ] Export connections lets you choose which to include, confirms overwrite, writes atomically, and never includes passwords.
- [ ] All existing guarantees hold and keep their tests: a password never appears in any `Action` or `Effect` (a test that scans `Debug` of every emitted effect for the canary), id minted once across retries, modal waits and keeps fields on failure, `SecretNotStored` fallback, `ForgetSecret` on delete or storage change.
- [ ] Old `connections.json` files load unchanged. A round trip through the new version preserves unknown keys.

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
