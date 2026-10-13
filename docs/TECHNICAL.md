# qry — Technical Documentation

qry is a terminal SQL client written in Rust. You keep a list of database
connections, open one, browse its tables, write SQL in an editor, read the
result in a scrollable grid and export it to a file. It runs on Linux, macOS
and Windows and talks to SQLite, PostgreSQL, MySQL, MariaDB and Oracle.

This document describes how qry is put together, and records the move of the
Oracle backend from the `oracle` crate to Oracle's own `oracledb` crate.

---

## 1. Workspace layout

```
Cargo.toml                  workspace manifest; shared dependency versions
.config/config.json         default keybindings, compiled into the binary
.envrc                      direnv: config, data and logs stay in the repo
.github/workflows/          ci.yml (checks), cd.yml (release builds on tags)
crates/
  qry-core/                 database access and export; no terminal code
  qry/                      the binary: terminal, panes, input, storage
docs/TECHNICAL.md           this file
```

The split is a rule, not a convenience: `qry-core` must never depend on
`ratatui`, `crossterm` or `clap`. Everything in it can be tested without a
TTY.

| Setting        | Value                                   |
|----------------|-----------------------------------------|
| Edition        | 2024                                    |
| MSRV           | Rust 1.89 (`rust-version` in the workspace) |
| Async runtime  | tokio (multi-threaded)                  |
| UI             | ratatui 0.30 + crossterm 0.29           |
| License        | MIT                                     |

---

## 2. `qry-core`: database access

### 2.1 The `Database` trait (`shared.rs`)

Every backend implements one async trait:

```rust
#[async_trait]
pub trait Database: Send + Sync {
    fn kind(&self) -> DatabaseType;
    fn label(&self) -> String;                       // user@host:port/db
    async fn query(&self, sql: &str) -> Result<QueryResult>;
    async fn execute(&self, sql: &str) -> Result<u64>;
    async fn tables(&self) -> Result<Vec<String>>;   // default per kind
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,              // None = NULL
    pub affected: Option<u64>,
}
```

The result model is deliberately text-only. Each backend turns its native
values into strings, so the grid and the exporters never deal with types.
NULL stays distinct from the empty string (`None` versus `Some("")`).

`query` is what the editor calls for **every** statement, including DDL and
DML. A statement that returns no rows comes back with empty `columns` and the
affected count. The worker uses empty `columns` to say "Statement executed"
rather than "Fetched 0 rows".

`tables` has a default implementation that picks a catalogue query by
`kind()` (`sqlite_master`, or `information_schema` for Postgres and
MySQL/MariaDB). Oracle has no `information_schema` and overrides it.

### 2.2 `Driver` (`lib.rs`)

`Driver` holds at most one open `Box<dyn Database>`. `connect` builds the
right backend from a `ConnectionConfig` enum and replaces whatever was open.
`query` and `tables` fail with "not connected" when nothing is open.

`ConnectionConfig` implements `Debug` by hand, so printing one shows
`Kind(user@host:port/db)` and never the password. There is a test for this.

### 2.3 Backends

| Backend    | Crate             | How it runs                                   | Value conversion |
|------------|-------------------|-----------------------------------------------|------------------|
| SQLite     | `tokio-rusqlite` (bundled SQLite) | async wrapper over a background thread | BLOBs as `x'…'` hex |
| PostgreSQL | `tokio-postgres`  | native async; the connection future runs on a spawned task, aborted on drop | simple-query protocol: every value already text |
| MySQL      | `mysql_async`     | native async; `Conn` behind a `tokio::sync::Mutex` | `Value` rendered per variant |
| MariaDB    | `mysql_async`     | same code as MySQL, its own config type        | same |
| Oracle     | `oracledb` (see §6) | blocking driver run on `spawn_blocking`, `Arc<Mutex<Connection>>` | per Oracle type, see §6.4 |

**TLS.** `SslMode` uses libpq's names (`disable`, `allow`, `prefer`,
`require`, `verify-ca`, `verify-full`) for every backend.

- **Oracle** speaks TLS (TCPS). `require` and stricter encrypt the connection
  and verify the server. See §7.
- **PostgreSQL, MySQL, MariaDB** do not speak TLS yet.
  `SslMode::ensure_supported` refuses the last three modes *before* any
  network traffic, so a user who asked for encryption never gets a plain
  connection by accident.

**Autocommit.** Every backend commits as it goes. Postgres and MySQL do this
by default. Oracle does not, so qry commits after each statement that is not
a query (§6.3).

### 2.4 Export (`exporter.rs`)

