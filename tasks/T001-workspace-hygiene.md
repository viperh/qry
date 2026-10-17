# T001 — Workspace hygiene and lint baseline

**Phase:** 0 — Foundation · **Depends on:** — · **Blocks:** T002, T003, T201 (and everything after them)

## Description

Today the workspace has the bare template setup and nothing that holds the code to a standard:

- No `rustfmt.toml` or `rust-toolchain.toml`, and no `[workspace.lints]` table in the root `Cargo.toml`.
- Clippy only runs with `-D warnings` in CI, and the files are not consistently formatted:
  - `crates/qry/src/app.rs` has a double blank line after the `strum` import.
  - `crates/qry/src/action.rs` imports in a mixed order.
  - `crates/qry/src/components/form.rs` has one-line `fn` bodies inside the test module.
- Dependencies are declared two ways:
  - Some crates pin versions directly instead of going through `[workspace.dependencies]`: `crates/qry/Cargo.toml` pins `ratatui-textarea`, `serde_json`, `zeroize`, `keyring` and `winresource`, and `crates/qry-core/Cargo.toml` pins `tokio-postgres`, `mysql_async`, `oracledb` and others.
  - `strum` is listed with `workspace = true, features=["derive"]` even though the workspace entry already enables `derive`.
- All 173 tests are inline `#[cfg(test)] mod tests { … }` blocks at the bottom of the file they test, e.g. `crates/qry/src/components/home.rs` from line ~330, `results.rs`, `conntree.rs` and `config.rs`.
- No supply-chain checks: no `deny.toml` and no Dependabot.
- `.gitignore` only lists `/target` and `/.data`.
- The `.idea/` directory is committed.

This task sets the baseline every later task builds on. It mirrors sverb's root configuration:

- `~/Projects/sverb/rustfmt.toml`
- `~/Projects/sverb/rust-toolchain.toml`
- the `[workspace.lints]` table in `~/Projects/sverb/Cargo.toml`
- `~/Projects/sverb/deny.toml`
- `~/Projects/sverb/.github/dependabot.yml`

### 1. Toolchain and formatting

`rust-toolchain.toml`:

```toml
# Development toolchain. The minimum supported version (MSRV) is
# `rust-version` in the workspace Cargo.toml; CI checks it separately (T003).
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`rustfmt.toml`:

```toml
edition = "2024"
# Unstable: stable rustfmt ignores them and nightly applies them. The code
# should already follow this layout so both agree.
imports_granularity = "Crate"
group_imports = "StdExternalCrate"
```

Run `cargo fmt --all` once and commit the result **as its own commit**, `style: cargo fmt the workspace`, so the reformatting stays out of reviews and `git blame` (it can go in `.git-blame-ignore-revs`).

### 2. Workspace lints

Add to the root `Cargo.toml`:

```toml
[workspace.lints.rust]
unsafe_code = "deny"
missing_debug_implementations = "warn"
unreachable_pub = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
dbg_macro = "deny"
todo = "warn"
unwrap_used = "warn"
expect_used = "warn"
await_holding_lock = "deny"
large_futures = "warn"
```

Each crate adds `[lints] workspace = true`.

- Fix every warning this surfaces in non-test code. Unwraps that really cannot fail get `.expect()` with an `#[allow(clippy::expect_used, reason = "…")]` on the narrowest item. Others turn into `?` or a handled error.
- `unsafe_code = "deny"` has to hold. `crates/qry/src/tui.rs` and `errors.rs` currently use `libc` for suspend handling; check that they contain no `unsafe` blocks, or move them behind a single audited `#[allow(unsafe_code)]` module with a comment explaining why.
- Every test file and test module starts with:
  ```rust
  #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
  ```

### 3. Dependency centralisation and MSRV

- Every external dependency used by any crate moves into `[workspace.dependencies]`, grouped under the existing comment headings plus `# --- database drivers ---` and `# --- export ---`. Crates reference them only with `{ workspace = true }`, adding `features = [...]` where needed.
- The exact `oracledb` pin (`=26.0.0-beta.4`) and its comment move to the workspace table unchanged.
- One `rust-version` is set in `[workspace.package]`, and every crate has `rust-version.workspace = true`. It is currently `1.89`. Verify that the codebase really builds on 1.89 (edition 2024 and the dependencies may need more); if not, raise it to the lowest version that works and record that version in the commit message. T003's `msrv` job enforces it.
- Remove the redundant `features=["derive"]` on `strum` in `crates/qry/Cargo.toml`.

### 4. Supply chain: `deny.toml`

`deny.toml` mirrors sverb's. The important part for qry is the `wrappers` bans, which keep the UI out of `qry-core`:

