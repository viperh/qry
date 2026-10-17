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

- [x] `crates/qry/src/fuzz.rs` exists with the seven `fuzz_*` functions above, and `crates/qry-core/src/fuzz.rs` has stubs for `fuzz_sql_lex` and `fuzz_sql_split` (filled in by T302).
- [x] `import::from_bytes` exists, and `from_file` delegates to it.
- [x] `fuzz/` builds with `cargo +nightly fuzz build`, and `cargo +nightly fuzz list` lists every target named in `fuzz.yml`'s matrix.
- [x] The root workspace excludes `fuzz`, and `cargo build --workspace` does not build it.
- [x] `fuzz/seed-corpus.sh` is idempotent, and gives every target a non-empty corpus directory.
- [x] Every `fuzz_*` function has a proptest twin, and `cargo test --workspace` runs them on stable.
- [x] The four property tests in section 3 exist and pass.
- [ ] `fuzz-smoke` in `ci.yml` runs every target for 30 s and is green. A deliberately planted `panic!` in one importer makes it red, with a crash artifact uploaded (verify once on a throwaway branch).
- [ ] `fuzz.yml` runs nightly and on `workflow_dispatch`, persists the corpus through the cache, and minimises it on success.
- [x] Any crash found while doing this task is fixed, with its input added as a regression test (in `tests/fixtures/fuzz-regressions/<target>/` and replayed by a unit test).
- [x] `*.proptest-regressions` files are committed.

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

## Implementation notes

