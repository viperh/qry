# T301 — Schema browser

**Phase:** 3 — Features · **Depends on:** T103, T202 · **Blocks:** T302 (completion), T304 (describe)

## Description

### Current state

`crates/qry/src/components/conntree.rs` draws a flat, two-level list:

- **Rows:** each `Node` (a `StoredConnection` plus `label`, `expanded`, `tables: Option<Vec<String>>`, `loading`, `unsaved`) is drawn as `▸ name` / `▾ name`, with plain table names indented four spaces underneath.
- **Notes:** `Row::Note` covers `loading…`, `(no tables)` and `(not connected)`.
- **Active connection:** shown by **bold** text only. There is no colour, driver indicator or connection state.
- **Expanding** a connection that is not the active one sends `Action::Connect` and sets `expand_when_connected`. Only one connection can be open, because the worker in `db.rs` owns a single `qry_core::Driver`.
- **Tables** come from `Database::tables()` (`crates/qry-core/src/shared.rs:54`). The default implementation runs `information_schema` / `sqlite_master` SQL and `bail!`s for Oracle. `Oracle` overrides it (`oracle.rs:371`) with `ALL_TABLES UNION ALL ALL_VIEWS` for `CURRENT_SCHEMA`. The result is a `Vec<String>`, so tables and views are indistinguishable. There are no schemas, columns or indexes.
- **Keys** (`TreeCommand` in `keymap.rs`): Up/Down/First/Last, Toggle (space), Connect (enter), Edit (`e`), Delete (`Del`, immediate, no confirmation).
- **Delete** drops the node, sends `ForgetSecret` if the secret was in the keyring, and saves.

### Target design

The tree becomes a real schema browser over every open session (T103), drawn in the tachy style. It has no box: a title row on `surface`, and an amber bold title when focused.

```
 CONNECTIONS                       ← title row (surface); amber bold when focused
 ● pg  prod-db            [prod]   ← green dot = connected, coral tag from T308
 ▾ ● pg  local-app
   ▾ public
     ▾ Tables  12
       ▸ users
       ▾ orders
         ▾ Columns
             id          int8        PK
             user_id     int8        FK → users.id
             total       numeric     NULL
             created_at  timestamptz
         ▸ Indexes  2
     ▸ Views  3
   ▸ audit
 ▸ ○ sq  scratch :memory:          ← hollow dot = not connected
 ▸ ✗ or  billing                   ← coral ✗ = last connect failed
 / ord▏                            ← filter line (amber prompt), matches in amber
```

**Node hierarchy:**

```
Connection → Catalog (MySQL/MariaDB database, Postgres schema, Oracle owner; SQLite has one "main")
           → Folder (Tables, Views; Functions later)
           → Object (table or view)
           → Folder (Columns, Indexes)
           → Leaf (column or index)
```

Rules for the hierarchy:

- A connection's own schema/database comes first and is expanded on first open.
- System schemas (`pg_catalog`, `information_schema`, `mysql`, `performance_schema`, `sys`, Oracle `SYS`/`SYSTEM`/...) are hidden behind a "show system objects" toggle (`.`).
- Folders show a dim item count once loaded.

**Lazy loading.** Every expandable node loads its children on first expand through a T101 effect, `Effect::LoadChildren { session, node: NodePath }`.

- While loading, a teal braille spinner (`⠋⠙⠹…`, 100 ms frames on the T101 virtual clock) is drawn after the node label.
- A failure turns into a coral `! error text` child row that can be retried with `r`.
- Results are cached per session in a `SchemaCache`. The cache is invalidated by `r` (refresh the node) or `R` (refresh the whole connection), and after a DDL statement completes in that session (`CREATE`/`ALTER`/`DROP` detected via the T302 lexer).

**Styling**, all through T201 helpers:

- Driver badge (`pg`, `my`, `ma`, `sq`, `or`) in teal dim.
- Connection name in fg; selected row in `selection_bg`. The active session's connection is bold, with an amber gutter `▌`.
- Folder labels in `fg_dim`, counts in dim.
- Column type in `fg_muted`. Markers: `PK` amber, `FK → t.col` purple, `NULL` dim.
- Filter matches use `match_highlight` (amber on amber_tint).

**Filter (`/`).**

- Opens a one-line prompt at the bottom of the tree (`/ ` amber, block cursor).
- It filters the **loaded** nodes case-insensitively by substring, keeping the ancestors of every match.
- `Enter` keeps the filter and returns to navigation. `Esc` clears it.
- The filter never triggers loads. A note `filter covers loaded objects only` is shown dim when some nodes are unloaded.