```toml
[graph]
all-features = true

[advisories]
version = 2
unmaintained = "workspace"
yanked = "deny"
ignore = []

[licenses]
version = 2
allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause",
         "BSD-3-Clause", "ISC", "Zlib", "Unicode-3.0", "CC0-1.0", "0BSD", "MPL-2.0"]
confidence-threshold = 0.9
unused-allowed-license = "allow"

[licenses.private]
ignore = true

[bans]
multiple-versions = "warn"
wildcards = "allow"
deny = [
  { crate = "ratatui",   wrappers = ["qry", "ratatui-textarea"], reason = "UI belongs to the qry crate only" },
  { crate = "crossterm", wrappers = ["qry", "ratatui", "ratatui-crossterm", "ratatui-textarea"], reason = "UI belongs to the qry crate only" },
  { crate = "clap",      wrappers = ["qry", "clap_complete", "clap_mangen"], reason = "CLI parsing belongs to the qry crate only" },
]

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

- Run `cargo deny check` locally.
- Any license or advisory that fails is either resolved or allowed with a written `reason`. Expect `oracledb`, `rust_xlsxwriter` and the keyring backends to need a look.
- Do not ban `openssl` blindly. Check whether `tokio-postgres` or `mysql_async` pull in `native-tls` under the current features, and record the decision as a comment in `deny.toml`.

### 5. Dependabot and `.gitignore`

`.github/dependabot.yml`:

- Weekly `cargo` updates, patch updates grouped, `open-pull-requests-limit: 10`.
- Weekly `github-actions` updates, all grouped.

`.gitignore` additions:

```
*.snap.new
*.pending-snap
/fuzz/target
/fuzz/corpus
/fuzz/artifacts
/fuzz/coverage
/fuzz/Cargo.lock
/result
/.idea/workspace.xml
```

Decide whether `.idea/` stays tracked. The recommendation is to untrack it apart from shared code-style files, and to note the decision in the commit.

### 6. Moving tests to sibling files

Move every inline `mod tests { … }` out to a sibling file, following sverb's convention:

- `foo.rs` gets `#[cfg(test)] mod foo_tests;` and the body moves to `foo_tests.rs` in the same directory.
- If a module is already a directory module, use `dir/tests.rs`.

This changes no test logic, so the test count must stay exactly the same (record it before and after).

Shared helpers move into the new files they belong to:

- `components::form::tests::key`, `press`, `type_str`, `render` and `lines`, which `home.rs` imports as `crate::components::form::tests::key`, move to `crate::components::form_tests` with `pub(crate)` visibility. T002 later moves them into `testing.rs`.

### Out of scope

- The README's stale template "Layout" section (it still describes `home.rs` as "copy this shape for new components" and lists no database modules). T401 rewrites it.
- The CI workflow changes themselves (T003).

## Acceptance criteria

- [x] `rust-toolchain.toml` and `rustfmt.toml` exist with the contents above. `cargo fmt --all --check` passes on stable.
- [x] The root `Cargo.toml` has `[workspace.lints.rust]` and `[workspace.lints.clippy]` as above, and both `crates/qry/Cargo.toml` and `crates/qry-core/Cargo.toml` contain `[lints] workspace = true`.
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes with the new lints. Every `#[allow]` added in non-test code carries a `reason = "…"`.
- [x] `grep -rn "unsafe" crates/*/src` finds nothing, or only one allow-listed module with a justification comment.
- [x] Every external dependency is declared once in `[workspace.dependencies]`. `grep -E '^[a-z_-]+ = "' crates/*/Cargo.toml` returns no versioned dependency lines, only `workspace = true` forms.
- [x] Exactly one `rust-version` exists, in `[workspace.package]`, and `cargo +<that version> check --workspace --all-features --locked` succeeds.
- [x] `deny.toml` exists and `cargo deny check advisories bans licenses sources --all-features` passes locally.
- [x] Adding `ratatui` as a dependency of `qry-core` makes `cargo deny check bans` fail. Verify this once by hand and describe it in the PR.
- [x] `.github/dependabot.yml` exists for `cargo` and `github-actions`, weekly and grouped.
- [x] `.gitignore` contains the entries listed above.
- [x] No file under `crates/` contains an inline `mod tests {`. Every former inline test module is now a sibling `*_tests.rs` or `tests.rs` file.
- [x] `cargo test --workspace --all-features --locked` runs the same number of tests as before the move (state both counts in the PR description), and all of them pass.
- [x] The formatting change is its own commit, and its hash is listed in `.git-blame-ignore-revs`.

## Tests

No new behaviour, so no new functional tests. These checks prove the baseline:

| Check | Command | Must |
|---|---|---|
| Format | `cargo fmt --all --check` | pass |
| Lints | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | pass |
| Lints, no default features | `cargo clippy --workspace --all-targets --no-default-features --locked -- -D warnings` | pass |
| Test count unchanged | `cargo test --workspace --all-features --locked 2>&1 \| grep -E '^test result'` | sum is equal before and after |
| Supply chain | `cargo deny check advisories bans licenses sources --all-features` | pass |
| MSRV | `cargo +$(MSRV) check --workspace --all-features --locked` | pass |