`Exporter::from_result` takes a `QueryResult` and an `ExportConfig` (path,
separator, type) and writes one of:

| Type  | Extension | Notes |
|-------|-----------|-------|
| CSV   | `.csv`    | RFC 4180 quoting with the chosen separator |
| Text  | `.txt`    | separator-delimited; tabs and newlines inside values become spaces |
| Excel | `.xlsx`   | `rust_xlsxwriter`, every cell a string |
| JSON  | `.json`   | one object per row keyed by column name, NULL as `null` |

`ExportType::with_extension` adds the extension unless it is already there
(ignoring case). The worker runs the write on `spawn_blocking`.

---

## 3. `qry`: the application

### 3.1 Start-up (`main.rs`)

1. `errors::init` installs color-eyre, a panic hook that restores the
   terminal, and human-panic for release builds.
2. `logging::init` points `tracing` at `<data dir>/qry.log`.
3. `clap` parses `--tick-rate` and `--frame-rate`.
4. `App::new` loads the config, spawns the database worker and builds the
   component tree; `App::run` enters the event loop.

### 3.2 The event loop and `Action` (`app.rs`, `action.rs`)

All communication goes through one enum, `Action`, on an unbounded tokio
channel. Terminal events (keys, resize, tick, render) are turned into actions;
components return actions from `handle_key_event` and `update`; `App`
dispatches each one to every component and handles the global ones (quit,
mode changes, saving connections, forwarding commands to the worker).

Actions are cloned once per component, so the large payload (`QueryDone`)
is wrapped in an `Arc`.

`Mode` (`Home`, `AddConnModal`, `ExpoModal`, `PasswordModal`) picks the
keybinding table and which modal is shown.

### 3.3 The database worker (`db.rs`)

The event loop never awaits the database. `db::spawn` starts a task that owns
the `Driver` and receives `DbCommand`s (`Connect`, `PasswordEntered`, `Query`,
`Export`, `ListTables`). Commands run one at a time, in order. Answers return
as actions: `Connected` / `ConnectFailed`, `QueryDone` plus a status line
with the elapsed time, `TablesLoaded` / `TablesFailed`, `Exported` /
`ExportFailed`. Error text goes through `secrets::redact` before it is shown
or logged.

### 3.4 Components (`components/`)

| Component     | File              | Role |
|---------------|-------------------|------|
| `Home`        | `home.rs`         | lays out the panes, owns focus and the open modal |
| `ConnTree`    | `conntree.rs`     | saved connections; expand to list tables; connect, edit, delete |
| `Editor`      | `editor.rs`       | `ratatui-textarea` SQL editor; F8 runs the query |
| `Results`     | `results.rs`      | table with cell cursor and scrollbars; columns capped at 40 chars |
| `Statuspanel` | `statuspanel.rs`  | last success or error |
| `ConnForm`    | `connform.rs`     | new / edit connection modal (driver-specific fields) |
| `ExportForm`  | `exportform.rs`   | export modal |
| `PasswordForm`| `passwordform.rs` | password prompt |
| `Help`        | `help.rs`         | keybinding overlay built from the live keymaps |
| `Form`, `TextInput` | `form.rs`, `textinput.rs` | shared form plumbing |

A modal that triggers a connect or an export stays open until the worker
answers (`Home::awaiting`), so a typo does not throw away the form.

### 3.5 Configuration and keybindings (`config.rs`, `keymap.rs`)

Defaults come from `.config/config.json`, compiled in. At start-up qry also
reads `config.{json5,json,yaml,toml,ini}` from the user config directory
(`QRY_CONFIG` overrides it) and layers it on top.

- `keybindings` are keyed by `Mode`, then by key sequence, and name an
  `Action`.
- `panes` are keyed by pane (`Editor`, `Results`, `Tree`, `Form`, `Help`) and
  map one key to a pane command enum (`EditorCommand`, `ResultsCommand`, …).
  User keys win over defaults; missing keys fall back to them.

### 3.6 Saved connections (`connections.rs`)

Connections live in `<data dir>/connections.json`, a versioned file
(`VERSION = 1`; a newer version is refused, not guessed at). Writes go to a
temp file which is then renamed over the original.

`StoredConnection` holds an immutable `id` (the keychain account), a name, a
`Driver`, the connection fields that driver uses (`host`, `port`, `user`,
`database`, Oracle `service` and `wallet`, SQLite `path` and `mode`,
`schema`, `tls`) and a
`secret` mode. Unknown fields are kept in `extra`, so an older qry does not
strip what a newer one wrote. `to_config(password)` produces the
`qry_core::ConnectionConfig`.

### 3.7 Secrets (`secrets.rs`)