**Actions** (Tree context, T102):

| Key | Action |
|---|---|
| `↑ ↓ k j`, `Home End`, `PgUp PgDn` | move |
| `→ l` / `← h` | expand / collapse (collapse on a child jumps to the parent) |
| `space` | toggle |
| `Enter` on a connection | connect (opens a session, T103) or switch to its session |
| `Enter` on a table/view | append `SELECT * FROM <qualified, quoted name>;` to the session's editor as a new statement, and run it with the session row limit (T305) |
| `s` | same as Enter, but inserts without running |
| `d` | describe: focus the inspector (T304) on the table's columns |
| `y` | copy the qualified name (OSC 52, via the T303 clipboard helper) |
| `r` / `R` | refresh node / connection |
| `n` | new connection (T308) |
| `e` | edit connection (T308) |
| `D` / `Del` | delete connection, after a coral confirm dialog (T203) |
| `x` | disconnect session |
| `.` | toggle system objects |
| `/` | filter |

Identifier quoting is per dialect: `"x"` for Postgres, SQLite and Oracle, `` `x` `` for MySQL and MariaDB. A name is quoted only when needed (not a lowercase plain identifier for Postgres, not uppercase for Oracle). This lives in `qry_core::sql::quote_ident(dialect, name)`.

### qry-core split

New module `crates/qry-core/src/schema.rs`:

```rust
pub struct Catalog { pub name: String, pub is_default: bool, pub is_system: bool }
pub enum ObjectKind { Table, View, MaterializedView }
pub struct DbObject { pub catalog: String, pub name: String, pub kind: ObjectKind }
pub struct Column {
    pub name: String, pub data_type: String, pub nullable: bool,
    pub primary_key: bool, pub references: Option<(String, String)>, // (table, column)
    pub default: Option<String>, pub ordinal: u32,
}
pub struct Index { pub name: String, pub columns: Vec<String>, pub unique: bool }
```

`Database` gains these methods (`shared.rs`):

```rust
async fn catalogs(&self) -> Result<Vec<Catalog>>;
async fn objects(&self, catalog: &str) -> Result<Vec<DbObject>>;
async fn columns(&self, catalog: &str, object: &str) -> Result<Vec<Column>>;
async fn indexes(&self, catalog: &str, object: &str) -> Result<Vec<Index>>;
```

- They use per-backend SQL, with every name passed as a bind parameter, never interpolated.
  - **Postgres:** `pg_namespace` / `information_schema.tables` / `information_schema.columns` plus `pg_constraint` for PK/FK, and `pg_indexes`.
  - **MySQL/MariaDB:** `information_schema.SCHEMATA` / `TABLES` / `COLUMNS` (`COLUMN_KEY`) / `KEY_COLUMN_USAGE` / `STATISTICS`.
  - **SQLite:** `sqlite_master`, `pragma_table_info(?)`, `pragma_foreign_key_list(?)`, `pragma_index_list(?)`.
  - **Oracle:** `ALL_USERS` (owners with objects), `ALL_TABLES` / `ALL_VIEWS` / `ALL_MVIEWS`, `ALL_TAB_COLUMNS`, `ALL_CONSTRAINTS` + `ALL_CONS_COLUMNS`, `ALL_INDEXES` + `ALL_IND_COLUMNS`.
- Postgres currently uses `simple_query`, which cannot bind. These calls use the extended protocol (`client.query(sql, &[&catalog])`).
- `tables()` is kept as a thin wrapper over `objects(default catalog)` until T103 removes its last caller.
- `SchemaCache` (pure, in qry-core) holds the loaded tree per session, with `invalidate(path)` and `names_for_completion()`, which T302 uses.

The tree component in qry holds only view state: expanded set, selection, offset, filter text. It renders from `SchemaCache`, and it is a pure reducer slice under T101.

### Edge cases

- A connection with thousands of tables: render only visible rows (virtualised over a flattened `Vec<VisibleRow>` computed on change, not on every frame).
- Names containing spaces, quotes or unicode, and names that need quoting. Mixed-case Oracle names.
- A session that disconnects while a load is in flight: discard the late answer by `(SessionId, generation)`.
- A failed load must not wipe children that were already loaded.
- A permission error on one catalog is shown only on that catalog.
- The SQLite `:memory:` scratch session works like the others.

### Out of scope

Functions, procedures, sequences, triggers (a follow-up adds a Functions folder), ER diagrams, editing table structure.

## Acceptance criteria

