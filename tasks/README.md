# qry redesign — task index

A full redesign of qry's interface and functionality.

- **Look and feel** come from `~/Projects/tachy`: the `Theme` palette (dark, amber/teal/purple/coral accents), borderless main screen, top bar with a mode pill, status line, key-hint bar, toasts, and the full-screen help overlay.
- **Testing and CI/CD** come from `~/Projects/sverb`: insta snapshot tests through an app harness, proptest and cargo-fuzz twins, database service containers, cargo-deny, an MSRV check, layering checks, and a tag-driven release with reproducible archives and packaging channels.

Each task file has a **Description**, **Acceptance criteria** and **Tests** section. A task is done when every acceptance box is ticked, every listed test exists and passes, and CI is green.

## Conventions every task follows

- Test placement follows sverb:
  - Unit tests go in sibling files (`foo.rs` with `#[cfg(test)] mod foo_tests;`), not inline `mod tests {}`.
  - Integration tests go in `crates/<crate>/tests/`, with shared helpers in `tests/common/mod.rs`.
  - Snapshot names include the terminal size, e.g. `help_120x40`.
- Drawing code never names a colour. Every style comes from a `Theme` helper (T201), and the `no_raw_colors` test enforces it.
- `qry-core` never depends on `ratatui`, `crossterm` or `clap`. `cargo-deny` `wrappers` and the layering test enforce it (T001, T003).
- Every cargo command in CI uses `--locked`.
- Commits use conventional-commit prefixes (`feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `test:`, `ci:`, `build:`, `chore:`).

## Tasks

| ID | Title | Depends on |
|---|---|---|
| **Phase 0 — Foundation** | | |
| [T001](T001-workspace-hygiene.md) | Workspace hygiene and lint baseline | — |
| [T002](T002-test-infrastructure.md) | Test infrastructure: harness, snapshots, conventions | T001 |
| [T003](T003-ci-pipeline.md) | CI pipeline | T001, T002 |
| [T004](T004-database-integration-tests.md) | Database integration test suite | T002, T003 |
| [T005](T005-property-tests-and-fuzzing.md) | Property tests and fuzzing | T002, T003 |
| [T006](T006-benchmarks.md) | Benchmarks and performance gates | T003 |
| [T007](T007-release-pipeline.md) | Release pipeline (CD) and packaging | T003 |
| **Phase 1 — Architecture** | | |
| [T101](T101-state-reducer-architecture.md) | App state and reducer architecture | T002 |
| [T102](T102-keymap-contexts.md) | Keymap redesign: contexts and chord display | T101 (warnings shown as toasts once T205 lands) |
| [T103](T103-multi-connection-sessions.md) | Multiple open connections (sessions) | T101 |
| **Phase 2 — Visual system** | | |
| [T201](T201-theme-system.md) | Theme system ported from tachy | T001 |
| [T202](T202-screen-layout-and-chrome.md) | Screen layout and chrome | T101, T201 |
| [T203](T203-overlay-framework.md) | Overlay framework and dialog restyle | T201, T202 |
| [T204](T204-help-overlay.md) | Help overlay (tachy-style) | T102, T203 |
| [T205](T205-toasts.md) | Toasts and notifications | T202 |
| [T206](T206-command-palette.md) | Command palette | T102, T203 |
| **Phase 3 — Features** | | |
| [T301](T301-schema-browser.md) | Schema browser | T103, T202 |
| [T302](T302-sql-editor.md) | SQL editor: highlighting and statement execution | T201, T202 |
| [T303](T303-results-grid.md) | Results grid redesign | T201, T202 |
| [T304](T304-record-inspector.md) | Record inspector and value popup | T303 |
| [T305](T305-query-jobs.md) | Query jobs: progress, cancellation, timing, row limits | T103, T202 |
| [T306](T306-query-history.md) | Query history and saved snippets | T302 |
| [T307](T307-export-redesign.md) | Export redesign | T203, T305 |
| [T308](T308-connection-dialogs.md) | Connection management dialogs | T203, T103 |
| **Phase 4 — Docs** | | |
| [T401](T401-documentation.md) | Documentation and generated references | T102, T204 |

## Suggested order

T001, then T002, then T003. After that, T201 and T101 can run in parallel, then T102, T202, T203. Phase 3 tasks are mostly independent once their dependencies land. T004–T007 can be done any time after T003.