Two rules:

1. A password is never written to a file qry controls.
2. A password never travels inside an `Action`, so it cannot reach the
   screen, the log or a panic message.

`Secret` says where a password comes from: `Keyring` (OS keychain through the
`keyring` crate, account = connection id), `Env { var }`, `Prompt` (asked on
every connect) or `None` (SQLite, trust auth). A typed password goes into a
process-wide one-slot stash, which the worker takes from. `Password` zeroizes
on drop and its `Debug` prints `[redacted]`. `redact` scrubs passwords from
DSN-shaped text in error messages.

---

## 4. Running

```sh
cargo run -p qry
cargo run -p qry -- --version    # git info and resolved directories
```

With direnv, `.envrc` sets `QRY_CONFIG`, `QRY_DATA` and `QRY_LOG_LEVEL=debug`
so everything stays inside the repo.

---

## 5. Tests and checks

Unit tests sit next to the code they test. The four gates CI runs, which must
all pass before a change goes in:

```sh
cargo test --locked --all-features --workspace
cargo fmt --all --check
cargo clippy --all-targets --all-features --workspace -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --all-features --workspace
```

Tests that need a live server are not part of the suite. The backend tests
that do run are offline: they use SQLite in memory, or check that bad
configurations are refused before any connection is attempted, or connect to
`127.0.0.1:1` so a refused connection fails fast.

---

## 6. Oracle backend: moving to `oracledb`

### 6.1 The problem

The Oracle backend used the `oracle` crate (rust-oracle, by Kubo Takehiro).
It wraps ODPI-C, which loads the Oracle Client libraries at run time. On a
machine without Oracle Instant Client every connection fails with:

```
DPI-1047: Cannot locate a 64-bit Oracle Client library
```

That cannot be fixed inside qry. It means every user has to download and
install Instant Client and put it on the library path, which is a poor fit
for a single static binary.

### 6.2 The replacement

