# T007 — Release pipeline (CD) and packaging

**Phase:** 0 — Foundation · **Depends on:** T003 · **Blocks:** the first tagged release of the redesign

## Description

`.github/workflows/cd.yml` is the ratatui-template release.

- **Trigger:** tags matching `'[v]?[0-9]+.[0-9]+.[0-9]+'`. That is a glob, not a regex, so the `?` does not mean "optional v" the way the author intended.
- **Build matrix:** six targets: macOS x86_64 and arm64, Linux gnu x86_64, aarch64 and i686 (via `cross`), and Windows msvc.
- **Packaging:** each target is tarred with `tar czvf`, which embeds mtimes and the owner, so it is not reproducible. `certutil` and `shasum` write a `.sha256` per archive, and each matrix leg uploads straight to a GitHub release with `softprops/action-gh-release@v2`.

That leaves several gaps:

- No check that the tag matches `workspace.package.version` (`0.1.0` in `Cargo.toml`).
- No dry run.
- No changelog, so the release has no notes.
- No man page and no shell completions.
- No combined checksum file.
- No package-manager channels.
- The glibc Linux binaries are tied to the runner's glibc.

This task replaces it with a release modelled on `~/Projects/sverb/.github/workflows/cd.yml`, `scripts/release-package.sh`, `scripts/update-packaging.py`, `packaging/`, `flake.nix`, `CHANGELOG.md` and `docs/release.md`.

### 1. Assets from the binary: `qry generate`

Add a hidden subcommand to `crates/qry/src/cli.rs`:

```
qry generate man                     # writes qry.1 to stdout (clap_mangen)
qry generate completions <shell>     # bash | zsh | fish | powershell | elvish (clap_complete)
```

- `clap_mangen` and `clap_complete` go in `[workspace.dependencies]`. T001's `deny.toml` `wrappers` already allow them to depend on `clap`.
- The man page includes a short `ENVIRONMENT` section: `QRY_CONFIG`, `QRY_DATA`, `QRY_LOG_LEVEL`, `QRY_KEYRING`, `NO_COLOR`, `COLORTERM`.
- The subcommand runs before the TUI starts, and never enters raw mode.

### 2. `cd.yml`

```yaml
on:
  push:
    tags: ['v[0-9]+.[0-9]+.[0-9]+*']
  workflow_dispatch:      # always a dry run: builds everything, publishes nothing
permissions: { contents: read }
```

Jobs:

1. **`meta`:**
   - Reads `workspace.package.version` with `python3 -c 'import tomllib; …'`.
   - On a tag, fails with `::error::` unless `${GITHUB_REF_NAME#v}` equals it, and sets `publish=true`. On dispatch, sets `publish=false`.
   - Outputs `version` and `source-date-epoch=$(git log -1 --format=%ct)`.
2. **`assets`:**
   - Builds the debug binary and runs `qry generate man > qry.1` and `qry generate completions {bash,zsh,fish,powershell}`.
   - Validates with `bash -n qry.bash`, `zsh -n _qry` (if zsh is installed) and `groff -man -ww -z qry.1`.
   - Uploads the `qry-assets` artifact.
3. **`linux`:**
   - Matrix `x86_64-unknown-linux-musl` (with `musl-tools`) and `aarch64-unknown-linux-musl` (with `cross`).
   - Build env:
     ```
     RUSTFLAGS="-C target-feature=+crt-static -C strip=symbols \
       --remap-path-prefix=$GITHUB_WORKSPACE=/build/qry --remap-path-prefix=$HOME/.cargo=/cargo"
     SOURCE_DATE_EPOCH=${{ needs.meta.outputs.source-date-epoch }}
     ```
   - Build command: `cargo build --locked --release -p qry --target …`.
   - **Static check:** `file "$bin" | grep -Eq 'statically linked|static-pie linked'`.
   - **Reproducibility check (x86_64):** `git worktree add --detach "$other" HEAD`, build there with its own `CARGO_TARGET_DIR`, then `cmp` the two binaries.
   - Package with `scripts/release-package.sh`.