One structural test is added now and extended in T003:

- `crates/qry/tests/workspace_lints.rs`:
  - **`every_crate_inherits_workspace_lints`** uses `cargo_metadata` (dev-dependency) to load the workspace. For each workspace member it reads the manifest and asserts it contains `[lints]` with `workspace = true`.
  - **`every_crate_declares_the_workspace_rust_version`** asserts every package's `rust_version` is `Some` and that all of them are equal.
  - **`every_crate_is_mit_licensed`** asserts `license == Some("MIT")` for every member.
  - **`no_inline_test_modules`** walks `crates/*/src/**/*.rs` and fails if any file contains the line `mod tests {`, listing the offending files.

## Implementation notes

- **MSRV is 1.96, not 1.89.** The locked build dependencies `vergen-gix` and `vergen` 10.0.3 declare `rust-version = "1.96.0"` (every `vergen-gix` 10.x needs at least 1.95), so 1.89 cannot build. `cargo check --workspace --all-features --locked` passes with a standalone Rust 1.96.0 toolchain. This machine has no rustup, so it was run with that toolchain on `PATH` rather than as `cargo +1.96`.
- **Two formatting commits, both in `.git-blame-ignore-revs`:** `a4b18c3 style: cargo fmt the workspace` (stable rustfmt) and `84dd8c2 style: group and merge imports with nightly rustfmt`. Stable rustfmt ignores `imports_granularity` and `group_imports`, so a nightly rustfmt was run once to apply them. Stable and nightly `cargo fmt --all --check` now both pass.
- **cargo-deny flag order:** cargo-deny 0.20 rejects `cargo deny check … --all-features`. The working form is `cargo deny --all-features check advisories bans licenses sources`, which passes. Fixes needed to get there:
  - `webpki-roots` (via `oracledb`) gets a `CDLA-Permissive-2.0` license exception.
  - The yanked `yoke-derive` 0.8.3 was bumped to 0.8.4 in `Cargo.lock`.
  - `oracledb`, `rust_xlsxwriter` and the keyring backends needed nothing else.
  - The five `unused-wrapper` warnings are expected: `clap_complete`, `clap_mangen` and the other listed wrappers are not in the tree yet.
- **OpenSSL / native-tls are banned.** `cargo tree --target all --all-features -i` finds neither on any target: Postgres connects with `NoTls`, and `mysql_async` uses its default features. rustls is already in the tree through `oracledb`. `deny.toml` explains the reasoning.
- **Layering ban checked by hand.** With `ratatui = { workspace = true }` added to `qry-core`, `cargo deny check bans` fails with `error[banned]: crate 'ratatui = 0.30.2' is explicitly banned`. The change was then reverted.
- **`unsafe`.** `tui.rs` and `errors.rs` have no `unsafe`; `errors.rs` only uses the `libc::EXIT_FAILURE` constant. The only `unsafe` was `std::env::set_var` in two tests. It now lives in a single `#[cfg(test)]` helper, `secrets::set_test_var`, with `#[allow(unsafe_code, reason)]` and a justification comment. `unsafe_code` is `deny` rather than `forbid` so that this one allow works.
- **Sibling test files use `#[path]`.** For a non-root file such as `components/form.rs`, a plain `mod form_tests;` would resolve to `components/form/form_tests.rs`. Each module therefore declares `#[cfg(test)] #[path = "form_tests.rs"] mod form_tests;`, as sverb does. `lib.rs` uses a plain `mod lib_tests;`. The form helpers are `pub(crate)` in `form::form_tests`, and `components.rs` re-exports them as `crate::components::form_tests`.
- **Test count:** 173 before the move (146 qry + 27 qry-core), and 173 after. The 4 new structural tests in `crates/qry/tests/workspace_lints.rs` bring the total to 177.
- **Lint fixes beyond renaming:**
  - `unreachable_pub` turned about 150 `pub` items in the `qry` binary into `pub(crate)`.
  - `missing_debug_implementations` led to hand-written `Debug` impls that redact the password for the database configs, connections and `Driver`.
  - Non-test unwraps became handled errors: non-UTF-8 config/data directories, `Tui`'s `Drop`, and the event loop's init send. The two that cannot fail (`conntree` table rows, the single-char key parse) and the test-only `Config::embedded` carry `#[allow(…, reason)]`.
  - The file-wide `#![allow(dead_code)]` in `tui.rs` was narrowed to the three unused builder methods.
- **`.idea/`:** only the shared `dictionaries/` and `inspectionProfiles/` stay tracked. `modules.xml`, `qry.iml`, `vcs.xml` and `.idea/.gitignore` were untracked and are now ignored.
- The workspace has no cargo features, so the `--no-default-features` clippy run is the same as the default one. It passes.
- CI is not touched here (T003). CI being green on GitHub can't be checked locally.
