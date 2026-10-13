# T005 — Property tests and fuzzing

**Phase:** 0 — Foundation · **Depends on:** T002, T003 · **Blocks:** none (T302 adds its lexer targets here)

## Description

qry parses a lot of input it does not control, and none of that parsing is fuzzed or property-tested today:

- **Connection imports** in `crates/qry/src/import.rs` (894 lines): qry backups, Oracle SQL Developer `connections.json`, and DBeaver `data-sources.json` (`Format::{Qry, SqlDeveloper, DBeaver}`, plus `Auto` detection). These files come from other tools and other machines.
- **The stored connections file**, through `connections::parse` (`crates/qry/src/connections.rs:281`).
- **User key bindings**, through `config::parse_key_sequence` (`crates/qry/src/config.rs:308`) and `parse_style` (`config.rs:359`), applied to whatever the user writes in `config.json5/yaml/toml`.
- **SQL text** from T302: a lexer for highlighting and a statement splitter for "run statement under cursor". These run on every keystroke, so a panic there takes the whole TUI down with the user's unsaved query.

A panic in any of these is a crash with the terminal in raw mode. This task adds fuzz targets and proptest "twins", following sverb:

- `~/Projects/sverb/fuzz/` is a separate cargo-fuzz workspace with 13 `[[bin]]` targets, `seed-corpus.sh`, and a `.gitignore`.
- Each fuzz body is a public `fuzz_*` function, also run by a proptest on stable (`crates/sverb-term/src/fuzz.rs`), so fuzz code cannot rot.
- `fuzz-smoke` runs 30 s per target in `ci.yml`.
- The nightly `fuzz.yml` runs 600 s per target with a cached, minimised corpus.

### 1. Fuzz entry points live in the crates

Each crate gets a `fuzz` module, public but `#[doc(hidden)]`, so both the cargo-fuzz targets and the proptests call exactly the same code:

```rust
// crates/qry/src/fuzz.rs
#![doc(hidden)]
//! Entry points for `fuzz/` and their proptest twins. Each must never panic,
//! whatever the input; returning an error is fine.

pub fn fuzz_import_qry(data: &[u8])         { let _ = crate::import::from_bytes(data, Format::Qry); }
pub fn fuzz_import_dbeaver(data: &[u8])     { let _ = crate::import::from_bytes(data, Format::DBeaver); }
pub fn fuzz_import_sqldeveloper(data: &[u8]) { let _ = crate::import::from_bytes(data, Format::SqlDeveloper); }
pub fn fuzz_import_auto(data: &[u8])        { let _ = crate::import::from_bytes(data, Format::Auto); }
pub fn fuzz_connections_parse(data: &[u8])  { let text = String::from_utf8_lossy(data);
                                              let _ = crate::connections::parse(&text, Path::new("fuzz.json")); }
pub fn fuzz_keymap_parse(data: &[u8])       { let text = String::from_utf8_lossy(data);
                                              if let Ok(keys) = crate::config::parse_key_sequence(&text) {
                                                  // round-trip: display then parse gives the same keys
                                                  let shown: Vec<_> = keys.iter().map(crate::config::key_event_to_string).collect();
                                                  let again = crate::config::parse_key_sequence(&shown.concat());
                                                  assert_eq!(again.as_deref(), Ok(&keys[..]));
                                              } }
pub fn fuzz_style_parse(data: &[u8])        { let _ = crate::config::parse_style(&String::from_utf8_lossy(data)); }
```

```rust
// crates/qry-core/src/fuzz.rs  (filled in by T302, which moves the SQL lexer to qry-core)
pub fn fuzz_sql_lex(data: &[u8]);    // lex(text) never panics; token spans tile the input exactly
pub fn fuzz_sql_split(data: &[u8]);  // split(text) never panics; statement ranges are in bounds,
                                     // non-overlapping, ascending, and land on char boundaries
```

`import::from_file` currently reads a path. Add `import::from_bytes(&[u8], Format)` and have `from_file` call it, so the parsers can run without disk I/O. Do the same for anything else that only takes a path.

Some invariants are checked inside the fuzz bodies, beyond "doesn't panic", because they are the bugs that matter:

- **Key bindings** must round-trip through display and parse, as in `fuzz_keymap_parse` above.
- **SQL lexing:** the concatenation of all token texts equals the input.
- **SQL splitting:** every byte that is not whitespace or a comment belongs to exactly one statement.
- **Imports:** the result never holds a password. Assert `Secret::None | Secret::Keyring | Secret::Env` only, never a plaintext field. This is the same guarantee the import code documents.

### 2. The `fuzz/` workspace