4. **`macos`:**
   - Runs on `macos-14`. Builds `x86_64-apple-darwin` and `aarch64-apple-darwin`, combines them with `lipo -create`, and asserts that `lipo -info` lists both architectures.
   - **Optional signing:** with `APPLE_CERTIFICATE_P12`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_NOTARY_KEY`, `APPLE_NOTARY_KEY_ID` and `APPLE_NOTARY_ISSUER` set, it builds a temporary keychain, runs `codesign --force --options runtime --timestamp`, then `xcrun notarytool submit --wait`.
   - Without the certificate it prints `::notice::unsigned macOS build` and continues.
5. **`windows`:**
   - Runs on `windows-latest` with `shell: bash` and `git config --global core.autocrlf false`.
   - Builds `x86_64-pc-windows-msvc`. `crates/qry/build.rs` embeds `assets/icon.ico` through `winresource`; check that the icon shows in the built `.exe`.
   - Packages a `zip -X`.
6. **`release`** (`contents: write`):
   - Downloads all artifacts.
   - Concatenates the per-archive `.sha256` files into `SHA256SUMS` (sorted by file name) and checks them with `sha256sum -c SHA256SUMS`.
   - Extracts the notes:
     ```bash
     awk -v v="$VERSION" '
       /^## \[/ { if (on) exit; on = index($0, "[" v "]") > 0; next }
       on' CHANGELOG.md > notes.md
     [[ -s notes.md ]] || { echo "::error::CHANGELOG.md has no section for $VERSION"; exit 1; }
     ```
     A tag with no changelog section **fails** the release. A dry run only warns.
   - Optionally builds a CycloneDX SBOM with `cargo-cyclonedx`, with `continue-on-error: true`.
   - With `publish`, runs `softprops/action-gh-release@v2` with `body_path: notes.md` and `files: dist/*`. Otherwise uploads the `release-dry-run` artifact.
7. **`channels`** (only with `publish`):
   - Runs `gh release download "$TAG" -p SHA256SUMS`, then computes the source tarball's sha256.
   - Runs `python3 scripts/update-packaging.py "$VERSION" SHA256SUMS --source-sha256 … --out out`.
   - Publishes each channel only if its secret is non-empty, otherwise prints `::notice::skipping <channel>`:
     - **AUR:** `KSXGitHub/github-actions-deploy-aur@v3`, for `qry` (from source) and `qry-bin`. Secret: `AUR_SSH_PRIVATE_KEY`.
     - **Homebrew tap:** `git clone`, commit, push. Secret: `HOMEBREW_TAP_TOKEN`.
     - **Scoop bucket:** the same. Secret: `SCOOP_BUCKET_TOKEN`.

The `i686-unknown-linux-gnu` target is dropped. Say so in the CHANGELOG under "Removed". Re-adding it later as `i686-unknown-linux-musl` is fine if anyone asks.

### 3. The keychain on musl: a decision is required

`crates/qry/Cargo.toml` depends on `keyring = "4.2.0"`, which `crates/qry/src/secrets.rs` uses to store passwords. On Linux the keyring backends talk to the Secret Service over D-Bus, or to the kernel keyutils.

- Some backend configurations link `libdbus` dynamically, which **breaks a `+crt-static` musl build** or makes it fail at run time on systems without that library.
- The others work statically: a pure-Rust D-Bus implementation (`zbus`), or keyutils.

Before the musl targets ship:

- [ ] Identify which backend features `keyring` 4.2 enables by default, and whether they link `libdbus`. Use `cargo tree -e features -p qry --target x86_64-unknown-linux-musl`, plus `ldd` and `file` on a test build.
- [x] Choose one of these and record the decision in `docs/release.md`:
  - (a) Enable a pure-Rust Secret Service backend, so the static build keeps keychain support.
  - (b) Use keyutils on musl only. Passwords then survive only until reboot or logout, which the UI must say.
  - (c) Ship glibc builds (`x86_64-unknown-linux-gnu` built on the oldest supported Ubuntu runner) instead of musl.
- [ ] Prove the choice works: on a desktop Linux machine (the maintainer runs Omarchy/Hyprland), the release binary saves a password with "remember", restarts, and connects without asking. On a headless box with no Secret Service, saving fails gracefully with the existing `SecretNotStored` path.

### 4. Reproducible archives: `scripts/release-package.sh`

Usage: `scripts/release-package.sh <version> <label> <binary> <out-dir> [assets-dir]`.

```bash
"$tar" --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$SOURCE_DATE_EPOCH" \
       --format=gnu -cf - "$stem" | gzip -n -9 > "$out/$stem.tar.gz"
```

- The archive stem is `qry-<version>-<label>`, with labels `linux-x86_64`, `linux-aarch64`, `macos-universal` and `windows-x86_64`.
- Each archive contains the binary, `LICENSE`, `README.md`, `CHANGELOG.md`, `man/qry.1` and `completions/*`.
- Windows uses `zip -X`.
- A `.sha256` file is written next to each archive.
- The script uses GNU tar (`gtar` on macOS).

### 5. `packaging/`

```
packaging/
  aur/qry/PKGBUILD        aur/qry/.SRCINFO          # builds from the source tarball
  aur/qry-bin/PKGBUILD    aur/qry-bin/.SRCINFO      # prebuilt musl; installs man page + completions
  homebrew/qry.rb                                     # on_macos universal; on_linux on_intel/on_arm
  scoop/qry.json                                      # checkver.github, autoupdate, hash.url = "$url.sha256"
```

- The committed files carry the placeholder version `0.0.0` and zeroed hashes. Only CI rewrites them.
- `scripts/update-packaging.py <version> <SHA256SUMS> --source-sha256 <hash> --out <dir>` rewrites the version, URLs and checksums in all four, and regenerates `.SRCINFO`. It is stdlib only.
- `scripts/update-packaging.py --self-test` runs against a synthetic `SHA256SUMS` and asserts every placeholder was replaced.

A `packaging` job in `ci.yml` (added to `ci-ok`'s `needs`):

- runs `update-packaging.py --self-test`
- runs `bash -n` on both PKGBUILDs, `ruby -c` on the formula and `jq -e .` on the Scoop manifest
- builds the Linux archive twice and `cmp`s the two, then runs `sha256sum -c`
- runs `cargo package -p qry-core --locked --no-verify` as a publishability check

### 6. Changelog and release process

**`CHANGELOG.md`.** Keep a Changelog 1.1.0 plus SemVer, written by hand.

- The top section is `## [Unreleased]`.
- Seed it with the existing history from `git log`: Oracle support, connection import and export, connection editing, and so on, under `## [0.1.0]`.
- The redesign lands under `[Unreleased]` until it is tagged.

**`docs/release.md`.** The checklist:

1. Conventional commits are used throughout. `feat`, `fix`, `perf`, `sec`, `refactor` and `docs` make the changelog; `build`, `ci`, `chore` and `test` stay out.
2. Bump `workspace.package.version` and the `qry-core` path dependency's `version`.
3. Move `[Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`.
4. Run `cargo test --workspace` so `Cargo.lock` and any version-bearing snapshots update (the help title shows the version, as in T204).
5. Dry-run `cd.yml` with `workflow_dispatch`, and download and smoke-test the `release-dry-run` artifact on Linux, macOS and Windows.
6. Push the tag `vX.Y.Z`.
7. Verify each channel: `yay -S qry-bin`, `brew install viperh/tap/qry`, `scoop install qry`.

### 7. `flake.nix`

- Inputs: nixpkgs-unstable and flake-utils.
- Reads the version from `Cargo.toml` with `builtins.fromTOML`, and uses `cargoLock.lockFile = ./Cargo.lock`.
- `doCheck = false`, with a comment that CI runs the tests (several need servers).
- `postInstall` runs `qry generate man` and `generate completions` through `installShellFiles`.
- Provides `apps.default`.
- `devShells.default` contains cargo, rustc, clippy, rustfmt, rust-analyzer, python3, cargo-deny, cargo-insta and cargo-fuzz, with `QRY_KEYRING=off`.
- CI job `nix` (`cachix/install-nix-action@v31`):
  ```
  nix flake check --no-build
  nix build .#qry -L
  ./result/bin/qry --version
  test -f result/share/man/man1/qry.1.gz
  ```

### Out of scope

- crates.io publishing (manual, if ever).
- Container images: qry is a desktop TUI, and sverb's `image` job does not apply.

## Acceptance criteria

- [x] `qry generate man` prints a valid man page (`groff -man -ww -z` reports no warnings), and `qry generate completions bash|zsh|fish|powershell` print completion scripts. `bash -n` passes on the bash one.
- [ ] `cd.yml` triggers on `v[0-9]+.[0-9]+.[0-9]+*` tags and on `workflow_dispatch`. A dispatch run builds every target and uploads `release-dry-run` without creating a release (link the run).
- [ ] A tag that does not match `workspace.package.version` fails the `meta` job with a clear error (verify on a fork or a throwaway tag).
- [ ] A tag with no `CHANGELOG.md` section fails the `release` job.
- [ ] The Linux binaries are static (the `file` check is in the workflow), and the x86_64 binary is byte-identical across the two builds (the `cmp` step is in the workflow and passes).
- [x] Building the same archive twice with `scripts/release-package.sh` gives byte-identical `.tar.gz` files.
- [ ] The macOS archive contains one universal binary with both `x86_64` and `arm64`. Signing runs only when its secrets are present.
- [ ] The Windows zip runs `qry.exe --version` on a Windows runner and shows the embedded icon.
- [ ] Every release has `SHA256SUMS`, and the workflow verifies it before publishing.
- [ ] The keyring decision (section 3) is made and documented in `docs/release.md`, and the stated manual verification has been done on Linux desktop, headless Linux, macOS and Windows.
- [ ] `packaging/` holds the four manifests with placeholders. `scripts/update-packaging.py --self-test` passes, and the `packaging` CI job is green.
- [ ] The `channels` job skips each channel cleanly when its secret is empty, and the logs show a `::notice::`.
- [x] `CHANGELOG.md` exists in Keep a Changelog format, with `[Unreleased]` and the backfilled `[0.1.0]`.
- [x] `docs/release.md` holds the checklist above.
- [ ] `flake.nix` exists, the `nix` CI job is green, and `nix develop` gives a shell where `cargo test --workspace` passes.
- [x] The old six-target `cd.yml` matrix and its `use-cross` gnu entries are gone.

## Tests

Rust tests in `crates/qry/src/cli_tests.rs`:

- **`help_output_snapshot`**: `insta::assert_snapshot!("cli_help", Cli::command().render_long_help())`.
- **`generate_man_contains_the_environment_section`**: the output contains `QRY_CONFIG`, `QRY_KEYRING` and `NO_COLOR`.
- **`generate_completions_for_every_shell`**: for each shell, the output is non-empty and contains `qry`.
- **`generate_never_touches_the_terminal`**: runs `env!("CARGO_BIN_EXE_qry") generate man` with stdin and stdout not a TTY. Exits 0, and stdout contains no escape sequences such as `\x1b[?1049h`.

Script self-tests (run in the `packaging` CI job):

- **`update-packaging.py --self-test`**: every `0.0.0` and zero hash is replaced, and `.SRCINFO` matches the PKGBUILD.
- **`release-package.sh` determinism**: two runs, `cmp` equal.
- **`release-package.sh` contents**: `tar -tzf` lists `qry-<v>-<label>/qry`, `LICENSE`, `README.md`, `CHANGELOG.md`, `man/qry.1` and the `completions/` files.

Workflow verification, done once and recorded in the PR:

| Scenario | Expected |
|---|---|
| `workflow_dispatch` on `master` | all build jobs green; `release-dry-run` artifact; no release created; `channels` skipped |
| Tag `v9.9.9` with version `0.1.0` | `meta` fails with the mismatch message |
| Matching tag, no changelog section | `release` fails |
| Matching tag with a changelog section (on a fork) | release created with notes from the changelog, `SHA256SUMS` and all archives |

## Implementation notes

- **Unticked boxes need GitHub, a tag or other machines.** Nothing was pushed or tagged, so none of the runs in the table above exist, and no box that asks for a run or a green job is ticked: the dispatch run, the `v9.9.9` mismatch, the missing-changelog failure, the static and `cmp` checks, the macOS universal binary and signing, the Windows zip and icon, `SHA256SUMS` on a real release, the `packaging` and `nix` jobs going green, and the `channels` notices. Local stand-ins:
  - **`meta` and the release notes.** A scratch script loaded `cd.yml` with PyYAML and ran the two steps' `run:` text under bash with simulated events. Dispatch gives `publish=false` and a `::notice::`. Tag `v0.1.0` gives `publish=true`. Tag `v9.9.9` exits 1 with `::error title=Tag does not match the version::tag v9.9.9 is version 9.9.9, but workspace.package.version in Cargo.toml is 0.1.0 …`. Release notes for `0.1.0` come out without the link definitions. `9.9.9` with `PUBLISH=true` exits 1 with `::error::CHANGELOG.md has no section for 9.9.9`, and with `PUBLISH=false` it only warns.
  - **Lint.** `actionlint` 1.7.7 with `shellcheck` reports nothing for `cd.yml` and `ci.yml`. PyYAML loads both. `bash -n` passes on both PKGBUILDs and on `release-package.sh`, `ruby -c` on the formula, and `jq -e .` on the Scoop manifest. `shellcheck` is clean on `release-package.sh`.
  - **Assets.** `qry generate man` passes `groff -man -ww -z` with no output. `bash -n` passes on the bash completions and `zsh -n` on the zsh ones (groff 1.23 and zsh from apt).
  - **Archives.** `release-package.sh` was run twice on the same binary, the second time under `umask 077` and generating its own assets, for `linux-x86_64` (`tar.gz`) and `windows-x86_64` (`zip`). Both pairs are byte-identical (`cmp`), `sha256sum -c` passes, and the listing is the one the `packaging` job diffs against. A bad version (`1.2`) and an unknown label (`linux-i686`) exit 2.
  - **Flake.** `nix flake check --no-build` passes (`all checks passed!`). The session's proxy refuses GitHub tarball and API downloads, so the inputs were overridden with `git+https://…?shallow=1` and no lock was written. `nix build` was started and then given up when the shared disk filled. **There is no `flake.lock`.** Run `nix flake lock` once on a normal machine and commit the result. Until then the `nix` job resolves the inputs fresh on each run.
  - **musl.** A local `x86_64-unknown-linux-musl` release build (musl-tools from apt) was stopped half-way because the shared disk filled. So `file` and `ldd` were never run on a musl binary here. The evidence for section 3 is the dependency tree only, see below.
  - `cargo package -p qry-core --locked --no-verify` packages 31 files.
- **Section 3, the keychain decision: (a).** `cargo tree -e features -p qry --target x86_64-unknown-linux-musl` shows that `keyring` 4.2's defaults (`v1`) enable `zbus-secret-service-keyring-store`, then `secret-service` 5.2 with `crypto-rust`, on `zbus` 5. That is pure Rust: the tree has no `libdbus-sys`, `openssl-sys` or keyutils. So the static build keeps keychain support with no change, and the decision and the reasons are in `docs/release.md`. The first box stays unticked because it also asks for `ldd`/`file` on a test build, which never finished here (see musl above). The third box (desktop, headless, macOS and Windows checks) is a manual job for the maintainer. `docs/release.md` has the table.
- **`qry generate`** is a hidden subcommand (`Command::Generate { asset: Asset }`, `Asset::Man` and `Asset::Completions { shell: clap_complete::Shell }`). `main.rs` handles it before `errors::init` and `logging::init`, so it creates no log file and touches no terminal. The test checks that the data directory is never created.
  - The assets are built from `asset_command()`. That is a copy of the user-facing flags, the about text and the author, without the hidden subcommand, so neither the man page synopsis nor the completions mention `generate` or `[subcommands]`. Its version is the plain `CARGO_PKG_VERSION`: `--version`'s text has the build machine's config and data directories in it. clap_mangen has no environment section, so `ENVIRONMENT` is hand-written roff placed between OPTIONS and VERSION.
  - **New dependencies:** `clap_complete` 4.6.11 and `clap_mangen` 0.3.3 (plus `roff`), all `MIT OR Apache-2.0`, in `[workspace.dependencies]`. `deny.toml` already listed them as `clap` wrappers. `cargo-deny` is not installed here, so `deny` was not run.
  - **Tests.** `cli_tests.rs` has `help_output_snapshot`, `generate_man_contains_the_environment_section`, `generate_completions_for_every_shell` (all five shells, none mention `generate`), plus `generate_parses_man_and_completions`, `generate_is_hidden_from_the_help` and `generate_man_is_the_same_on_every_machine`. Two deviations:
    - The snapshot renders at `term_width(100)`. With `wrap_help`, clap otherwise wraps to whatever terminal runs the test. The snapshot lists `--theme`'s possible values, so it changes whenever a theme is added.
    - `generate_never_touches_the_terminal` is in `crates/qry/tests/generate.rs`, not `cli_tests.rs`. `CARGO_BIN_EXE_qry` is only set for integration tests. That file also runs the completions through the binary and checks that an unknown shell exits 2.
- **`cd.yml` deviations and details:**
  - The secrets used in `if:` conditions are first copied into env (`HAVE_CERTIFICATE`, and `AUR_SSH_PRIVATE_KEY` and the others in `channels`). GitHub does not allow `secrets` in a step's `if`. Every skipped channel logs `::notice::skipping <channel> (… is not set)`.
  - The reproducibility build gives the second worktree its own `--remap-path-prefix=$other=/build/qry`. With the same flags as the first build it would embed a different path. Its `CARGO_TARGET_DIR` is inside that worktree, so both builds remap their `OUT_DIR` paths the same way.
  - Release notes: after the spec's `awk`, a `sed` drops the Keep a Changelog link definitions. Without it, the last section (`[0.1.0]`) runs to the end of the file and carries them along.
  - A dry run with no changelog section writes a one-line placeholder `notes.md`, so the artifact is complete.
  - The Windows icon check loads `qry.exe` as a data file (`LoadLibraryEx`) and counts its `RT_GROUP_ICON` and `RT_VERSION` resources, in PowerShell. `zip` comes from `choco`, because Git Bash has none. The zip is also unpacked and `qry.exe --version` is run from it.
  - The SBOM (`cargo-cyclonedx`) files go into `dist/` with their own `.sha256` before `SHA256SUMS` is built, so they are covered too. Both SBOM steps are `continue-on-error`.
  - The Homebrew tap is `viperh/homebrew-tap` and the Scoop bucket `viperh/scoop-bucket`, set as workflow env. Those repositories do not exist yet.
- **`release-package.sh`** sets `LC_ALL=C TZ=UTC` and `umask 022` and uses `install -m` for fixed modes. `SOURCE_DATE_EPOCH` defaults to the last commit's time. It uses `gtar` when present and refuses a non-GNU tar. For zip it sets every mtime with `touch -d @…`, then zips a sorted file list next to the directory and moves the result, so a native Windows `zip.exe` never gets an MSYS path. Without `[assets-dir]` it generates the man page and completions with the binary itself.
- **`update-packaging.py`** is stdlib only.
  - It fills each manifest by meaning, not by position: PKGBUILD `sha256sums*` arrays by name, each formula `sha256` from the `url` right above it, and Scoop through `json`.
  - It regenerates `.SRCINFO` with a small PKGBUILD parser: assignments and arrays with `$var` expansion, function bodies skipped.
  - Extra modes: `--check-srcinfo`, and `--write-srcinfo` to run after editing a PKGBUILD.
  - `--self-test` checks the committed `.SRCINFO` files and does a full rewrite against a synthetic `SHA256SUMS`, with a different hash per archive, so a swapped hash is caught. It then checks that no `0.0.0` or zero hash is left, that each hash sits where it belongs, that `.SRCINFO` matches its PKGBUILD, and that Scoop's `autoupdate` keeps `$version`. Finally it checks that a `SHA256SUMS` missing an archive is rejected (exit 2).
- **`ci.yml`** gains `packaging` and `nix`, both in `ci-ok`'s `needs`. `packaging` packs a debug build: how the binary was built does not affect whether the archive is deterministic, and it saves a full LTO build.
- **Other changes outside new files:**
  - `crates/qry/Cargo.toml`'s description is now "A terminal SQL client for PostgreSQL, MySQL, MariaDB, SQLite and Oracle" instead of the template text. It is clap's `about`, so it shows in `--help`, the man page and the snapshot.
  - `build.rs`'s Windows `ProductName`/`FileDescription` say `qry` instead of `MyTool`.
  - The README's Releases section describes the new pipeline and the install commands.
- **The CHANGELOG** was not seeded from `git log` alone. The history begins at the redesign (59 commits since 2026-10-13), so `[0.1.0]` lists what the code and fixtures show existed before it, and `[Unreleased]` covers the redesign commits. "Removed" records the `i686-unknown-linux-gnu` and glibc builds.
