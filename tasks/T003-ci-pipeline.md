# T003 — CI pipeline

**Phase:** 0 — Foundation · **Depends on:** T001, T002 · **Blocks:** T004, T005, T006, T007

## Description

`.github/workflows/ci.yml` is the ratatui-template CI. It has four jobs: `test`, `rustfmt`, `clippy` and `docs`.

**Its push trigger points at a branch that does not exist.**

```yaml
on:
  push:
    branches:
      - main        # the default branch is `master`
  pull_request:
```

The default branch is `master`, so pushes to it never run CI; only pull requests do. The README's CI badge (`workflows/CI/badge.svg`) therefore reflects PR runs at best.

It also lacks things sverb's `ci.yml` has:

- No `--locked` on clippy or docs.
- No `concurrency` group and no `permissions` block.
- No database services: the Postgres, MySQL, MariaDB and Oracle drivers in `crates/qry-core` are only covered by whatever unit tests run without a server.
- No supply-chain check, no MSRV check and no layering check.

This task rewrites `ci.yml` after `~/Projects/sverb/.github/workflows/ci.yml`. It also adds the scripts and the Rust structural test the jobs call. Jobs owned by later tasks (`db`, `fuzz-smoke`, `bench-build`) are added here as part of the workflow, and their bodies are filled in by T004, T005 and T006. Until then they run their current minimal form, described per job below.

### Workflow header

```yaml
name: CI
on:
  push:
    branches: [master]
  pull_request:
  workflow_dispatch:
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true
permissions:
  contents: read
env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1
  CARGO_INCREMENTAL: 0
  QRY_KEYRING: "off"
```

Every job runs on `ubuntu-latest` and uses `actions/checkout@v4`, `dtolnay/rust-toolchain@stable` and `Swatinem/rust-cache@v2`.

- Jobs that compile the `qry` crate need `fetch-depth: 0`, because `crates/qry/build.rs` runs vergen-gix, which reads git history.
- Every cargo invocation uses `--locked`.

### Jobs

| Job | Commands |
|---|---|
| `fmt` | `cargo fmt --all --check` |
| `clippy` | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, then `cargo clippy --workspace --all-targets --no-default-features --locked -- -D warnings` |
| `docs` | `RUSTDOCFLAGS: -D warnings`, then `cargo doc --no-deps --document-private-items --all-features --workspace --locked` |
| `test` | `cargo test --workspace --all-features --locked` with `QRY_CONFIG`/`QRY_DATA` under `${{ runner.temp }}`. Runs unit tests, SQLite integration tests and snapshots. Real-server DB tests skip here because the `QRY_TEST_*_URL` variables are unset; their `db_or_skip!` macro (T004) would panic under `CI`, so this job sets `QRY_DB_TESTS: "skip"`, which the macro honours as an explicit opt-out |
| `db` | Service containers for postgres, mysql, mariadb and oracle; `cargo test -p qry-core --all-features --locked --test 'db_*'`. Defined fully in T004. Until then: the postgres service only, plus the existing `qry-core` tests |
| `msrv` | Reads `rust-version` from `cargo metadata` (snippet below), installs that toolchain, then `cargo +$MSRV check --workspace --all-features --locked` |
| `deny` | `EmbarkStudios/cargo-deny-action@v2` with `command: check advisories bans licenses sources` and `arguments: --all-features --locked` |
| `layering` | `cargo test -p qry --test workspace_metadata --locked`, then `python3 scripts/check-layering.py` |
| `fuzz-smoke` | Nightly toolchain, `cargo-fuzz`; builds every target and runs each for 30 s. Defined in T005; until then the job is skipped with `if: hashFiles('fuzz/Cargo.toml') != ''` |
| `bench-build` | `cargo bench --workspace --no-run --locked`, then `python3 scripts/bench-gate.py self-test`. Defined in T006; until then only `--no-run` |
| `windows-build` | `windows-latest`: `cargo build -p qry --locked` and `cargo test -p qry --locked --lib`. Keeps the `winresource` build script and the `cfg(not(windows))` `signal-hook` code honest. It is the one non-Linux job; the CD matrix covers macOS |

**MSRV step** (from sverb):

```yaml
      - name: Read rust-version
        id: msrv
        run: |
          msrv="$(cargo metadata --no-deps --format-version 1 --locked \
            | jq -r '[.packages[].rust_version // empty] | unique
                     | if length == 1 then .[0]
                       else error("workspace crates disagree on rust-version: \(.)") end')"
          echo "version=$msrv" >> "$GITHUB_OUTPUT"
      - uses: dtolnay/rust-toolchain@master
        with: { toolchain: "${{ steps.msrv.outputs.version }}" }
      - uses: Swatinem/rust-cache@v2
        with: { key: "msrv-${{ steps.msrv.outputs.version }}" }
      - run: cargo +${{ steps.msrv.outputs.version }} check --workspace --all-features --locked
```