```
fuzz/
  Cargo.toml          # cargo-fuzz = true, own [workspace], excluded from the root
  .gitignore          # target/ corpus/ artifacts/ coverage/ Cargo.lock
  seed-corpus.sh
  fuzz_targets/
    import_qry.rs  import_dbeaver.rs  import_sqldeveloper.rs  import_auto.rs
    connections_parse.rs  keymap_parse.rs  style_parse.rs
    sql_lex.rs  sql_split.rs          # added once T302 lands; listed here so the matrix is ready
```

The root `Cargo.toml` gets `exclude = ["fuzz"]`. `fuzz/Cargo.toml`:

```toml
[package]
name = "qry-fuzz"
version = "0.0.0"
publish = false
edition = "2024"

[package.metadata]
cargo-fuzz = true

[dependencies]
libfuzzer-sys = "0.4"
qry = { path = "../crates/qry", default-features = false }
qry-core = { path = "../crates/qry-core" }

[workspace]
members = ["."]

[profile.release]
debug = 1
panic = "abort"

[[bin]]
name = "import_dbeaver"
path = "fuzz_targets/import_dbeaver.rs"
test = false
doc = false
bench = false
# … one [[bin]] per target
```

A target is one line:

```rust
#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| qry::fuzz::fuzz_import_dbeaver(data));
```

**`seed-corpus.sh`.**

- It copies the T002 fixtures into `fuzz/corpus/<target>/`:
  - `tests/fixtures/dbeaver/*` → `import_dbeaver` and `import_auto`
  - `tests/fixtures/sqldeveloper/*` → `import_sqldeveloper` and `import_auto`
  - `tests/fixtures/qry-backup/*` → `import_qry` and `import_auto`
  - `tests/fixtures/sql/*` → `sql_lex` and `sql_split`
  - `.config/config.json` → `keymap_parse` seeds, one file per binding string, extracted with `jq`
- It writes hand-made seeds with `printf`:
  - SQL: an unterminated string, a nested block comment, a dollar-quoted body, `$$`, `q'[…]'` Oracle quoting
  - JSON: deeply nested arrays
  - key bindings: odd strings such as `<ctrl-shift-alt-f12>` and `<<>>`
- It must be idempotent.

### 3. Proptest twins

Each crate's `fuzz_tests.rs` (sibling of `fuzz.rs`) runs every `fuzz_*` body over arbitrary bytes and over structured inputs:

```rust
proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    #[test]
    fn import_dbeaver_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        fuzz_import_dbeaver(&data);
    }
}
```

Structured strategies find more than random bytes:

- **`arb_json_connections()`** generates valid-shaped DBeaver, SQL Developer and qry JSON with random field values: missing keys, wrong types, huge ports, unicode names.
- **`arb_key_sequence()`** generates `<mod-…-key>` strings from the real key-name table.
- **`arb_sql()`** (T302) generates statements joined by `;` with strings, comments and quoted identifiers mixed in.

Commit the `*.proptest-regressions` files that proptest writes. CONTRIBUTING (T401) says never to delete them.

Some properties are worth their own proptests, not just "doesn't panic":

- **`import_of_an_export_is_the_identity`.** For any `Vec<StoredConnection>` from `arb_stored_connection()`:
  1. `connections::export` to a temporary file,
  2. `import::from_file(Format::Qry)`,
  3. the result has the same connections modulo `id`.

  This guards the backup feature from commit `1d639ec`.
- **`connections_save_then_load_round_trips`**, using `save_to` and `load_from` on a temporary directory.
- **`keybinding_display_parse_round_trip`**, over `arb_key_sequence()`.
- **`auto_detection_agrees_with_explicit_format`.** For generated valid files of each format, `Format::Auto` picks the same format as the explicit one.

### 4. CI

**`fuzz-smoke` in `ci.yml`** (the T003 placeholder becomes real):

```yaml
  fuzz-smoke:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: dtolnay/rust-toolchain@nightly
      - uses: taiki-e/install-action@v2
        with: { tool: cargo-fuzz }
      - uses: Swatinem/rust-cache@v2
        with: { workspaces: fuzz }
      - run: fuzz/seed-corpus.sh
      - run: cargo +nightly fuzz build --target x86_64-unknown-linux-gnu
      - name: Run every target for 30 s
        run: |
          status=0
          for target in $(cargo +nightly fuzz list); do
            echo "::group::$target"
            if ! cargo +nightly fuzz run --target x86_64-unknown-linux-gnu "$target" "fuzz/corpus/$target" -- \
                 -max_total_time=30 -rss_limit_mb=2048 -timeout=10; then
              echo "::error::fuzz target $target crashed"; status=1
            fi
            echo "::endgroup::"
          done
          exit $status
      - uses: actions/upload-artifact@v4
        if: failure()
        with: { name: fuzz-crashes, path: fuzz/artifacts, if-no-files-found: ignore }
```