- [ ] The tree shows connection → catalog → Tables/Views → object → Columns/Indexes for all five backends. Oracle no longer `bail!`s anywhere in the schema path.
- [ ] Each node loads lazily on first expand, shows a teal spinner while loading, and shows a coral error row on failure. `r` retries the failed node only.
- [ ] Columns show type, a PK marker (amber), an FK target (purple) and nullability (dim), for every backend that exposes them.
- [ ] System schemas are hidden by default and shown with `.`.
- [ ] Connection rows show a driver badge plus a state glyph: `●` green connected, `○` dim not connected, `✗` coral failed, spinner while connecting.
- [ ] `/` filters loaded nodes and keeps their ancestors. Matched characters use `match_highlight`. `Esc` clears the filter.
- [ ] `Enter` on a table inserts a correctly quoted `SELECT * FROM …` and runs it under the row limit. `s` inserts without running.
- [ ] `Del`/`D` opens a coral confirm dialog. Confirming removes the connection, forgets its keyring secret and saves (the current `ForgetSecret` behaviour is kept). Cancelling changes nothing.
- [ ] A DDL statement run in a session invalidates that session's cache. The next expand reloads.
- [ ] All metadata queries bind names as parameters. A test with a hostile name (`x'; DROP TABLE t; --`) proves nothing is interpolated.
- [ ] No raw colours (the `no_raw_colors` test passes). All styles come from `Theme`.
- [ ] Moving through 10 000 loaded tables stays under the T006 render budget (bench `tree_render_10k`).

## Tests

**Unit tests** — `crates/qry-core/src/schema_tests.rs`:

- `cache_invalidate_drops_subtree_only`
- `cache_late_answer_with_old_generation_is_ignored`
- `cache_failed_reload_keeps_previous_children`
- `completion_names_include_tables_columns_and_catalogs`
- `quote_ident_postgres_plain_lowercase_unquoted`, `quote_ident_postgres_mixed_case_quoted`, `quote_ident_mysql_uses_backticks`, `quote_ident_doubles_embedded_quotes`, `quote_ident_oracle_uppercase_unquoted`
- `ddl_detection_create_alter_drop_only` (uses the T302 lexer; `SELECT 'drop table'` is not DDL)

**Unit tests** — `crates/qry/src/components/schematree_tests.rs`:

- `flatten_respects_expanded_set`
- `collapse_on_child_moves_to_parent`
- `filter_keeps_ancestors_of_matches`
- `filter_does_not_trigger_loads`
- `filter_esc_clears_and_restores_selection`
- `system_objects_hidden_until_toggled`
- `enter_on_table_emits_insert_and_run_effects`
- `s_on_table_emits_insert_only`
- `delete_opens_confirm_and_cancel_is_noop`
- `confirm_delete_forgets_keyring_secret_and_saves`
- `delete_prompt_secret_sends_no_forget`

**Proptest:** `prop_flatten_never_panics_and_selection_stays_in_bounds`. A random tree plus random key sequences (`↑↓←→ space / Esc`) must keep `selected < visible.len()`, or be 0 when empty.

**Snapshots** — insta, plain and styled, via the T002 `styled()` helper:

- `schematree_collapsed_40x20`
- `schematree_expanded_columns_40x30`
- `schematree_loading_spinner_40x10` (virtual clock advanced 300 ms)
- `schematree_load_error_40x10`
- `schematree_filter_ord_40x20`
- `schematree_focused_vs_unfocused_40x10`
- `schematree_long_names_truncated_24x10`

**AppHarness flows:**

- `h.keys("enter")` on the scratch connection, then `h.keys("l l")`, expands `main → Tables`. A fake session answers. Assert on the `h.render(120, 40)` snapshot `browser_flow_scratch_120x40`.
- `h.keys("/ u s e r enter enter")` runs `SELECT * FROM users` and the results pane shows data.
- `h.keys("D")`, then `h.keys("esc")`: the connection is still present. Then `h.keys("D enter")`: it is removed.

**DB integration tests** — `crates/qry-core/tests/schema_<backend>.rs`, each opening with `let db = db_or_skip!(postgres())` and so on (T004):

- `catalogs_lists_default_first_and_flags_system`
- `objects_distinguishes_tables_and_views`
- `columns_report_type_nullability_pk_fk` (fixture: `users(id pk)`, `orders(user_id fk)`)
- `indexes_report_unique_and_columns`
- `hostile_names_are_bound_not_interpolated`
- `mixed_case_and_spaced_names_round_trip`
- SQLite runs always (no env needed), on a temp file.
- Oracle runs against `gvenzl/oracle-free` under `QRY_TEST_ORACLE_URL`.
