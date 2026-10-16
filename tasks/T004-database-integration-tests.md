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

- [x] `crates/qry-core/tests/common/mod.rs` provides `TestDb`, `Backend`, `db_or_skip!` and `skip_or_fail` as described.
- [ ] Seed files exist for postgres, mysql (also used for mariadb), oracle and sqlite under `tests/fixtures/sql/`, and every one loads without error.
- [ ] Every contract test below runs for SQLite, Postgres, MySQL, MariaDB and Oracle. The CI `db` job log shows each `<name>_<backend>` as `ok`, with zero skipped.
- [x] With no `QRY_TEST_*` variables and no `CI`, `cargo test -p qry-core` passes locally and prints one skip notice per backend.
- [x] With `CI=true` and no URLs, the server tests panic with a message naming the missing variable (verify once).
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

## Implementation notes

- **Seven boxes stay unticked, because no Oracle or MySQL server and no GitHub run were available.** Postgres and MariaDB were checked against real local servers instead (below), so the unticked boxes are mostly unverified rather than known to fail:
  - **Seeds load:** `seed_sqlite.sql`, `seed_postgres.sql` and `seed_mysql.sql` load (SQLite always; Postgres 16; MariaDB 10.11). `seed_oracle.sql` has never run.
  - **Every contract test on every backend, zero skipped, on CI**, **`zz_no_leftover_test_databases` on CI**, and **the `db` job green**: need a GitHub run. `db` is in `ci-ok`'s `needs` (it already was).
  - **Each test creates and drops its own database, in parallel:** done and checked for SQLite, Postgres and MariaDB, with the default thread count. The Oracle path (a user per test, sessions killed before `DROP USER … CASCADE`) has never run.
  - **Oracle `tables()`:** the dead `bail!` arm is gone (see below), but the override returning the seed's tables is unverified.
  - **`docker compose … up -d --wait`:** there is a `docker` CLI but no daemon here. `docker compose -f deploy/docker-compose.test.yml config -q` accepts the file, and `scripts/db-test-env.sh` prints the four `export` lines.
- **What was run against real servers.** A throw-away Postgres 16 cluster (`initdb` with scram-sha-256, user/password/database `qry`, like the CI service) and MariaDB 10.11 (`mariadb-install-db`, root password `qry`), both started by hand in a temp directory and removed afterwards:
  - Postgres: all 18 contract tests and the 3 `db_postgres.rs` tests pass. With a test that panics while connected, its database is still dropped (`DROP DATABASE … WITH (FORCE)` from `Drop`). A planted `qry_test_000000000001…` database makes `zz_no_leftover_test_databases` fail and name it. With a URL whose database does not exist, every test panics with `QRY_TEST_POSTGRES_URL is set, but connecting to it failed: …`; a configured server never skips.
  - MariaDB, pointed at by both `QRY_TEST_MYSQL_URL` and `QRY_TEST_MARIADB_URL`: all contract tests pass as `::mariadb`, and the `db_mysql.rs`/`db_mariadb.rs` tests pass. As `::mysql`, only `column_kinds_mapped` fails, as it should: MariaDB sends JSON as text, and real MySQL 8.4 sends `MYSQL_TYPE_JSON`. MySQL 8.4 itself (and its `caching_sha2_password` login over plain TCP) is unverified.
  - Oracle: nothing ran. The Oracle expectations come from reading the `oracledb` 26.0.0-beta.4 sources: `Display` of `OracleTimestamp` and `OracleNumber`, `DbError::offset`, CLOBs fetched as strings by default.
- **Layout** (as specified, with `Backend::Sqlite` added):
  - `crates/qry-core/tests/common/mod.rs`: `Backend` (`Sqlite`, `Postgres`, `Mysql`, `MariaDb`, `Oracle`, with `env_var`, `dialect` and `seed_path`), `TestDb { backend, name, config, … }` with `seeded`, `fresh`, `connect`, `cleanup` and `sqlite_path`, plus `db_or_skip!`, `fresh_or_skip!`, `skip_or_fail`, `url`, `admin`, `parse_url`/`config_from_url`, `test_db_name`/`created_at`, `leftover_test_databases`, `q` (run or panic with the statement and the error chain), `run_script` (splits with `qry_core::sql::split`, as the editor does) and small assertions (`column`, `lower_names`, `lower`, `assert_no_rows`). `common/mysql_family.rs` holds the bodies that `db_mysql.rs` and `db_mariadb.rs` share.
  - `db_contract.rs` (18 contracts × 5 backends = 90 tests), `db_sqlite.rs` (3), `db_postgres.rs` (3), `db_mysql.rs` (3), `db_mariadb.rs` (3) and `db_oracle.rs` (4).
  - The seeds are in `tests/fixtures/sql/` at the workspace root, which T002 left for this task. They hold `people` (5 rows: NULLs, `Zoë`, `東京`, an empty note, a negative balance, binary avatars, a quote), the `active_people` view, and `orders` with a foreign key.
