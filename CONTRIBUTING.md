# Contributing to qry

Before sending a change, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features --workspace --locked -- -D warnings
QRY_KEYRING=off cargo test --workspace --all-features --locked
cargo deny --all-features check
```

Commits use conventional-commit prefixes (`feat:`, `fix:`, `perf:`,
`refactor:`, `docs:`, `test:`, `ci:`, `build:`, `chore:`).

## Writing tests

### Where tests go

- **Unit tests** live in a sibling file, not in an inline `mod tests {}`. For
  `src/foo.rs` the tests are in `src/foo_tests.rs`, declared at the bottom of
  `foo.rs`:

  ```rust
  #[cfg(test)]
  #[path = "foo_tests.rs"]
  mod foo_tests;
  ```

  The `#[path]` is needed for any file other than `lib.rs`, because a plain
  `mod foo_tests;` inside `foo.rs` would look for `foo/foo_tests.rs`. The
  `no_inline_test_modules` check in `crates/qry/tests/workspace_metadata.rs`
  enforces this.
- **Integration tests** live in `crates/<crate>/tests/*.rs`. Helpers they
  share go in `tests/common/mod.rs`, which starts with
  `#![allow(dead_code, unreachable_pub)]` because not every test file uses
  every helper.
- Every test file starts with
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`: a
  failing `unwrap` is a fine way for a test to fail.
- Test names are descriptive snake_case sentences, such as
  `a_failed_export_keeps_the_modal_and_the_path`.

### The test toolkit

- `qry::testing` has `AppHarness`, which feeds keys and events to
  `App::handle` the way the runtime does, without a terminal, and records
  the effects instead of carrying them out, plus `buffer_to_string`,
  `styled`, `parse_keys`, `render_with` and the `SIZES` every view must
  render at. Its module docs have an example.
- `qry_core::testing` has `FakeDatabase`, a `Database` that answers from a
  script of `Reply`s (rows, an error, a delay, a hang) and records every
  statement it was sent, plus the `result()` and `grid()` builders for
  readable fixtures.
- Both modules exist only for tests and behind the `test-util` feature. A
  release build never contains them. Within the workspace, `qry` lists
  itself as a dev-dependency with `test-util` on, so its tests always have
  them.

### Snapshots

Whole screens are checked with [insta](https://insta.rs) snapshots rather
than with `contains` on the text. Keep a targeted `contains` where only one
fact matters.

- Name snapshots after what they show and the terminal size:
  `home_empty_80x24`, `help_120x40`.
- Use `testing::styled` instead of `testing::buffer_to_string` when the
  colours are the point. It adds one line per style run under the text.
- Snapshots live in `snapshots/` next to the test file and are committed.
  Pending ones (`*.snap.new`) are git-ignored.
- After a change that alters a screen, run the tests and then review:

  ```sh
  cargo install cargo-insta   # once
  cargo insta test --workspace --all-features   # or plain cargo test
  cargo insta review          # accept or reject each change
  cargo insta accept          # or accept them all
  ```

- CI runs with `CI=true`, so a missing or changed snapshot fails the build
  instead of being written.

### Time

No test waits in real time. Use the harness's virtual clock
(`AppHarness::advance`), or run async tests under
`#[tokio::test(start_paused = true)]` and move the clock with
`tokio::time::advance`. A `Reply::Delay` from `FakeDatabase` waits on
`tokio::time`, so a paused clock skips it.

### Files, directories and the keychain

- Read fixtures through `env!("CARGO_MANIFEST_DIR")`. Fixtures shared between
  crates live in `tests/fixtures/` at the repository root (`dbeaver/`,
  `sqldeveloper/`, `qry-backup/`, `sql/`), and `qry::testing::fixture`
  gives their path.
- Write scratch files with `tempfile` or under `env!("CARGO_TARGET_TMPDIR")`.
  Tests never write into the repository, or into the user's real config or
  data directories. A test that runs code which reads them sets `QRY_CONFIG`
  and `QRY_DATA` to a temporary directory.
- Run tests with `QRY_KEYRING=off`. It replaces the OS keychain with an
  in-memory store, so no test prompts for, reads or writes the developer's
  real keychain. The `qry` crate's own unit tests always use the in-memory
  store, whatever the variable says.

### Property tests and fuzzing

- Commit the `*.proptest-regressions` files proptest writes, and never
  delete them: they replay the cases that once failed.
- `fuzz/` is a cargo-fuzz workspace of its own, left out of the root one.
  Each target calls one `fuzz_*` function in `qry::fuzz` or
  `qry_core::fuzz`, and the sibling `fuzz_tests.rs` runs the same function
  as a proptest on stable, so the fuzz code cannot rot. With nightly and
  `cargo install cargo-fuzz`:

  ```sh
  fuzz/seed-corpus.sh
  cargo +nightly fuzz run sql_split fuzz/corpus/sql_split -- -max_total_time=60
  ```

- A crash input goes into `tests/fixtures/fuzz-regressions/<target>/`,
  where `known_crash_inputs_stay_fixed` replays it, next to the fix.
- **A new parser of untrusted input** comes with all five of:
  1. a `fuzz_*` entry point,
  2. a target in `fuzz/fuzz_targets/` and a `[[bin]]` in `fuzz/Cargo.toml`,
  3. seeds in `fuzz/seed-corpus.sh`,
  4. a proptest twin in `fuzz_tests.rs`,
  5. a line in the matrix of `.github/workflows/fuzz.yml`.

  `crates/qry/tests/fuzz_inventory.rs` fails when the names do not line up.