[`oracledb`](https://crates.io/crates/oracledb) is Oracle's own Rust driver
([github.com/oracle/rust-oracledb](https://github.com/oracle/rust-oracledb)).
It is a pure-Rust **thin** driver: it speaks Oracle's network protocol itself
and needs no Oracle Client libraries.

| | `oracle` 0.6 (old) | `oracledb` 26.0.0-beta.4 (new) |
|---|---|---|
| Maintainer | community | Oracle Corporation |
| Client libraries | Instant Client required at run time | none |
| Implementation | Rust over ODPI-C (C) | pure Rust, `#![forbid(unsafe_code)]` |
| API | blocking | blocking |
| MSRV | — | 1.89, same as qry |
| Databases | 11.2 and later | 12.1 and later (12, 18, 19, 21, 26ai) |
| License | UPL / Apache-2.0 | UPL-1.0 OR Apache-2.0 |
| Status | stable | **pre-release** (beta) |

The crate is still a beta, so its API may change between releases. qry pins
the exact version (`=26.0.0-beta.4`) so an upgrade only happens on purpose.

### 6.3 What changes in `crates/qry-core/src/oracle.rs`

The module's shape stays the same. `OracleConfig`, `Oracle::connect`, the
`Database` impl and `tables()` keep their signatures, so nothing outside the
file changes except `Cargo.toml`.

| Concern | Old (`oracle`) | New (`oracledb`) |
|---|---|---|
| Connecting | `Connection::connect(user, pw, "//host:port/svc")` | `oracledb::connect(Config::default().set_credentials(user, pw).set_connect_string("//host:port/svc")?)` |
| Autocommit | `conn.set_autocommit(true)` | no such switch; qry calls `conn.commit()` after every statement that is not a query |
| Query vs. statement | `stmt.is_query()`, then `stmt.query` or `stmt.execute` | same: `conn.statement(sql)?.build()?`, then `is_query()` |
| Column names | `result_set.column_info()[i].name()` | `cursor.columns()[i].name()` |
| Rows affected | `stmt.row_count()` | `ExecResult::rows_affected()` |
| Cell to text | `row.get::<_, Option<String>>(i)` converted any type | only character types convert to `String`; qry formats the other types itself (§6.4) |
| Blocking work | `spawn_blocking` + `Arc<Mutex<Connection>>` | unchanged |
| Wallet / TLS | refused before connecting | unchanged: still refused before connecting |

**Autocommit, in detail.** Oracle sessions start a transaction on the first
DML statement and keep it open until a commit. The other qry backends commit
each statement, and a user expects an `INSERT` in the editor to stick. After
any non-query statement the backend therefore issues `COMMIT`. For DDL this
is harmless (Oracle already commits implicitly). A failed statement is not
committed, and the error is returned as before.

### 6.4 Turning Oracle values into text

`oracledb` converts to `String` only from character columns (`VARCHAR2`,
`CHAR`, `NVARCHAR2`, `CLOB`, `LONG`, …) and `ROWID`. qry asks for the value as
each type it knows, in this order, and formats the first one that matches:

| Oracle type | Read as | Shown as |
|---|---|---|
| character, CLOB, LONG, ROWID | `String` | the text |
| `NUMBER`, `FLOAT`, `INTEGER` | `OracleNumber` | Oracle's decimal text, no rounding |
| `BINARY_DOUBLE` / `BINARY_FLOAT` | `f64` / `f32` | Rust float formatting |
| `DATE`, `TIMESTAMP [WITH [LOCAL] TIME ZONE]` | `OracleTimestamp` | ISO 8601, e.g. `2026-10-13T09:30:00.000000000Z` |
| `INTERVAL DAY TO SECOND` / `YEAR TO MONTH` | `OracleIntervalDS` / `OracleIntervalYM` | the driver's `Display` |
| `BOOLEAN` | `bool` | `true` / `false` |
| `RAW`, `LONG RAW`, `BLOB` | `Vec<u8>` | `x'…'` hex, same as SQLite |
| `JSON` | `JsonValue` | compact JSON text |
| `VECTOR` | `Vector` | the driver's `Debug` form |
| anything else | — | the error names the column and its type |

NULL is `None` for every type. LOBs are fetched inline (the driver's default),
not as locators.

### 6.5 Tests

Every existing Oracle test is kept and still passes:

- `label_is_user_at_host_port_service`: label and connect string.
- `debug_output_never_contains_the_password`.
- `identifiers_are_quoted_for_the_session_schema`.
- `a_wallet_is_refused_before_connecting`. Renamed
  `a_wallet_without_tls_is_refused_before_connecting` once TLS arrived (§7):
  a wallet is now refused only when the SSL mode is not TLS.
- `an_unreachable_server_is_an_error_not_a_panic`. Before, this passed
  because Instant Client was missing. Now it passes because nothing listens
  on `127.0.0.1:1`, which is what it was meant to test.

New offline tests:

- the connect string qry builds is accepted by `oracledb`'s own parser;
- the JSON formatter renders objects, arrays, strings with escapes, and NULL.

Tests against a live Oracle server stay out of CI, as for the other backends.
To try one by hand, a free container works:

```sh
docker run -d -p 1521:1521 -e ORACLE_PASSWORD=secret gvenzl/oracle-free
# then in qry: host localhost, port 1521, service FREEPDB1, user system
```

### 6.6 Known limits

- **Beta driver.** Pinned exactly. Read the changelog before raising the pin.
- **TLS.** Supported since §7, with the limits listed there.
- **Thin mode only.** Features that need Thick mode (for example some
  external authentication methods) are not available. Username and password
  over TCP, the only thing qry supports, works.
- **Older databases.** The thin driver needs Oracle Database 12.1 or later.

---

## 7. Encrypted Oracle connections (TCPS)

### 7.1 What the user sets

The connection form has two fields for this:

| Field | Stored as | Meaning for Oracle |
|---|---|---|
| SSL mode | `tls` | `disable`, `allow`, `prefer`: plain TCP. `require`, `verify-ca`, `verify-full`: TCPS. |
| Wallet | `wallet` | Optional. A folder holding `ewallet.pem`. Only allowed over TCPS. |

Oracle listeners usually serve TCPS on a port of their own, often **2484**.
The default port stays 1521, so set the port when you turn TLS on.

The Wallet field is refused for every other backend ("Wallet is only used by
Oracle").

### 7.2 How it works

`OracleConfig` (in `qry-core`) gained `ssl_mode`. `uses_tls()` is true for
`require`, `verify-ca` and `verify-full`. When it is true:

1. The connect string becomes `tcps://host:port/service` instead of
   `//host:port/service`. `oracledb` negotiates TLS as soon as the TCP
   connection opens, before any Oracle traffic.
2. The wallet directory, if set, goes to `Config::set_wallet_location`.

The driver uses rustls. It always verifies the server's certificate chain and
checks that the certificate names the host qry connected to. The trusted
roots are the Mozilla set (`webpki-roots`, compiled in, so the OS store is
not used) plus any certificates in the wallet.

What this means for each mode:

| Mode | libpq meaning | What qry does for Oracle |
|---|---|---|
| `disable`, `allow`, `prefer` | no TLS, or TLS if offered | plain TCP. There is nothing to fall back from, because TCPS is on its own port. |
| `require` | encrypt, don't verify | encrypt **and** verify chain and host name |
| `verify-ca` | encrypt, verify chain | encrypt **and** verify chain and host name |
| `verify-full` | encrypt, verify chain and host | encrypt and verify chain and host name |

The driver has no switch to skip verification. So `require` and `verify-ca`
are stricter than libpq's, never weaker. A server with a private CA needs
that CA in a wallet (§7.3).

### 7.3 Wallets

`ewallet.pem` is read in one of two ways:

- **No private key in the file:** every certificate in it is added to the
  trusted roots. Use this for a database whose certificate comes from a
  private or self-signed CA. Put the CA certificate in `ewallet.pem`.
- **With a private key:** the file is a client certificate chain and its key,
  sent for **mutual TLS** (for example an Autonomous Database mTLS wallet).
  In this case the driver does *not* also add those certificates to the
  trusted roots, so the server's certificate must come from a CA in the
  Mozilla set.

Wallet checks run before the driver touches the network:

| Situation | Message |
|---|---|
| Wallet set, SSL mode not TLS | the wallet `…` is only used over TLS; set the SSL mode to `require` or stricter |
| `ewallet.pem` missing or unreadable | could not read the wallet file `…/ewallet.pem` |
| Key is encrypted (`BEGIN ENCRYPTED PRIVATE KEY`) | the private key in `…` is encrypted, and qry cannot take a wallet password yet |

### 7.4 Checks before connecting

- **Host name.** With TLS, the host must be a name a certificate can carry (a
  DNS name or an IP address). `rustls-pki-types` checks it first, because
  the driver would otherwise panic on a bad one inside its TLS setup.
- **Old records.** Before TLS, an Oracle record's `tls` field held a wallet
  directory. `StoredConnection::oracle_tls` still reads that shape as
  `verify-full` plus that wallet. Opening the record in the form and saving
  it writes the new shape (`tls` = mode, `wallet` = directory).

### 7.5 Tests

All run offline. The TLS tests use a TLS server on loopback, built from
rustls (a dev-dependency) and the fixtures in `crates/qry-core/testdata/`:

| File | What it is |
|---|---|
| `ca.pem` | a test CA, valid until 2126. Its private key was deleted after signing. |
| `server.pem` | a certificate for `127.0.0.1` (SAN `IP:127.0.0.1`), signed by that CA |
| `server.key` | the unencrypted PKCS#8 key for `server.pem` |

| Test | Shows |
|---|---|
| `require_and_stricter_go_over_tcps` | which modes use TLS, the `tcps://` string, and that the driver's descriptor says `PROTOCOL=tcps` |
| `tls_is_negotiated_before_anything_else_is_sent` | over TLS the first bytes on the wire are a TLS record (`16 03`); without TLS they are a TNS CONNECT packet |
| `an_untrusted_certificate_is_refused` | a certificate from an unknown CA fails the handshake, and no data crosses |
| `a_wallet_can_add_the_ca_to_trust` | with the CA in the wallet, the handshake succeeds and the TNS CONNECT arrives inside TLS |
| `a_wallet_without_tls_is_refused_before_connecting` | wallet plus plain mode is refused |
| `a_host_no_certificate_can_name_is_refused_before_connecting` | a bad host name is refused, with no panic |
| `a_missing_wallet_file_says_where_it_looked` | the message names `ewallet.pem` and the folder |
| `an_encrypted_wallet_key_is_explained` | an encrypted key gets a clear message |
| `an_oracle_record_carries_its_ssl_mode_and_wallet`, `an_old_oracle_record_with_a_wallet_in_tls_means_tcps` | stored record → `OracleConfig` |
| `oracle_takes_an_ssl_mode_and_a_wallet`, `a_wallet_is_refused_for_anything_but_oracle`, `editing_an_old_oracle_record_moves_its_wallet_out_of_tls` | the form |

### 7.6 Limits

- **Encrypted wallet keys.** No wallet password yet. Supporting one means
  another secret, which must follow §3.7: kept in the keychain under its own
  account, never in a file or an action. Then pass it to
  `Config::set_wallet_password`.
- **No custom server DN.** `SSL_SERVER_CERT_DN` is not exposed. The host-name
  check is what guards the connection.
- **OS trust store.** Not consulted. A corporate CA installed system-wide has
  to be put in a wallet.
- **No live test yet.** The tests cover the TLS layer against a real rustls
  server, but not a TCPS listener on an actual Oracle database.
- **Other backends.** PostgreSQL, MySQL and MariaDB still refuse `require`
  and stricter (§2.3).