- **Decisions and deviations:**
  - **Test names are `<name>::<backend>`, not `<name>_<backend>`.** Joining identifiers in `macro_rules!` needs `paste`, which is not in `Cargo.lock`. So `contract!` emits `mod <name> { async fn sqlite() … async fn oracle() }`, and each module calls the contract function of the same name (`<name>`, not `contract_<name>`). `cargo test <name>` and `cargo test ::postgres` still filter as you would expect.
  - **Database names:** `qry_test_` plus 32 hex digits shaped like a UUID v7. The first 12 digits are the creation time in Unix milliseconds, and the rest are hashed from `RandomState`, a counter, the pid and the clock. No `uuid`/`rand` dependency was added. `created_at` reads the time back, so `zz_no_leftover_test_databases` needs no server-side creation time (Postgres keeps none). A `qry_test_` name that does not decode counts as a leftover.
  - **Cleanup on panic:** `TestDb`'s `Drop` runs when `cleanup()` was not reached. It drops the database from a thread of its own, with its own current-thread runtime, so no `catch_unwind` is needed in the macros. A failure there is printed, never a panic inside a panic. The happy path calls `db.cleanup().await`.
  - **Skip notices** are written once per backend and test binary, straight to `std::io::stderr()`. That bypasses libtest's capture, so a plain `cargo test -p qry-core` shows e.g. `skipped: postgres (QRY_TEST_POSTGRES_URL is not set)`, once in `db_contract` and once in `db_postgres`. `QRY_DB_TESTS=skip` (case-insensitive) wins over `CI`. SQLite never skips, also under `QRY_DB_TESTS=skip`.
  - **`CI=true` alone makes the server tests fail on purpose.** That is the specified behaviour. The suggested local check `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked` therefore needs `QRY_DB_TESTS=skip` now, as the CI `test` job already sets it. Without it, the 85 server tests panic with `QRY_TEST_<X>_URL is not set on CI`.
  - **Oracle test users** are created by the URL's account (`system` on CI) with `CREATE USER qry_test_… IDENTIFIED BY "Qry_test_pw1" QUOTA UNLIMITED ON USERS` and `GRANT CREATE SESSION, CREATE TABLE, CREATE VIEW`. Before `DROP USER … CASCADE`, the user's sessions in `v$session` are killed, retrying for up to 10 s (ORA-01940), and ORA-01918 counts as already dropped.
  - **The rustls TLS tests stay in `oracle_tests.rs`.** They need a loopback socket but no database, and they use private helpers. Moving them would only add churn.
  - **Dialect differences, each written as a `match` in the contract:** Oracle stores `''` as NULL. SQLite has no fixed-point type (`-12.5`, `1000`, `0`, `42.1`), no booleans (`1`/`0`) and no timestamp type (it uses `datetime(…)`), and its column kinds come from the first value. Oracle's integers are `NUMBER`, so their kind is `Decimal`. MariaDB's JSON column is text. MySQL/MariaDB report no error position (T302). SQLite skips the two connect-error contracts, because it has no accounts and no port.
- **Small bugs fixed in `qry-core`** (the contract pins one textual form per type):
  - **`Database::tables()` is now a required method.** Each backend has its own query: SQLite `sqlite_master`, Postgres `current_schema()`, MySQL/MariaDB `DATABASE()` (`mysql::TABLES_SQL`), and Oracle its `all_tables`/`all_views` override. The unreachable Oracle `bail!` arm in `shared.rs` is gone, and `shared::first_column` collects the names. `FakeDatabase` already overrode `tables()`.
  - **Binary renders as `\x` plus lowercase hex everywhere.** SQLite and Oracle used `x'…'`. MySQL/MariaDB ran blobs through `from_utf8_lossy`, which garbled them; the binary collation (63) now tells `BLOB`/`VARBINARY` from `TEXT`/`VARCHAR`, and also gives `TEXT` the kind `Text` instead of `Binary`.
  - **Booleans:** Postgres `t`/`f` and MySQL/MariaDB `tinyint(1)` `1`/`0` are now `true`/`false`, and `tinyint(1)` has the kind `Bool`.
  - **Oracle `DATE`/`TIMESTAMP`** were written `1990-04-01T00:00:00.000000000Z`. A `DATE` is now `1990-04-01`, or `… HH:MM:SS` when the time is not midnight. A timestamp is `YYYY-MM-DD HH:MM:SS[.f…]`, with trailing zeros trimmed, and a `WITH TIME ZONE` value is followed by `±HH:MM`.
  - **Oracle `NUMBER(p,s)`** keeps its scale (`-12.50`, not `-12.5`).
  - **Oracle error positions:** a server error becomes a `QueryError` whose position comes from `DbError::offset`, a 0-based character count where 0 means none.
  - **SQLite `affected` for DML** was the row count of the (empty) result, so 0. A statement without columns now reports `conn.changes()`.
- **CI:** the `db` job now has the four services and URLs from this task's snippet and runs `cargo test -p qry-core --all-features --locked --no-fail-fast --test 'db_*'`, with a step that prints the server logs on failure. Oracle is not split into `db-oracle` yet; the job comment says when to do it. The `--locked` grep still finds only `cargo fmt` and `cargo fuzz` lines.
- **Local runs:** `deploy/docker-compose.test.yml` (the same images, credentials, ports and health checks as CI) and `scripts/db-test-env.sh` (prints the `export` lines; `QRY_TEST_HOST` overrides `localhost`). Documenting them in CONTRIBUTING is T401's.
- **Tests added:** the 106 integration tests above, `mysql_tests.rs` (4: collation, `tinyint(1)`, numbers, cells) and 3 in `oracle_tests.rs` (`numbers_keep_the_scale_of_their_column`, `dates_and_timestamps_are_iso_8601_with_a_space`, `the_column_type_says_how_a_timestamp_is_written`). That makes 97 `qry-core` unit tests.
- **Checks run locally:** `cargo fmt --all`, clippy with `-D warnings` for both feature sets, `QRY_KEYRING=off CI=true QRY_DB_TESTS=skip cargo test --workspace --all-features --locked` (403 `qry` unit tests, every integration test, 97 in `qry-core`), plain `cargo test -p qry-core` without `CI`, and the CI docs command. The server runs are described above.