- **Three boxes stay unticked because this machine has no nightly toolchain and no cargo-fuzz** (and the network and disk were too limited to install them). `fuzz/` is complete, but `cargo +nightly fuzz build`/`list`, the 30 s `fuzz-smoke` run, the planted-panic check on a throwaway branch and the first nightly `fuzz.yml` run have not happened. What was checked: a stable `cargo check --bins` inside `fuzz/` type-checks all 11 targets against `libfuzzer-sys 0.4.13`; `fuzz_inventory` checks that the `[[bin]]`s, the `fuzz.yml` matrix and the `pub fn fuzz_*` entry points name the same 11 targets, and that each target calls its own entry point. The first person with nightly should run `fuzz/seed-corpus.sh && cargo +nightly fuzz build && cargo +nightly fuzz list`, plant a `panic!` in `import::dbeaver` on a throwaway branch to see `fuzz-smoke` go red with a `fuzz-crashes` artifact, and tick the three boxes. T302's "property-tested and fuzzed" box should be ticked once `sql_lex`/`sql_split` have run under libFuzzer.
- **Targets (11).** `qry::fuzz`: `import_qry`, `import_dbeaver`, `import_sqldeveloper`, `import_auto`, `connections_parse`, `keymap_parse`, `palette_score`. `qry_core::fuzz`: `sql_lex`, `sql_split`, `grid`, `copy_render`. Differences from the spec: `fuzz_style_parse` is gone, because T201 removed `parse_style`. `fuzz_keymap_parse` uses `keymap::chord_to_string` and `keymap::parse_key_sequence` (T102 moved the parser and removed `key_event_to_string`). `palette_score` is T206's matcher: query, NUL, then the text. `grid` and `copy_render` cover T303's groundwork, because a database's answers are untrusted input too. `qry_core::fuzz::result_from_bytes` turns bytes into a `QueryResult`: the column count, a kind per column, two choice bytes, then names and cells split by `0xFF`, with a lone `0xFE` meaning NULL. The SQL bodies are full implementations, not stubs. Both `fuzz` modules are `pub` and `#[doc(hidden)]`.
- **Invariants checked inside the bodies.**
  - Imports: every record's `Secret` is one of its four variants, none of which can hold a password, and the serialised record has no key named like a password at any depth (`password`, `passwd`, `pwd`, any case).
  - Key bindings: the printed chord parses back to the same keys.
  - Palette: positions are increasing, in bounds, and one per query character.
  - `sql_lex`: in all four dialects, the tokens are non-empty, contiguous and on char boundaries, and their texts concatenate to the input.
  - `sql_split`: in all four dialects, spans are in bounds, on char boundaries, ascending, non-overlapping and equal to their `text`, and `is_ddl` and `statement_at` answer. For PostgreSQL and SQLite, every byte that is not whitespace, a comment or `;` is in exactly one statement (an unterminated token's trailing blanks excepted, as T302 defined). MySQL `DELIMITER` lines and Oracle `/` lines are deliberately in no statement, so those two dialects get only the structural checks.
  - `grid`: widths stay within the cap, `visible_columns` fits the room for any widths and room taken straight from the bytes, the sort is a permutation, and the search finishes, with matches in order that really contain the needle.
  - `copy_render`: covers every format and dialect, with a row and a column out of range. TSV and Markdown give one line per row plus their header lines.
- **Bugs found and fixed.**
  - `grid::sort_permutation` panicked on a numeric column holding `NaN`. `NaN` compared equal to every number, so the comparison was not a total order, and std's sort panics when it detects that. The `grid` proptest twin found it on its first run. `NaN` now sorts with the text, after the numbers. It has its own commit. The input is in `tests/fixtures/fuzz-regressions/grid/nan-in-a-numeric-sort.bin` (21 values, minimised by hand; the replay test fails without the fix), and the unit test is `grid_tests::sort_numeric_with_nan_puts_it_after_the_numbers`.
  - A qry backup could carry a plaintext password into the connection file. `StoredConnection.extra` keeps unknown keys, so a hand-edited backup's `"password": "…"` was imported and saved. This was found by reasoning about the import invariant, not by a fuzzer run. A `Format::Qry` import now drops every field named like a password, at any depth (`import::strip_passwords`). The input is in `fuzz-regressions/import_qry/` and `import_auto/`, and the unit test is `fuzz_tests::a_password_in_a_hand_edited_backup_is_dropped`. `connections::load_from` still keeps unknown fields of qry's own file as before.
- **`import::from_bytes(&[u8], Format)`** converts without disk I/O; its errors name "the file". `from_file` reads the file and then calls the same body (`from_named_bytes`) with the real path, so its errors still name the file (the zip and not-UTF-8 checks moved there). To be reached from integration tests, `import::from_file` and `connections::{parse, load_from, save_to, export}` are now `pub` instead of `pub(crate)`.
- **Proptest twins.** These are `crates/qry/src/fuzz_tests.rs` and `crates/qry-core/src/fuzz_tests.rs`, at 256 cases each. Every twin runs its body over random bytes (up to 4 KiB) and over a structured strategy:
  - `arb_json_connections()`: DBeaver, SQL Developer and qry shapes with any subset of the real keys, values of every JSON type, out-of-range numbers, unicode, nesting and a table of JDBC URLs.
  - `arb_key_text()`: the real key names, modifiers in mixed case, both spellings, and junk characters.
  - `arb_sql()`: dialect pieces such as dollar quotes, `q'[`, `DELIMITER`, `/` lines, PL/SQL words and open quotes, joined by `;`.
  - `arb_result_bytes()`: NaN, infinities, leading zeros, natural-sort text, NULLs and line breaks.

  A 2,000-case run of every twin found nothing beyond the NaN bug. `known_crash_inputs_stay_fixed` in each crate replays its own targets' directories under `tests/fixtures/fuzz-regressions/`. The `qry` one fails on a directory that names no target. `sql_fixtures_pass_the_fuzz_bodies` runs the SQL fixtures through both SQL bodies.
- **Structured properties.** `crates/qry/tests/import_props.rs` runs 512 cases: the four tests in section 3 plus `imports_never_carry_a_plaintext_password`. `arb_stored_connection()` generates every field, all four `Secret`s, and `extra` fields named `x_…`, because an import drops keys named like a password and a known field name would not survive the flatten. `arb_key_sequence()` only produces text that must parse; it asserts that the text parses, then that it round-trips. The auto-detection test also compares connections, modulo the freshly minted ids, and the skipped entries.
- **Fixtures.** `tests/fixtures/sql/` is new. It holds `oracle-plsql.sql` (procedure and anonymous block ended by `/`, `q'[…]'`), `postgres-dollar-quoted.sql` (`$$` and `$body$` bodies, `E''`, nested comment), `mysql-delimiter.sql` and `sqlite-mixed.sql` (brackets, blobs, unicode, an unterminated string). T302's notes named `crates/qry-core/tests/fixtures/sql/`, but CONTRIBUTING and this task use the shared root `tests/fixtures/sql/`, so the files are there.
- **`fuzz/seed-corpus.sh`.**
  - It copies the fixtures as the spec lists. qry backups also seed `connections_parse`, and each regression input seeds its own target.
  - It extracts every binding from `.config/config.json` with `jq`, after dropping its whole-line `//` comments (the file is JSON5).
  - It adds hand-made seeds for every target, including the grid and copy byte layout.
  - Each seed has a fixed name: the fixture's own, or `seed-<sha1 prefix>`. A second run gave a byte-identical corpus.
  - It fails if any target is left empty.
- **CI.** `fuzz-smoke` follows the spec's snippet: nightly, cargo-fuzz, a cache keyed on `fuzz`, the seed script, then `build` and `run` with `--target x86_64-unknown-linux-gnu`, uploading `fuzz/artifacts` on failure. The T003 presence gate and the `RUSTUP_TOOLCHAIN` override are gone, since `+nightly` overrides `rust-toolchain.toml`. `fuzz.yml` has these triggers and steps:
  - It runs on `cron "17 3 * * *"` and on `workflow_dispatch` with an input `seconds` (default 600).
  - It has one matrix job per target, with `fail-fast: false` and `timeout-minutes: 30`.
  - The corpus is cached with `actions/cache@v4` under key `fuzz-corpus-<target>-<run_id>`, with restore-keys `fuzz-corpus-<target>-`.
  - It seeds, builds, runs and then runs `cargo +nightly fuzz cmin` on success, uploading `fuzz-crashes-<target>` on failure.
  - The cache saves at the end of a green job, so after `cmin`.
- **Proptest regressions.** The existing files stay committed. The one new failure, the NaN case, was a 1 KB random seed. It was replaced by the minimal fixture and unit test above, so no new `*.proptest-regressions` file was added.
- **CONTRIBUTING** gained a "Property tests and fuzzing" section: committing regression files, running a target, where crash inputs go, and the five-part rule for a new parser, which `fuzz_inventory` enforces.
- **Checks run locally:**
  - `cargo fmt --all`. It ran on stable, so the nightly-only import grouping options were skipped.
  - clippy with `-D warnings`.
  - `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked`: 413 `qry` unit tests, 3 in `fuzz_inventory`, 5 in `import_props`, 98 in `qry-core`.
  - The CI docs command with `RUSTDOCFLAGS=-D warnings`.
  - `cargo check --bins` in `fuzz/` on stable.

  Because the disk was shared and nearly full, these ran with `CARGO_INCREMENTAL=0` and, for clippy and tests, `CARGO_PROFILE_{DEV,TEST}_DEBUG=0`.

Fuzzing run (later): nightly (`cargo 1.101.0-nightly 2026-10-12`) and `cargo-fuzz` were installed. `fuzz/seed-corpus.sh` seeded all 11 targets, `cargo +nightly fuzz build --fuzz-dir fuzz` built them, and `cargo +nightly fuzz list` printed the 11 names that `fuzz_inventory` matches against `fuzz.yml`'s matrix, so the third box is ticked. Each target then ran for 60 s (`-max_total_time=60`, one job). There were no crashes, timeouts or OOMs, and `fuzz/artifacts` stayed empty. Runs per target: `connections_parse` 1.82 M, `copy_render` 99 k, `grid` 483 k, `import_auto` 1.64 M, `import_dbeaver` 1.74 M, `import_qry` 1.88 M, `import_sqldeveloper` 1.64 M, `keymap_parse` 1.83 M, `palette_score` 704 k, `sql_lex` 297 k, `sql_split` 87 k. No new regression inputs were needed. `fuzz/target`, the corpus and `fuzz/Cargo.lock` were deleted afterwards. The `fuzz-smoke` box (CI and the planted panic) and the `fuzz.yml` box still need GitHub runs. On Rust 1.99, clippy's new `chunks_exact_to_as_chunks` lint rejected `qry_core::fuzz::fuzz_grid`'s `chunks_exact(2)`, so it now uses `as_chunks::<2>()`.

Schedule removed at the owner's request: `fuzz.yml` no longer runs on a cron;
it runs only by hand (`workflow_dispatch`, Actions → Run workflow). The
boxes about it running nightly no longer apply as written.

