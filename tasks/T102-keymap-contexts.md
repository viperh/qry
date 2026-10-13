# T102 — Keymap redesign: contexts and chord display

**Phase:** 1 — Architecture · **Depends on:** T101 · **Blocks:** T204, T206, T401, and every feature task that adds bindings

## Description

### Current state

Keybindings live in two unrelated systems:

1. **Mode bindings.** `config.rs` `KeyBindings(HashMap<Mode, HashMap<Vec<KeyEvent>, Action>>)`, filled from the `"keybindings"` section of `.config/config.json`. They are matched in `app.rs` `handle_key_event`, including multi-key sequences buffered in `last_tick_key_events` and cleared every tick.
2. **Pane bindings.** In `keymap.rs`, each pane has its own command enum (`EditorCommand`, `ResultsCommand`, `TreeCommand`, `FormCommand`, `HelpCommand`) in a `Keymap<C>(HashMap<KeyEvent, C>)`. They are grouped in `PaneKeys` with `merge_defaults` and come from the `"panes"` section.

`keymap.rs` `display_key` and `config.rs` `key_event_to_string` are two separate key-to-text functions. Defaults include odd choices: `i/k/j/l` for up/down/left/right, `alt-c` copy, `ctrl-u` undo. Nothing detects two actions bound to the same chord. The help overlay (`components/help.rs`) stitches both systems together by hand.

### Target design

The model is tachy's single keymap (`crates/tachy/src/keymap.rs`, `chord_to_string` at line 177, `display_chord`; defaults in `.config/config.json`), keyed by context.

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, EnumIter, Display)]
pub enum KeyContext { Global, Tree, Editor, Results, Inspector, Dialog, Help, Palette, Jobs }

