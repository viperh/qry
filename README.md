# qry

[![CI](https://github.com/viperh/qry/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/viperh/qry/actions/workflows/ci.yml)

A starting point for terminal user interfaces in Rust, built on
[ratatui](https://ratatui.rs) with an async [tokio](https://tokio.rs) event
loop, layered configuration, file logging and cross-platform release builds.

## Layout

The workspace deliberately splits the UI from everything else:

```
Cargo.toml              workspace manifest — all dependency versions live here
.config/config.json     default theme and key bindings, baked into the binary
.envrc                  direnv: keep config/data/logs inside the repo
crates/
  qry/                  the binary: terminal, rendering, input, config, logging
    build.rs            vergen — stamps git/build info into the version string
    src/
      main.rs           entry point
      app/              the pure reducer: State, UiEvent -> App::handle -> Vec<Effect>
        state.rs        everything the views draw, as plain data
        event.rs        UiEvent, and the worker's and files' answers
        effects.rs      Effect: what the runtime should do
        reducer/        how each event changes the state, one file per concern
        dialogs/        the modal forms
      views.rs, views/  free functions drawing the State with a Theme
      runtime.rs        the event loop: terminal, tasks, files, keychain, effects
      action.rs         Action: the one command vocabulary keys are bound to
      keymap.rs         key contexts, chord parsing and display, lookup
      db.rs             the database worker
      cli.rs            clap argument parsing
      config.rs         layered config: theme and key bindings
      config_migrate.rs reads the pre-context keybinding format
      errors.rs         panic hooks, color-eyre, human-panic
      logging.rs        tracing subscriber writing to a log file
      tui.rs            terminal setup/teardown and the crossterm event stream
  qry-core/             domain logic, no terminal dependencies
    src/lib.rs
```

`qry-core` must never depend on `ratatui`, `crossterm` or `clap`. Keeping the
domain there means it can be unit tested without a TTY, and it stays reusable
if you later add a second front end (a CLI, a daemon, a web UI).

## Running

```sh
cargo run -p qry
```

`Ctrl-q` quits and `F1` lists the keys; the defaults are in
`.config/config.json`.

```sh
cargo run -p qry -- --tick-rate 4 --frame-rate 60
cargo run -p qry -- --version    # prints git info and the resolved directories
cargo run -p qry -- --print-config # prints the config in effect
```

## Configuration

Defaults are compiled in from `.config/config.json`. At startup the app also
looks in the per-user config directory (printed by `--version`) for
`config.json5`, `config.json`, `config.yaml`, `config.toml` or `config.ini`,
and layers whatever it finds on top. Set `QRY_CONFIG` to override that
directory outright.

Key bindings are keyed by context (`Global`, `Tree`, `Editor`, `Results`,
`Dialog`, `Help`, …), then by key: `"<ctrl-a>"` for a single chord, `"<g><g>"`
for a sequence. Every value must name an `Action` variant, or be `null` to
unbind a default. A user file only needs the keys it changes. A config in the
old per-mode format is converted with a warning. `qry --print-config` prints
the bindings in effect.

## Logging

Logs go to `<data dir>/qry.log`. Set `QRY_LOG_LEVEL` (or `RUST_LOG`) to change
the filter, and `QRY_DATA` to change the directory.

## Checks

The local equivalents of the CI jobs in `.github/workflows/ci.yml`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
QRY_KEYRING=off cargo test --workspace --all-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --all-features --workspace --locked
cargo deny --all-features check advisories bans licenses sources
python3 scripts/check-layering.py
```

CI also checks the MSRV (`rust-version` in `Cargo.toml`), builds on Windows
and runs the database drivers against real servers.

`Cargo.lock` is committed on purpose — CI builds with `--locked`.

## Releases

Pushing a tag `vX.Y.Z` that matches `workspace.package.version` runs
`.github/workflows/cd.yml`. It builds static Linux binaries (musl, x86_64 and
aarch64), a universal macOS binary and a Windows binary, packs them into
reproducible archives with the man page and shell completions, and publishes
a GitHub release with `SHA256SUMS` and notes from [CHANGELOG.md](CHANGELOG.md).
It then updates the AUR, Homebrew and Scoop packages from `packaging/`. A
manual run of the workflow is a dry run. The checklist is
[docs/release.md](docs/release.md).

```sh
brew install viperh/tap/qry      # macOS, Linux
yay -S qry-bin                   # Arch (or `qry` to build from source)
scoop install qry                # Windows, after adding the bucket
nix run github:viperh/qry        # Nix
```

## License

MIT — see [LICENSE](LICENSE).