**A final `ci-ok` job** has `needs:` on every job and `if: always()`. It fails if any needed job failed or was cancelled. Branch protection requires only `ci-ok`, so adding a job later doesn't need a settings change. Document this in a comment at the top of the job.

### Layering checks

These enforce the rule in the README: "`qry-core` must never depend on `ratatui`, `crossterm` or `clap`". `deny.toml` (T001) already blocks it through `wrappers`; these are the second, independent check, as sverb does with `crates/sverb-e2e/tests/workspace_metadata.rs` plus `scripts/check-layering.py`.

**`crates/qry/tests/workspace_metadata.rs`.** Extends T001's `workspace_lints.rs`; merge the two files into this one. It uses `cargo_metadata` to resolve the dependency graph with all features and asserts:

- `qry-core`'s transitive normal dependencies contain none of `ratatui`, `crossterm`, `clap`, `ratatui-textarea`, `keyring`.
- No workspace crate depends on `qry`.
- Every workspace crate has `license = "MIT"`, a `rust-version`, and `[lints] workspace = true`.

**`scripts/check-layering.py`.**

- Stdlib only. Runs `cargo metadata --format-version 1 --locked` twice, once with `--all-features` and once with `--no-default-features`, and walks the resolve graph against a small rules table at the top of the file.
- Prints each violation as `qry-core -> ratatui (via foo -> bar)` and exits 1 on any.
- Has `--self-test`, which runs the checker on a synthetic metadata JSON with one planted violation and asserts it is found. CI runs the self-test first.

### README and badge

Update the badge to `https://github.com/viperh/qry/actions/workflows/ci.yml/badge.svg?branch=master`.

Replace the README "Checks" block with the local equivalents of every CI job, so a contributor can run the same gates (T401 owns the final wording):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
QRY_KEYRING=off cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --all-features --workspace --locked
cargo deny check
python3 scripts/check-layering.py
```

### Out of scope

- Release workflows (T007).
- The nightly `fuzz.yml` (T005) and `bench.yml` (T006).
- An OS test matrix. sverb dropped its per-OS test matrix; qry keeps the single Windows build job and relies on CD for macOS.

## Acceptance criteria

- [ ] `ci.yml` triggers on push to `master`, on pull requests and on `workflow_dispatch`. A push to `master` produces a run (link one in the PR).
- [ ] The workflow has `concurrency` with `cancel-in-progress: true`, top-level `permissions: contents: read`, and the env block above.
- [ ] Jobs `fmt`, `clippy` (both feature sets), `docs`, `test`, `db`, `msrv`, `deny`, `layering`, `fuzz-smoke`, `bench-build`, `windows-build` and `ci-ok` exist, and all pass on `master`.
- [ ] `grep -n "cargo " .github/workflows/ci.yml | grep -v -- "--locked"` returns only `cargo fmt` and `cargo fuzz` lines.
- [ ] The `msrv` job fails when one crate's `rust-version` is changed to differ (verify once on a throwaway branch).
- [ ] The `layering` job fails when `ratatui` is added to `qry-core`'s dependencies (verify once on a throwaway branch; both the Rust test and the script must catch it).
- [ ] `scripts/check-layering.py --self-test` exits 0, and `scripts/check-layering.py` exits 0 on the real workspace.
- [ ] `ci-ok` fails when any other job fails, and is the only required status check (record the branch-protection setting in the PR description).
- [ ] The README badge points at `ci.yml` on `master`.
- [ ] A clean CI run of the whole workflow takes under 15 minutes wall time with a warm cache (record the time in the PR).

## Tests

In `crates/qry/tests/workspace_metadata.rs`:

- **`qry_core_has_no_ui_dependencies`**: resolves the graph with all features, then collects `qry-core`'s transitive normal (non-dev, non-build) dependencies. It asserts none of `ratatui`, `crossterm`, `clap`, `ratatui-textarea` or `keyring` is present. On failure it prints the dependency path.
- **`no_workspace_crate_depends_on_the_binary`**
- **`every_crate_inherits_workspace_lints`**, **`every_crate_declares_the_workspace_rust_version`**, **`every_crate_is_mit_licensed`**: moved from T001.

In `scripts/check-layering.py --self-test`:

- A synthetic graph with `qry-core -> foo -> crossterm` must report exactly one violation, with the path `qry-core -> foo -> crossterm`.
- A clean synthetic graph must report none.

Verification of the workflow itself, done once and recorded in the PR:

| Planted fault (throwaway branch) | Job that must go red |
|---|---|
| Unformatted line | `fmt` |
| `let _ = x.unwrap();` in non-test code | `clippy` |
| Broken intra-doc link `[Foo]` | `docs` |
| Failing assertion | `test` |
| `ratatui` added to `qry-core` | `layering` and `deny` |
| Mismatched `rust-version` | `msrv` |
| Code using an API newer than the MSRV | `msrv` |
| Any of the above | `ci-ok` |