/// The one command vocabulary. The reducer (T101) interprets it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, EnumIter)]
pub enum Action {
    Quit, Suspend, Help, Palette, FocusNext, FocusPrev, FocusTree, FocusEditor, FocusResults,
    NewConnection, EditConnection, DeleteConnection, ImportConnections, ExportConnections,
    SwitchSession(u8), NextSession, PrevSession, CloseSession,
    RunStatement, RunAll, RunSelection, CancelQuery, ExportResults,
    Up, Down, Left, Right, PageUp, PageDown, Top, Bottom, LineStart, LineEnd,
    Toggle, Open, Refresh, Filter, Search, CopyCell, CopyRow, Inspect, ToggleInspector,
    ToggleJobs, ToggleHints, Cancel, Submit, NextField, PrevField,
    // editor-only text commands
    WordLeft, WordRight, SelectLeft, /* ... */ Undo, Redo, Cut, Copy, Paste,
}
impl Action { pub fn description(&self) -> &'static str; }

pub struct Keymap {
    by_context: HashMap<KeyContext, Trie<KeyEvent, Action>>,   // supports sequences such as g g
}
impl Keymap {
    pub fn resolve(&self, ctx: &[KeyContext], pending: &[KeyEvent]) -> Lookup; // Match(Action) | Prefix | None
    pub fn chords_for(&self, ctx: KeyContext, a: &Action) -> Vec<Vec<KeyEvent>>;
}
pub fn chord_to_string(chord: &[KeyEvent]) -> String;   // "ctrl-p", "Enter", "←", "g g"
pub fn display_chord(chords: &[Vec<KeyEvent>]) -> String; // plain and shortest first, for the hint bar
```

**Resolution order.** The reducer calls `resolve` with a context stack, innermost first:
- `[Help]`, `[Palette]` or `[Dialog]` when an overlay is open. These are exclusive: no Global fallback, except `ctrl-q` stays `Quit` in Dialog.
- Otherwise `[focused pane, Global]`.

A partial sequence is kept in `State.pending_keys`. A `ScheduleTimer` of 1 s clears it; this replaces the per-tick drain.

**Config schema:**
```json
{
  "keybindings": {
    "Global":  { "<ctrl-q>": "Quit", "<f1>": "Help", "<ctrl-p>": "Palette" },
    "Results": { "<g><g>": "Top", "<shift-g>": "Bottom", "<y>": "CopyCell", "<h>": null }
  }
}
```
- User files merge over the embedded defaults per context and per chord.
- `null` unbinds a default.
- An unknown action or context name is a load error naming the file, context and chord.

**Conflicts.** After the merge:
- Two chords in one context where one is a strict prefix of the other is a load warning. The shorter chord wins only after the timeout; it is reported once as an error toast through T205 and logged.
- The same chord mapping to two actions cannot happen in a map, so it is never a conflict.
- A Global chord shadowed by a pane chord is allowed and documented.

**Migrating the old format.** If the config has a `"panes"` section or a mode key (`Home`, `AddConnModal`, …):
- Translate it automatically: `Home` becomes `Global`; `AddConnModal`/`ExpoModal`/… become `Dialog`; `panes.Editor` becomes `Editor`; and so on.
- Map old command names to the new `Action` names through a fixed table.
- Show a one-time warning toast: `config uses the old keybinding format; it was converted, run qry --print-config to see the new one`.
- A key that cannot be mapped is listed in the warning and skipped. This is the decision for this task: migrate, don't reject.

**Defaults** (they replace the current ones; vim-ish `j/k` replaces `i/k`):

| Context | Bindings |
|---|---|
| Global | `ctrl-q` Quit · `F1` Help · `ctrl-p` Palette · `Tab` / `shift-Tab` FocusNext/Prev · `ctrl-n` NewConnection · `ctrl-1`…`ctrl-9` SwitchSession(n) · `ctrl-PgDn`/`ctrl-PgUp` Next/PrevSession · `ctrl-z` Suspend (unix) |
| Tree, Results, Inspector, Jobs | `?` Help · `:` Palette · `j`/`↓` Down · `k`/`↑` Up · `h`/`←` Left · `l`/`→` Right · `g g` Top · `G` Bottom |
| Editor | `ctrl-Enter` and `F5` RunStatement · `F8` RunAll · `ctrl-e` RunSelection · `ctrl-c` CancelQuery (only while a job runs; otherwise Copy) · standard text editing (`ctrl-z` Undo inside the editor, `ctrl-shift-z`/`ctrl-y` Redo, `ctrl-x/c/v`) |
| Tree | `Enter` Open/connect · `Space` Toggle · `e` Edit · `Del` Delete · `r` Refresh · `/` Filter |
| Results | `y` CopyCell · `Y` CopyRow · `/` Search · `i` ToggleInspector · `Enter` Inspect (value popup) · `ctrl-s` ExportResults |
| Dialog | `Enter` Submit · `Esc` Cancel · `Tab`/`shift-Tab` Next/PrevField |
| Help | `j`/`k` scroll · `PgUp`/`PgDn` · `Home`/`G` · `?`/`Esc` close |
| Palette | `Enter` Submit · `Esc` Cancel · `↑`/`↓` |

Terminals that cannot send `ctrl-Enter` fall back to `F5`. The hint bar (T202) uses `display_chord` so the bar shows `F5` when that is the only chord that works. `?` and `:` are deliberately not bound in Editor, so they can be typed.

**Mode pill** (drawn by T202, decided here because it derives from the context):
- `NORMAL` (teal) when Tree, Results, Inspector or Jobs is focused.
- `EDIT` (amber) when Editor is focused.
- `COMMAND` (purple) when the palette is open.
- Help and dialogs keep the pill of the context underneath (`State.pill`).

`chord_to_string` rules, as in tachy:
- Modifiers become `ctrl-`, `alt-`, `shift-`; `shift-` is dropped for uppercase letters (`G`, not `shift-g`).
- Named keys: `Enter Esc Tab Space Backspace Del PgUp PgDn Home End Ins F1…F12`.
- Arrows: `← → ↑ ↓`.
- A sequence is joined with a space.

Delete `config.rs` `key_event_to_string` and `keymap.rs` `display_key`, and keep one parser (`parse_key_sequence`).

### Out of scope

- Mouse bindings.
- A per-user keybinding editor UI.
- Rendering the help overlay (T204) and the palette (T206). This task only provides the data they read.

## Acceptance criteria

- [x] `KeyContext` and a single `Action` enum exist; `EditorCommand`, `ResultsCommand`, `TreeCommand`, `FormCommand`, `HelpCommand`, `PaneKeys` and the `Mode`-keyed `KeyBindings` are deleted.
- [x] `.config/config.json` uses the new schema with every default in the table above. Every `Action` variant has a non-empty `description()`.
- [x] A user config can add, override and unbind (`null`) per context; an unknown action name fails to load with a message naming file, context and chord.
- [x] Multi-key sequences work with a 1 s timeout driven by `Effect::ScheduleTimer`, not by tick drains.
- [ ] Prefix conflicts produce exactly one warning toast at startup and a log line.
- [ ] An old-format config is converted automatically with a single warning toast; unmappable keys are listed.
- [x] `qry --print-config` prints the merged effective keymap in the new schema.
- [x] `chord_to_string` and `display_chord` are the only key-to-text functions in the crate.
- [x] `?` and `:` type literally in the editor; `F1` and `ctrl-p` work everywhere except inside a dialog text field, where `ctrl-p` is passed through.
- [x] `State.pill` follows the rules above.

## Tests

`crates/qry/src/keymap_tests.rs`:
- `every_action_has_a_description`: iterate `Action::iter()`.
- `every_default_binding_parses`: load `Config::embedded()` with no warnings.
- `chord_to_string_names` (table test): `ctrl-p`, `Enter`, `shift-Tab`, `←`, `G`, `F5`, `g g`, `alt-Del`.
- `display_chord_prefers_plain_shortest`: `[ctrl-Enter, F5]` gives `F5`; `[k, ↑]` gives `k`.
- `parse_and_print_round_trip` (proptest): a random chord passed through `chord_to_string` then `parse_key_sequence` is equal.
- `user_override_and_unbind_merge`: a user file setting `Results <h>: null` and `<x>: CopyCell` leaves the other defaults intact.
- `unknown_action_is_a_load_error_with_location`: the message contains the file path, `Results` and `<x>`.
- `prefix_conflict_reported_once`: `<g>` and `<g><g>` in one context give one warning.
- `resolve_overlay_contexts_are_exclusive`: with Help open, `ctrl-n` does nothing; `ctrl-q` in a dialog still quits.
- `pane_binding_shadows_global`: an Editor-local `Tab` (indent) beats Global `FocusNext`, and the help shows both entries.

`crates/qry/src/config_migrate_tests.rs`:
- `old_format_is_converted`: feed the pre-redesign `.config/config.json` from git history (stored as `tests/fixtures/config/legacy-0.1.json`). Assert the converted keymap has `Global ctrl-n → NewConnection` and `Editor F8 → RunAll`, and that one warning is reported.
- `unmappable_old_keys_are_listed`.

`crates/qry/src/app/reducer_tests.rs` (harness):
- `sequence_times_out_after_one_second`: press `g`, then `advance(1001)`, then `g`; no Top. `g g` within 1 s gives Top.
- `question_mark_types_in_editor_but_opens_help_in_results`.
- `pill_follows_focus`: Editor gives EDIT, Tab to Results gives NORMAL, open palette gives COMMAND, open help keeps the underlying pill.
- `ctrl_digit_switches_session` (uses the T103 fake sessions).

Snapshot:
- `print_config_default`: insta snapshot of `qry --print-config` output.

## Implementation notes

- **Two boxes stay unticked only for the toast.** Prefix conflicts and an old-format config are each reported as one warning line by `Config::warnings()`, which `Runtime::new` logs with `tracing::warn!` and prints to stderr as `qry: …` before the TUI starts, as T201 did for `styles`. `qry --print-config` prints the same lines to stderr. T205 turns them into toasts (an error toast for the conflicts, a warning toast for the migration) and can then tick both boxes.
- **Where things live.** `action.rs` holds `Action` (`Copy`, `Hash`, `EnumIter`, `Display`, serde; `description()` and `rank()` for declaration order). `keymap.rs` holds `KeyContext` (with `pill()`), `Keymap`, `Lookup::{Match, Prefix, None}`, `normalise`, `chord_to_string`, `display_chord`, `parse_key_sequence` and `parse_key_event` (the parser moved here from `config.rs`). `config_migrate.rs` reads the old format. `Config` is now `{ keymap, theme, warnings }` and no longer `Deserialize`; each file is read into a private `RawFile` (`theme`, `keybindings`, `panes`, `styles` as `serde_json` values) and layered by `Config::layered`.
- **`Keymap` is a `HashMap<KeyContext, HashMap<Vec<KeyEvent>, Action>>`, not a trie.** There are a few dozen bindings per context, so `resolve` scans for a longer binding that starts with the pending keys. crossterm's `KeyCode`/`KeyModifiers` are not `Ord`, which ruled out a `BTreeMap` range. API: `bind`, `unbind`, `resolve(&[KeyContext], &[KeyEvent]) -> Lookup`, `exact` (what a waiting sequence runs at the timeout), `chords_for(ctx, &action)` (display order), `bindings(ctx)`, `prefix_conflicts()`.
- **Resolution.** The first context in the stack that knows the keys answers; a binding that is both complete and the start of a longer one is `Prefix`, and only runs (via `exact`) when the timer fires. `State::contexts()` is `[Help]` or `[Dialog]` for the top overlay, else `[focused pane, Global]`. Actions that open things, focus, quit, suspend and the not-yet-live ones are handled whatever the context; the rest go to the innermost context's handler (`tree::run`, `editor::run`, `results::run`, `dialogs::act`, `help::act`). Before any lookup, a plain character for the open dialog's focused text field is typed (so Space types and ctrl-p, unbound in `Dialog`, falls through to the field and is ignored). An unbound key in the editor is typed. If a key breaks a waiting sequence, the waiting keys' exact binding (if any) runs and the key is handled afresh.
- **Normalisation** (`keymap::normalise`), applied to every bound chord and every looked-up key: only Ctrl/Alt/Shift; a character without Ctrl drops Shift (`G`, `?`); with Ctrl it is lower-cased **and keeps Shift**, so `ctrl-shift-z` (Redo) differs from `ctrl-z` (Undo) on terminals that report it (tachy dropped Shift there, which would have merged them); AltGr (Ctrl+Alt on a non-alphanumeric) is the plain character; `BackTab` always has Shift and Shift+Tab becomes `BackTab`.
- **Syntax.** `parse_key_sequence` reads both `<ctrl-q>` / `<g><g>` (the config's) and `chord_to_string`'s output (`ctrl-q`, `g g`, `shift-Tab`, `←`, `PgDn`, `Del`); the `<…>` wrapper is optional per chord, a single character keeps its case, and `lt`/`gt` spell `<`/`>`. `qry --print-config` writes keys with `chord_to_string`, so no third key-to-text function exists; `printed_config_loads_back_to_the_same_keymap` reads the output back on its own (not over the defaults) and gets the identical keymap.
- **Timers.** `Effect::ScheduleTimer { timer: Timer, after: Duration }` and `UiEvent::Timer(Timer)` with `Timer::KeySequence(u64)`. Each key that leaves a sequence waiting bumps `State::key_timer` and schedules a 1 s timer (`keys::SEQUENCE_TIMEOUT`), so each key restarts the wait; a timer whose number is not the current one is ignored, so there is no `CancelTimer`. The runtime spawns a `tokio::time::sleep` and sends the event on the executor's event channel. The harness delivers timers on its virtual clock in `advance` (its `Pending` now holds `UiEvent`s). Ticks no longer touch `pending_keys`.
- **Config loading.** `Config::new()` reads each existing user file with the `config` crate on its own (same file list and order as before; a later file wins per chord), so errors name the file: an unknown context lists the contexts, a bad key or an unknown action names `file`, `keybindings.<Context>` and the chord. `null` unbinds. `Config::with_user_file(path, json)` (test-util) loads the defaults plus one JSON file the way `config.json` is read. The old builder defaults `data_dir`/`config_dir` were dropped (nothing read them).
- **Migration.** A file with a `panes` section or an old mode name (`Home`, `AddConnModal`, `ExpoModal`, `PasswordModal`, `ImportModal`, `ConnExportModal`) is converted: `Home` → `Global`, the modals → `Dialog`, `panes.{Editor,Results,Tree,Form,Help}` → `Editor`, `Results`, `Tree`, `Dialog`, `Help`. `ChangeMode(AddConnModal|ExpoModal|ImportModal|ConnExportModal|Home)` → `NewConnection|ExportResults|ImportConnections|ExportConnections|Cancel`; renamed commands: `RunQuery`→`RunAll`, `FirstColumn`/`LastColumn`→`LineStart`/`LineEnd`, `FirstRow`/`First`→`Top`, `LastRow`/`Last`→`Bottom`, `Connect`→`Open`, `Edit`→`EditConnection`, `Delete`→`DeleteConnection`, `ScrollUp`/`ScrollDown`→`Up`/`Down`, `Close`→`Cancel`. Skipped and listed: `Execute`, `ChangeMode(PasswordModal)`, unknown names, unparsable keys, unknown panes (`panes.X`). The warning is `<file>: config uses the old keybinding format; it was converted, run qry --print-config to see the new one` plus `; these keys could not be converted and were skipped: Home <ctrl-x>: {"Execute":"…"}, …`. The fixture `tests/fixtures/config/legacy-0.1.json` is `.config/config.json` at `1d639ec` (before T001, with `styles`). Loaded as a user file it also gives the `styles` warning and a prefix-conflict warning (its Results `<g>` FirstRow becomes `g` Top next to the default `g g`); the test counts the migration warnings, which are exactly one.
- **Defaults beyond the table**, so nothing became unreachable before the palette: Global `ctrl-o` ImportConnections; Tree `ctrl-s` ExportConnections, `Home`/`End` Top/Bottom; Results `PgUp`/`PgDn`, `Home`/`End` LineStart/LineEnd (first/last column); the editor keeps all its old text keys (`ctrl-y` is now Redo, Paste is `ctrl-v`, Copy is `ctrl-c`, `alt-c`/`ctrl-u`/`ctrl-r` are gone); Dialog also has `←`/`→`/`Home`/`End`/`Backspace`/`Del`, `Space` NextChoice, `F1` Help and `ctrl-q` Quit; Help also has `↓`/`↑`, `End`, `F1`, `ctrl-p` and `ctrl-q`; Palette also has `ctrl-p`, `F1` and `ctrl-q`. `ctrl-e` no longer exports (it is RunSelection); exporting results is `ctrl-s` in Results. `ctrl-h` is no longer Help (`F1` is). Inspector and Jobs have the shared navigation only.
- **Actions added beyond the spec list:** `ClearScreen` (kept, unbound), `NextChoice` (dialog choice fields), `NewLine`, `Indent` (inserts a tab; used by `pane_binding_shadows_global`), the editor's select/delete commands. **Actions that do nothing yet**, bound so the defaults do not move later: `Palette` (T206), `SwitchSession`/`NextSession`/`PrevSession`/`CloseSession` (T103), `Filter` (T301), `Search`/`CopyCell`/`CopyRow`/`Inspect`/`ToggleInspector` (T303/T304), `ToggleJobs` (T305), `ToggleHints` (T202). `CancelQuery` always copies, as no job can be cancelled before T305. `RunStatement` runs the whole text until T302 splits statements; `RunSelection` runs the selection, or the whole text without one. `Suspend` only emits `Effect::Suspend` on unix (`cfg!(unix)`), so the binding is the same everywhere and the print-config snapshot is platform-independent. Tree `h`/`←` collapses (or goes from a table up to its connection), `l`/`→` expands, `r` re-reads the tables of the open connection.
- **Pill.** `State::pill()` (a method, not a stored field): the topmost overlay whose context has a pill, else the focused pane's (`KeyContext::pill`: Tree/Results/Inspector/Jobs NORMAL, Editor EDIT, Palette COMMAND, Help/Dialog/Global none). There is no palette overlay yet, so `pill_follows_focus` checks COMMAND through `KeyContext::Palette.pill()`; T206 adds `Overlay::Palette` and must map it to `KeyContext::Palette` in `State::contexts()` and `State::pill()`.
- **Help (`reducer/help.rs`).** `HelpState::from_config` lists Global, Connection tree, Editor, Results, Dialogs and Help (Inspector, Jobs and Palette are left out until they exist), one line per action in declaration order with its chords in display order joined by ` / `; consecutive actions with the same description (the nine `SwitchSession`s) share a line written `ctrl-1 … ctrl-9`. `HelpState::scroll` takes an `Action` (Up/Down/PageUp/PageDown/Top/Bottom). The help snapshot was re-accepted after review: new keys and descriptions, `F1` opens it.
- **Removed:** `KeySection`, `Action::ChangeMode`, `Action::Execute`, `State::key_section`, `State::asked` (the prompt's name now goes straight into `PasswordForm::new`), `DialogState::section`, `config::KeyBindings`, `PaneKeys`, the five command enums, `display_key`, `key_event_to_string`. Forms take `Option<Action>` in `handle_key` and gained `is_typing()` and `act(Action)`.
- **Harness API changes.** `section()` is gone; `contexts() -> Vec<KeyContext>` and `dialog_title() -> Option<&'static str>` replace it. `parse_keys` uses `keymap::parse_key_event`, so `←`, `PgDn`, `F5` work and keys come back normalised (a lone capital still carries Shift, as terminals send it). Tests that exported results now focus the results and press `ctrl-s` (`reducer_tests::open_export`). `every_view_renders_at_every_size` now also asserts each opener really opened its overlay (the help and export openers had silently stopped opening anything).
- **`ctrl_digit_switches_session`** checks that `ctrl-3` resolves to `SwitchSession(3)` and changes nothing yet; T103 must turn it into the real test with its fake sessions.
- **Later tasks:** T005's snippet that calls `config::key_event_to_string` must use `keymap::chord_to_string` (the parse/print round-trip proptest now lives in `keymap_tests::parse_and_print_round_trip`; its regression file is `crates/qry/proptest-regressions/keymap_tests.txt`). T204 reads `HelpState`/`Keymap::chords_for`/`Action::description`; T202's hint bar should use `display_chord(&keymap.chords_for(ctx, &action))`. T206 lists `Action::iter()` with `description()` and `chords_for`. Feature tasks add an `Action` variant with a description, a default in `.config/config.json`, and a handler arm, and re-accept `print_config_default`.
- **Tests.** 254 before, 284 after: 239 `qry` unit tests, 2 in `tests/harness.rs`, 3 in `tests/no_raw_colors.rs`, 3 in the new `tests/print_config.rs` (runs the binary with a temporary `QRY_CONFIG`: defaults, an old-format override with its warning, an unknown action failing with the file's path), 6 in `tests/workspace_metadata.rs`, 31 in `qry-core`. New beyond the list: `the_defaults_are_the_documented_ones` (the whole table), parser and normalisation tests ported from `config_tests`, `chords_for_lists_every_chord_plain_first`, `the_pill_follows_the_context`, `a_broken_sequence_hands_on_the_key`, `a_prefix_binding_runs_once_the_sequence_times_out` (including a stale timer), `ticks_leave_a_half_typed_sequence_alone`, `overlays_take_the_keys_alone`, `space_types_in_a_text_field_and_picks_in_a_choice`, `tree_keys_follow_vim`, editor `f5_runs_the_statement_and_ctrl_e_the_selection` and `a_selection_over_lines_runs_as_written`, `printed_config_loads_back_to_the_same_keymap`, the config error-location tests, `runtime_tests::a_scheduled_timer_comes_back_as_an_event_once_it_runs_out` and `cli_tests::print_config_is_a_flag`. The determinism proptest now also sends `Timer` events and the new default chords. `print_config_default` lives in `config_tests` (snapshot under `crates/qry/src/snapshots/`).
- **Checks run locally:** `cargo fmt --all` (stable, then nightly rustfmt for import grouping), clippy with `-D warnings`, `QRY_KEYRING=off cargo test --workspace --all-features --locked`, the CI docs command with `RUSTDOCFLAGS=-D warnings`, `cargo check --workspace --all-features --locked` with Rust 1.96, and a pty smoke run (F1, Esc, Tab, typing `?:` in the editor, F8, `g g`, `G`, ctrl-n, Esc, ctrl-q; exit status 0).

