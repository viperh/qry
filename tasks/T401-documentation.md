# T401 — Documentation and generated references

**Phase:** 4 — Docs · **Depends on:** T102, T204 (and the features it documents) · **Blocks:** release 1.0 (T007)

## Description

### Current state

- **README.md is still the ratatui template's text.** It describes qry as "a starting point for terminal user interfaces in Rust".
  - Its Layout section lists files that no longer match: only `home.rs` is described, and conntree, editor, results, the forms, `db.rs`, `connections.rs`, `import.rs` and `secrets.rs` are missing.
  - "Running" has a broken code span (`` `q`, `Ctrl-q quit ``).
  - It never says what qry does (an SQL client for PostgreSQL, MySQL, MariaDB, SQLite and Oracle), how passwords are stored, or how to import connections.
- The CI badge points at `workflows/CI` while the workflow triggers on `main`, but the branch is `master` (T003 fixes the workflow).
- There is no CONTRIBUTING.md, no CHANGELOG.md and no `docs/` directory. Commit `92aaf9a` ("rm docs") removed a previous one.
- Keybindings are documented only by the in-app help (`components/help.rs`, rebuilt by T204).

### Target design

Documentation is split into hand-written guides and **generated references that tests keep in sync**, following sverb's bless pattern (`crates/sverb-tui/tests/keybindings_doc.rs`, `SVERB_BLESS=1`).

#### Files

```
README.md                 what qry is, screenshot, features, install, quick start, links
CONTRIBUTING.md           dev setup, checks, test conventions, fuzz rule, commits, releases pointer
CHANGELOG.md              Keep a Changelog 1.1.0 + SemVer; Unreleased section
docs/
  keybindings.md          GENERATED from the default keymap (T102)
  config.md               GENERATED from the config schema; plus hand-written intro
  config.schema.json      GENERATED JSON Schema of the config file
  connections.md          connection file format, password storage, env vars, import sources
  theme.md                palette table (from T201), NO_COLOR / 256-colour behaviour
  architecture.md         crates, layering rule, reducer/effects (T101), sessions (T103), jobs (T305)
  testing.md              how to run unit/snapshot/DB/fuzz/bench suites locally
  release.md              release checklist (T007)
  assets/                 screenshots (PNG) and an asciinema cast / VHS tape source
```

#### README.md

1. **Title and badges:** CI, release, license, MSRV.
2. **What qry is, in one sentence:** *a fast, keyboard-driven terminal SQL client for PostgreSQL, MySQL, MariaDB, SQLite and Oracle*.
3. **Screenshot:** `docs/assets/qry-main.png`, plus a GIF from a VHS tape, `docs/assets/demo.tape`, kept in the repo so it can be regenerated.
4. **Features:**
   - schema browser
   - highlighted editor with statement-under-cursor execution
   - multi-result grid with sort, search and copy
   - inspector
   - jobs with cancel
   - history and snippets
   - exports (8 formats)
   - multiple sessions
   - keychain passwords
   - import from DBeaver and SQL Developer
   - NO_COLOR support
5. **Install, per channel from T007:**
   - prebuilt binaries
   - `cargo install qry --locked`
   - AUR `qry` / `qry-bin`
   - Homebrew tap
   - Scoop bucket
   - Nix flake (`nix run github:viperh/qry`)
6. **Quick start:**
   - Running `qry` opens a scratch in-memory SQLite.
   - `ctrl-n` adds a connection.
   - `ctrl-enter` runs the statement under the cursor.
   - `?` opens help.
7. **Configuration summary:** where the file lives (`qry --version` prints the dirs), `QRY_CONFIG` / `QRY_DATA` / `QRY_LOG_LEVEL`, `theme`, `max_rows`, keybinding remaps. Links to `docs/config.md` and `docs/keybindings.md`.
8. **Security notes:**
   - Passwords live only in the OS keychain, an environment variable, or a prompt. They are never written to qry's files and never imported.
   - History redaction (T306).
9. **Links:** CONTRIBUTING, CHANGELOG, license.

Every command shown in the README is exercised by a test (below).

#### Generated references

- **`docs/keybindings.md`**
  - It is produced by `qry_help::render_markdown(&Keymap::embedded())`.
  - It uses the **same help model** as the T204 overlay (groups, ordering, merged chords, descriptions), so the docs and the in-app help cannot disagree.
  - Each context is a `##` section with a `| Keys | Action |` table, and keys are rendered with T102 `chord_to_string`.
- **`docs/config.schema.json` and `docs/config.md`**
  - The schema is derived from the config structs with `schemars`, behind a `schema` dev feature or a build-time test.
  - `config.md` lists every key with type, default and description, taken from the doc comments, between `<!-- BEGIN GENERATED -->` / `<!-- END GENERATED -->` markers. Text outside the markers is hand-written and kept.
- **Man page**
  - `qry generate man` (a hidden clap subcommand, built with `clap_mangen`) writes `qry.1`.
  - `qry generate completions <shell>` (`clap_complete`) covers bash, zsh, fish and powershell.
  - Both are produced in CD (T007 `assets` job) and snapshot-tested.
- **Bless workflow**
  - Running the tests with `QRY_BLESS=1` rewrites every generated file in place.
  - Without it, a mismatch fails with a diff and the hint `run QRY_BLESS=1 cargo test -p qry --test docs`.

#### CONTRIBUTING.md

**Dev setup:**

- `rust-toolchain.toml` (T001); direnv `.envrc` keeps config, data and logs in the repo.
- Optional `nix develop`.
- `docker compose -f tests/docker-compose.yml up -d` for the DB suites (T004).

**The checks**, the same list CI runs (T003), copy-pasteable:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked
cargo deny check
```

The DB suites are opt-in with `QRY_TEST_POSTGRES_URL=… QRY_TEST_MYSQL_URL=… QRY_TEST_MARIADB_URL=… QRY_TEST_ORACLE_URL=… cargo test -p qry-core --tests`.

**Test conventions** (from T002):

- Sibling `*_tests.rs` files.
- Snapshot names carry the size (`help_120x40`). Review with `cargo insta review`; never commit `*.snap.new`.
- Use `AppHarness` for flows.
- No sleeps: use the virtual clock.
- Database tests use `db_or_skip!`.

**Rules for changes:**

- No raw colours outside `theme.rs` (the `no_raw_colors` test).
- `qry-core` must not depend on ratatui, crossterm or clap (cargo-deny wrappers, layering test).
- Every new parser of untrusted input gets a fuzz target, a seed in `fuzz/seed-corpus.sh`, a proptest twin, and a line in the `fuzz.yml` matrix (T005).
- Performance-sensitive code has a bench with a gate (T006).
- Conventional commits:
  - `feat:` / `fix:` / `perf:` / `refactor:` / `docs:` go in the changelog.
  - `test:` / `ci:` / `build:` / `chore:` do not.
- Add a CHANGELOG `Unreleased` entry in the same PR for user-visible changes.
- Regenerate generated docs with `QRY_BLESS=1`.

#### CHANGELOG.md

Keep a Changelog 1.1.0 format, SemVer.

- **Sections:** an `## [Unreleased]` section, then `## [X.Y.Z] - YYYY-MM-DD` with `### Added / Changed / Fixed / Removed / Security`.
- **Seed** with `## [0.1.0]`, describing what exists today:
  - the five backends
  - keychain passwords
  - import from qry, SQL Developer and DBeaver
  - connection export
  - the export formats
- The redesign lands under `Unreleased` until the release that ships it.
- T007's release-notes `awk` extraction reads this file. A test checks that the format parses: every version heading matches the regex and the dates are ISO.

#### docs/release.md

T007 owns the process; this task writes it down. The steps:

1. Bump `workspace.package.version`.
2. Move `Unreleased` to the version.
3. Run `cargo test`, which updates `Cargo.lock` and version-dependent snapshots.
4. Dry-run CD with `workflow_dispatch`.
5. Push the `vX.Y.Z` tag.
6. Verify the channels (AUR, Homebrew, Scoop, Nix).

### Edge cases

- Generated docs must be deterministic: sorted keys, no timestamps, no absolute paths, and the same output on every OS. CI runs the docs test on Linux only, but the output must not depend on the platform default keymap, so it uses the embedded defaults.
- A user's own config must not affect the generated docs. The test uses `Config::embedded()` and ignores `QRY_CONFIG`.
- Screenshots go stale. They are regenerated by `vhs docs/assets/demo.tape` manually, and are not CI-gated, but the README test asserts the referenced files exist.

### Out of scope

A documentation website (mdBook), translations, API docs beyond rustdoc.

## Acceptance criteria

- [ ] README.md describes qry accurately. It has no template leftovers ("starting point", the stale Layout section, the broken code span) and covers features, install per channel, quick start, configuration and security notes, with a screenshot.
- [ ] CONTRIBUTING.md lists the exact CI checks, the test conventions, the fuzz and bench rules, the colour and layering rules, and the commit convention.
- [ ] CHANGELOG.md follows Keep a Changelog 1.1.0, has `Unreleased` and `0.1.0`, and parses with T007's notes extraction.
- [ ] `docs/keybindings.md` is generated from the embedded keymap through the same help model as the T204 overlay. A test fails on drift. `QRY_BLESS=1` regenerates it.
- [ ] `docs/config.md` (between markers) and `docs/config.schema.json` are generated from the config types, and a test fails on drift.
- [ ] `qry generate man` and `qry generate completions <shell>` exist (hidden), and their output is snapshot-tested.
- [ ] `docs/connections.md`, `docs/theme.md`, `docs/architecture.md`, `docs/testing.md` and `docs/release.md` exist and match the implemented behaviour.
- [ ] All relative links in README, CONTRIBUTING and `docs/*.md` resolve (link-check test).
- [ ] The CI badge URL in the README matches the workflow name and default branch.

## Tests

**Integration tests** — `crates/qry/tests/docs.rs`:

- `keybindings_doc_matches_embedded_keymap`: bless with `QRY_BLESS=1`.
- `keybindings_doc_lists_every_bound_action`: every `(context, action)` with a default binding appears.
- `config_doc_generated_section_matches_schema`: bless; text outside the markers is preserved.
- `config_schema_json_matches_types`: bless.
- `generated_docs_ignore_user_config`: sets `QRY_CONFIG` to a dir with remaps and asserts the output is unchanged.
- `relative_links_resolve`: parse `[..](path)` links in README.md, CONTRIBUTING.md and `docs/**/*.md`, and assert each target exists. Anchors are checked against the headings.
- `readme_referenced_assets_exist`
- `readme_ci_badge_matches_workflow_and_branch`: reads `.github/workflows/ci.yml` `name:` and the `on.push.branches`.

**Integration tests** — `crates/qry/tests/changelog.rs`:

- `changelog_has_unreleased_and_semver_headings`
- `changelog_dates_are_iso`
- `release_notes_extraction_finds_section`: runs the T007 awk snippet (or an equivalent Rust function shared with the release script) for `0.1.0`.

**Snapshots** — insta, in `crates/qry/src/cli_tests.rs`:

- `cli_help` (`qry --help`)
- `cli_version_layout`: version redacted with `insta` filters, dirs redacted.
- `man_page`: `qry generate man`, with the version filtered.
- `completions@bash`, `completions@zsh`, `completions@fish`, `completions@powershell`: one snapshot each, via `snapshot_suffix`, as sverb does for help.

**Unit test** — `crates/qry/src/help_model_tests.rs` (shared with T204):

- `markdown_and_overlay_use_same_groups_and_order`
- `markdown_escapes_pipes_in_keys` (the `|` key)

**CI:** the docs tests run in the normal `test` job (T003). The `docs` rustdoc job stays `-D warnings`.
