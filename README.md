# qry

[![CI](https://github.com/viperh/qry/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/viperh/qry/actions/workflows/ci.yml)

A starting point for terminal user interfaces in Rust, built on
[ratatui](https://ratatui.rs) with an async [tokio](https://tokio.rs) event
loop, layered configuration, file logging and cross-platform release builds.

## Layout

The workspace deliberately splits the UI from everything else:

```
Cargo.toml              workspace manifest — all dependency versions live here
.config/config.json     default keybindings and styles, baked into the binary
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
      action.rs         the commands keys are bound to
      db.rs             the database worker
      cli.rs            clap argument parsing
      config.rs         layered config, keybinding and style parsing
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

`q`, `Ctrl-q quit
`.config/config.json`.

```sh
cargo run -p qry -- --tick-rate 4 --frame-rate 60
cargo run -p qry -- --version    # prints git info and the resolved directories
```

## Configuration

Defaults are compiled in from `.config/config.json`. At startup the app also
looks in the per-user config directory (printed by `--version`) for
`config.json5`, `config.json`, `config.yaml`, `config.toml` or `config.ini`,
and layers whatever it finds on top. Set `QRY_CONFIG` to override that
directory outright.

Keybindings are keyed by mode, then by key sequence: `"<Ctrl-a>"` for a single
chord, `"<g><g>"` for a sequence. Every value must name an `Action` variant.

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

Pushing a tag matching `v1.2.3` or `1.2.3` builds the binary for macOS
(x86_64/arm64), Linux (x86_64/arm64/i686) and Windows, then attaches tarballs
and SHA-256 sums to the GitHub release.

## License

MIT — see [LICENSE](LICENSE).
