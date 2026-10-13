# T004 — Database integration test suite

**Phase:** 0 — Foundation · **Depends on:** T002, T003 · **Blocks:** none directly; T103, T301, T305 and T307 add their own cases here

## Description

qry talks to five backends through `qry_core::Database` (`crates/qry-core/src/shared.rs:54`). The modules are `sqlite.rs`, `postgres.rs`, `mysql.rs`, `mariadb.rs` and `oracle.rs`. Only two get real coverage today:

- **SQLite** is tested in-memory, through the worker in `crates/qry/src/db.rs`, e.g. `query_result_comes_back_as_actions`.
- **Oracle**'s TLS host-name checks are tested against a local rustls server, with certificates in `crates/qry-core/testdata/`.

No test opens a real Postgres, MySQL, MariaDB or Oracle session, so several behaviours are unverified:

- type-to-string conversion (`NULL`, numeric, date, binary through `hex`)
- `affected` counts
- `tables()` (Oracle overrides it in `oracle.rs` with an `all_tables`/`all_views` query on `CURRENT_SCHEMA`; the default's Oracle arm in `shared.rs` is unreachable)
- TLS modes (`SslMode::ensure_supported`)
- error messages users see

This task adds a per-backend integration suite that runs against real servers in CI, and skips cleanly on a developer machine without them. It mirrors sverb's `server-db` job and `crates/sverb-server/tests/common/mod.rs` (`fresh_db`, `db_or_skip!`), plus the `require_docker!` rule that a broken CI setup must fail loudly, never pass by skipping.

### Layout

```
crates/qry-core/tests/
  common/mod.rs        # fresh_db(), TestDb, db_or_skip!, backend URL lookup, shared assertions
  db_sqlite.rs         # always runs (tempfile)
  db_postgres.rs
  db_mysql.rs
  db_mariadb.rs
  db_oracle.rs
  db_contract.rs       # one generic contract, instantiated for every backend that is available
tests/fixtures/sql/
  seed_<backend>.sql   # the same logical schema and rows, in each dialect
```

### Environment contract

| Variable | Example (CI) |
|---|---|
| `QRY_TEST_POSTGRES_URL` | `postgres://qry:qry@localhost:5432/qry` |
| `QRY_TEST_MYSQL_URL` | `mysql://root:qry@localhost:3306/qry` |
| `QRY_TEST_MARIADB_URL` | `mysql://root:qry@localhost:3307/qry` |
| `QRY_TEST_ORACLE_URL` | `oracle://system:qry@localhost:1521/FREEPDB1` |
| `QRY_DB_TESTS=skip` | an explicit opt-out, used by the T003 `test` job |

### `common/mod.rs`

```rust
pub enum Backend { Postgres, Mysql, MariaDb, Oracle }

/// A throw-away database (or schema/user for Oracle) that exists for one test.
pub struct TestDb { pub backend: Backend, pub name: String, pub config: ConnectionConfig, admin_url: String }

impl TestDb {
    /// Creates `qry_test_<uuid v7 simple>`, connects to it, and loads
    /// `tests/fixtures/sql/seed_<backend>.sql`.
    pub async fn seeded(backend: Backend) -> Option<TestDb>;
    pub async fn fresh(backend: Backend) -> Option<TestDb>;   // empty
    pub async fn connect(&self) -> qry_core::Driver;
    pub async fn cleanup(self);   // DROP DATABASE … WITH (FORCE) / DROP USER … CASCADE
}

#[macro_export]
macro_rules! db_or_skip {
    ($backend:expr) => { match $crate::common::TestDb::seeded($backend).await {
        Some(db) => db,
        None => { $crate::common::skip_or_fail($backend); return; }
    }};
}
```

`skip_or_fail` decides what happens when a backend is missing:

| Situation | Result |
|---|---|
| `QRY_DB_TESTS=skip` | prints `skipped: <backend> (QRY_DB_TESTS=skip)` and returns |
| `CI` is set and the URL is missing | **panics** with `QRY_TEST_<X>_URL is not set on CI` |
| Otherwise (local, no URL) | prints a skip notice and returns |

Isolation and cleanup:

- **Per-test databases:** Postgres uses `CREATE DATABASE`, MySQL and MariaDB use `CREATE DATABASE`, and Oracle uses `CREATE USER qry_test_x IDENTIFIED BY … QUOTA UNLIMITED ON USERS` plus the grants, connecting as that user. Every test gets its own, so tests can run in parallel with the default thread count.
- **Cleanup** runs even when the test panics, through an `async` guard or a `catch_unwind` wrapper in the macro body.
- **Stragglers:** a test named `zz_no_leftover_test_databases` lists `qry_test_%` databases older than 1 hour, and fails if any exist, so leaked databases are noticed.

### The seed schema

The same rows in every dialect, chosen to cover the type mapping:

- a table `people(id int pk, name text, born date, balance numeric(12,2), active bool, avatar blob/bytea, note text NULL)` with 5 rows, including NULLs, unicode (`Zoë`, `東京`), an empty string, a negative number and binary bytes
- a view `active_people`
- a second table `orders` with a foreign key, for schema-browser tests (T301)
- Oracle: `NUMBER`, `VARCHAR2`, `DATE`, `RAW`, `CLOB`

### `db_contract.rs`: the shared contract

Each test is a generic `async fn contract_<name>(db: &TestDb)`, run for every backend by a macro that emits `#[tokio::test] async fn <name>_<backend>()`. One assertion therefore covers all five dialects, and a dialect difference becomes an explicit, documented `match` in the expectation, not a silent skip.

### `db` CI job (fills in T003's placeholder)

```yaml
  db:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:16
        env: { POSTGRES_USER: qry, POSTGRES_PASSWORD: qry, POSTGRES_DB: qry }
        ports: ["5432:5432"]
        options: >-
          --health-cmd "pg_isready -U qry" --health-interval 5s --health-timeout 5s --health-retries 20
      mysql:
        image: mysql:8.4
        env: { MYSQL_ROOT_PASSWORD: qry, MYSQL_DATABASE: qry }
        ports: ["3306:3306"]
        options: >-
          --health-cmd "mysqladmin ping -h 127.0.0.1 -pqry" --health-interval 5s --health-timeout 5s --health-retries 30
      mariadb:
        image: mariadb:11.4
        env: { MARIADB_ROOT_PASSWORD: qry, MARIADB_DATABASE: qry }
        ports: ["3307:3306"]
        options: >-
          --health-cmd "healthcheck.sh --connect --innodb_initialized" --health-interval 5s --health-timeout 5s --health-retries 30
      oracle:
        image: gvenzl/oracle-free:23-slim
        env: { ORACLE_PASSWORD: qry }
        ports: ["1521:1521"]
        options: >-
          --health-cmd healthcheck.sh --health-interval 10s --health-timeout 5s --health-retries 60
    env:
      QRY_TEST_POSTGRES_URL: postgres://qry:qry@localhost:5432/qry
      QRY_TEST_MYSQL_URL: mysql://root:qry@localhost:3306/qry
      QRY_TEST_MARIADB_URL: mysql://root:qry@localhost:3307/qry
      QRY_TEST_ORACLE_URL: oracle://system:qry@localhost:1521/FREEPDB1
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test -p qry-core --all-features --locked --no-fail-fast --test 'db_*'
      - name: Server logs
        if: failure()
        run: for c in $(docker ps -aq); do docker logs "$c" 2>&1 | tail -n 200; done
```

If the Oracle image's start-up (often 60–90 s) dominates the job, split Oracle into its own `db-oracle` job, so the other backends report quickly. Both jobs feed `ci-ok`.

### Running locally

Add `deploy/docker-compose.test.yml` with the same four services and ports, and a `scripts/db-test-env.sh` that prints the `export QRY_TEST_…` lines:

```sh
docker compose -f deploy/docker-compose.test.yml up -d --wait
eval "$(scripts/db-test-env.sh)"
cargo test -p qry-core --test 'db_*'
```

Document this in CONTRIBUTING (T401).

### Bugs this task is expected to surface

Fix the small ones in this task. File anything larger as a follow-up, with a failing test marked `#[ignore = "T0xx: <reason>"]`.

- Oracle's `tables()` is overridden in `oracle.rs`, so the `bail!` arm in the default `Database::tables()` (`shared.rs`) is dead code. Cover the override with a contract test, and replace the dead arm with `unreachable!`-free code (e.g. move per-backend SQL into each backend). T301 extends this to schemas.
- Binary columns, dates and booleans render differently per backend. The contract pins one textual form per type, and each backend converts to it.

### Out of scope

- Schema browsing beyond tables (T301).
- Cancellation (T305) and multi-session (T103). Those tasks add contract cases here.

## Acceptance criteria

- [ ] `crates/qry-core/tests/common/mod.rs` provides `TestDb`, `Backend`, `db_or_skip!` and `skip_or_fail` as described.
- [ ] Seed files exist for postgres, mysql (also used for mariadb), oracle and sqlite under `tests/fixtures/sql/`, and every one loads without error.
- [ ] Every contract test below runs for SQLite, Postgres, MySQL, MariaDB and Oracle. The CI `db` job log shows each `<name>_<backend>` as `ok`, with zero skipped.
- [ ] With no `QRY_TEST_*` variables and no `CI`, `cargo test -p qry-core` passes locally and prints one skip notice per backend.
- [ ] With `CI=true` and no URLs, the server tests panic with a message naming the missing variable (verify once).
- [ ] Each test creates and drops its own database, and tests pass with the default parallel test threads.
- [ ] `zz_no_leftover_test_databases` passes on CI.
- [ ] Oracle `tables()` (the `oracle.rs` override) returns the seed tables and view under the current schema. The dead Oracle `bail!` arm in `shared.rs` is gone.
- [ ] `deploy/docker-compose.test.yml` and `scripts/db-test-env.sh` work: `docker compose … up -d --wait` followed by the tests passes on a developer machine.
- [ ] The `db` job (or `db` plus `db-oracle`) is green and listed under `ci-ok`'s `needs`.

## Tests

Contract tests in `db_contract.rs`, each instantiated for every backend:

- **`connects_and_reports_a_label`**: `Driver::connect` returns a label containing the host (or the file name for SQLite) and the database name.
- **`select_returns_columns_and_rows_in_order`**: `SELECT id, name FROM people ORDER BY id` gives columns `["id", "name"]` (case-normalised, since Oracle upper-cases) and 5 rows in id order.
- **`null_is_none_and_empty_string_is_some_empty`**: Oracle treats `''` as NULL. The test asserts that explicitly for Oracle, and documents it.
- **`unicode_round_trips`**: `Zoë` and `東京` come back byte-identical.
- **`numeric_keeps_scale_and_sign`**: `-12.50` comes back as `"-12.50"`, not `"-12.5"`.
- **`dates_render_as_iso_8601`**: `1990-04-01` for `DATE`. Timestamps are `YYYY-MM-DD HH:MM:SS[.fff]`.
- **`booleans_render_as_true_false`**: MySQL/MariaDB `tinyint(1)` included.
- **`binary_renders_as_hex`**: `avatar` bytes come back as `\x` plus lowercase hex, matching `hex::encode`.
- **`dml_reports_affected_rows`**: an `UPDATE people SET active = …` that touches 2 rows gives `affected == Some(2)` and no columns.
- **`ddl_succeeds_without_rows`**
- **`syntax_error_is_an_error_with_the_server_message`**: the error text contains the server's own wording, not just "query failed".
- **`tables_lists_tables_and_views_sorted`**: gives `["active_people", "orders", "people"]`, case-normalised.
- **`a_wrong_password_fails_with_an_auth_error`**: the connect error contains `password` or `authentication` (or `ORA-01017` for Oracle). The password does not appear in the error.
- **`a_closed_port_fails_fast`**: connecting to `127.0.0.1:1` fails within 5 s.

Backend-specific tests:

- **`db_postgres.rs`:**
  - **`ssl_mode_require_against_a_non_tls_server_fails_clearly`**
  - **`search_path_decides_current_schema_for_tables`**
- **`db_mysql.rs` and `db_mariadb.rs`:**
  - **`zero_dates_do_not_panic`** (`0000-00-00`)
  - **`multiple_result_sets_take_the_first`**: documents the current behaviour, which T302 revisits.
- **`db_oracle.rs`:**
  - **`clob_is_read_as_text`**
  - **`number_without_scale_is_an_integer_string`**
  - **`tables_uses_user_tables_and_views`**
  - The existing rustls-server TLS tests move here from `oracle.rs` unit tests, if they need a socket.
- **`db_sqlite.rs`:**
  - **`file_database_persists_across_connections`**: via `tempfile`.
  - **`read_only_rejects_writes`**: `SqliteConfig { read_only: true }`.

Housekeeping:

- **`zz_no_leftover_test_databases`**: once per server backend.

CI: `db` (and `db-oracle`, if split) green; `ci-ok` green.
