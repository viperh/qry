# Changelog

All notable changes to qry are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The release process is in [docs/release.md](docs/release.md).

## [Unreleased]

The interface redesign. Everything below lands in the first release after
0.1.0.

### Added

- Several connections open at once, each a session with its own database
  worker, editor and results, shown as tabs with indicators in the tree.
- A borderless screen with a top bar and mode pill, a status line and a
  key-hint bar.
- A theme system (dark palette with amber, teal, purple and coral accents)
  that adapts to 24-bit, 256-colour and `NO_COLOR` terminals, chosen with
  the config's `theme` or `--theme`.
- One overlay frame with a backdrop for every dialog, and a generic
  confirmation dialog.
- Toasts and a message history instead of the single status message.
- A full-screen, multi-column help overlay built from the live key map.
- A fuzzy command palette with live key hints and parametrised commands.
- A highlighted SQL editor that runs one statement at a time.
- Typed result columns, grid logic and copy writers.
- Key bindings grouped by context, with key sequences (`<g><g>`) on a
  timer; configs in the old per-mode format are converted with a warning.
- `--print-config` prints the config in effect.
- `QRY_KEYRING=off` keeps passwords in memory instead of the OS keychain.
- Release assets from the binary: `qry generate man` writes the man page,
  and `qry generate completions <shell>` writes completion scripts for
  bash, zsh, fish, PowerShell and elvish.
- Release archives are reproducible and come with a combined `SHA256SUMS`;
  Linux binaries are static (musl), and macOS gets one universal binary.
- Packages for the AUR (`qry`, `qry-bin`), a Homebrew tap and a Scoop
  bucket, and a Nix flake.

### Changed

- The application is a pure reducer with views and a runtime, split into
  the `qry` binary and the UI-free `qry-core` library.
- Passwords are typed key by key into a buffer that is cleared when
  dropped and can never be printed.
- `config.json` is read as JSON5, so the commented defaults load as a user
  file.

### Removed

- The `i686-unknown-linux-gnu` release build. It can come back as
  `i686-unknown-linux-musl` if anyone needs it.
- The glibc Linux release builds, replaced by static musl builds that run
  on any distribution.

### Fixed

- Deleting a connection drops the tree's indexes to it instead of keeping
  stale ones.

## [0.1.0]

The first version, before the redesign.

### Added

- A terminal SQL client built on ratatui, with a connection tree, a SQL
  editor and a results table.
- Drivers for PostgreSQL, MySQL, MariaDB, SQLite and Oracle (Oracle
  through the pure-Rust thin driver, with no Instant Client).
- Adding, editing and deleting saved connections.
- Passwords kept in the OS keychain, with a prompt when none is stored.
- Importing connections from DBeaver and SQL Developer.
- Exporting and importing connections as a qry backup file.
- Exporting results to CSV, plain text, JSON and Excel (`.xlsx`).
- Layered configuration (JSON, JSON5, YAML, TOML or INI) with configurable
  key bindings, and a log file in the data directory.
- Release builds for Linux, macOS and Windows.

[Unreleased]: https://github.com/viperh/qry/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/viperh/qry/releases/tag/v0.1.0