**`.github/workflows/fuzz.yml`** (nightly):

- Triggers: `schedule: cron "17 3 * * *"` and `workflow_dispatch` with an input `seconds` (default `"600"`).
- One job per target via a matrix, `fail-fast: false`, `timeout-minutes: 30`.
- The corpus persists in `actions/cache@v4`, with `key: fuzz-corpus-${{ matrix.target }}-${{ github.run_id }}` and `restore-keys: fuzz-corpus-${{ matrix.target }}-`.
- On success, `cargo +nightly fuzz cmin` minimises the corpus. On failure, crashes are uploaded as `fuzz-crashes-<target>`.

### Contribution rule

Write this into CONTRIBUTING (T401). A new parser of untrusted input must come with:

1. a `fuzz_*` entry point,
2. a target and a `[[bin]]`,
3. seeds in `seed-corpus.sh`,
4. a proptest twin,
5. a line in the `fuzz.yml` matrix.

`crates/qry/tests/fuzz_inventory.rs` enforces this (see Tests).

### Out of scope

- The SQL lexer and splitter themselves (T302). This task reserves their target names and the matrix entries.
- Coverage-guided corpus reports.

## Acceptance criteria

- [ ] `crates/qry/src/fuzz.rs` exists with the seven `fuzz_*` functions above, and `crates/qry-core/src/fuzz.rs` has stubs for `fuzz_sql_lex` and `fuzz_sql_split` (filled in by T302).
- [ ] `import::from_bytes` exists, and `from_file` delegates to it.
- [ ] `fuzz/` builds with `cargo +nightly fuzz build`, and `cargo +nightly fuzz list` lists every target named in `fuzz.yml`'s matrix.
- [ ] The root workspace excludes `fuzz`, and `cargo build --workspace` does not build it.
- [ ] `fuzz/seed-corpus.sh` is idempotent, and gives every target a non-empty corpus directory.
- [ ] Every `fuzz_*` function has a proptest twin, and `cargo test --workspace` runs them on stable.
- [ ] The four property tests in section 3 exist and pass.
- [ ] `fuzz-smoke` in `ci.yml` runs every target for 30 s and is green. A deliberately planted `panic!` in one importer makes it red, with a crash artifact uploaded (verify once on a throwaway branch).
- [ ] `fuzz.yml` runs nightly and on `workflow_dispatch`, persists the corpus through the cache, and minimises it on success.
- [ ] Any crash found while doing this task is fixed, with its input added as a regression test (in `tests/fixtures/fuzz-regressions/<target>/` and replayed by a unit test).
- [ ] `*.proptest-regressions` files are committed.

## Tests

`crates/qry/src/fuzz_tests.rs`, proptest twins at 256 cases each:

- **`import_qry_never_panics`**, **`import_dbeaver_never_panics`**, **`import_sqldeveloper_never_panics`**, **`import_auto_never_panics`**: over random bytes and over `arb_json_connections()`.
- **`connections_parse_never_panics`**
- **`keymap_parse_never_panics_and_round_trips`**
- **`style_parse_never_panics`**

Structured properties (`crates/qry/tests/import_props.rs`, with `ProptestConfig::with_cases(512)`):

- **`import_of_an_export_is_the_identity`**
- **`connections_save_then_load_round_trips`**
- **`keybinding_display_parse_round_trip`**
- **`auto_detection_agrees_with_explicit_format`**
- **`imports_never_carry_a_plaintext_password`**: for generated DBeaver and SQL Developer files that contain password fields, every imported `StoredConnection.secret` is `None`, `Keyring` or `Env`, and its serialised form does not contain the generated password string.

Regression replay (`crates/qry/src/fuzz_tests.rs`):

- **`known_crash_inputs_stay_fixed`**: reads every file under `tests/fixtures/fuzz-regressions/<target>/` and runs the matching `fuzz_*` function on it.

Inventory (`crates/qry/tests/fuzz_inventory.rs`):

- **`every_fuzz_entry_point_has_a_target_and_a_matrix_line`**: parses `fuzz/Cargo.toml` `[[bin]]` names and `.github/workflows/fuzz.yml`'s matrix, and greps `pub fn fuzz_` in both crates. All three sets must be equal (`fuzz_x` ↔ target `x`).

CI:

- `fuzz-smoke` green on every PR.
- `fuzz.yml` green on its first scheduled run, linked in the PR follow-up.
