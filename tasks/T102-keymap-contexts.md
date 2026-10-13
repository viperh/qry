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

- [ ] `KeyContext` and a single `Action` enum exist; `EditorCommand`, `ResultsCommand`, `TreeCommand`, `FormCommand`, `HelpCommand`, `PaneKeys` and the `Mode`-keyed `KeyBindings` are deleted.
- [ ] `.config/config.json` uses the new schema with every default in the table above. Every `Action` variant has a non-empty `description()`.
- [ ] A user config can add, override and unbind (`null`) per context; an unknown action name fails to load with a message naming file, context and chord.
- [ ] Multi-key sequences work with a 1 s timeout driven by `Effect::ScheduleTimer`, not by tick drains.
- [ ] Prefix conflicts produce exactly one warning toast at startup and a log line.
- [ ] An old-format config is converted automatically with a single warning toast; unmappable keys are listed.
- [ ] `qry --print-config` prints the merged effective keymap in the new schema.
- [ ] `chord_to_string` and `display_chord` are the only key-to-text functions in the crate.
- [ ] `?` and `:` type literally in the editor; `F1` and `ctrl-p` work everywhere except inside a dialog text field, where `ctrl-p` is passed through.
- [ ] `State.pill` follows the rules above.

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
