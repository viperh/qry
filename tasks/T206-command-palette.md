# T206 — Command palette

**Phase:** 2 — Visual system · **Depends on:** T102, T203 · **Blocks:** T306 (recent-query items), T401

## Description

### Current state

qry has no command palette. Every operation needs a dedicated chord, and some operations exist only as chords in one specific mode:
- `Home` mode: `<Ctrl-n>` new connection, `<Ctrl-e>` export, `<Ctrl-o>` import, `<Ctrl-s>` export connections.
- Tree pane: `e` edit, `<delete>` delete.

So a feature is only reachable if the user remembers the chord, and nothing can be found by name. As the redesign adds many new actions (sessions, inspector, history, maximise, theme, messages…), the palette becomes the main way to discover them.

### Target design

A tachy-style fuzzy command palette.

**Opening and closing**
- `ctrl-p` (everywhere) or `:` (when focus is not in the editor) runs the `OpenPalette` action.
- While the palette is open, the mode pill shows ` COMMAND ` in purple (T202).
- `Esc` closes it. `Enter` runs the selected item and closes it, unless the item needs an argument.

**Geometry**
- 96 columns wide, or `width − 8` when the terminal is narrower.
- Horizontally centred and top-anchored at row 3.
- Height = 2 (the input row plus the bottom border) + `min(matches, 17)` rows, at most 20 in total.
- Drawn with the T203 frame, using the purple accent, **no title**, and the backdrop behind it.

**Input row:** a `› ` prompt in `query_prompt` (bold amber) followed by the query with a block cursor. On the right, in dim: `14 / 63`, the number of matches out of the total.

**Item rows**
- The command label is in fg, with its fuzzy-matched characters in `palette_match` (bold amber).
- A category tag follows in dim (`connection`, `session`, `results`, `history`…).
- The item's current key chord, if any, is right-aligned in `hint_key` (from T102's `display_chord`).
- The selected item uses `palette_item_selected` (purple on purple_tint).
- `↑`/`↓`/`ctrl-k`/`ctrl-j` move the selection, `PgUp`/`PgDn` move a page, and the list scrolls to keep the selection visible.

**Items**
1. **Static commands:** every `Action` that has `palette_label()` (set in T102). Purely internal actions such as `Tick` and `Render` have none. Examples: `New connection`, `Edit connection`, `Delete connection`, `Import connections`, `Export connections`, `Export results`, `Run statement`, `Run all`, `Toggle inspector`, `Toggle jobs`, `Toggle hints`, `Maximise pane`, `Show messages`, `Help`, `Quit`.
2. **Dynamic items**, gathered from the reducer state at open time:
   - `Open connection: prod` for each stored connection (category `connection`).
   - `Switch to session 2: prod` for each open session (`session`, T103).
   - `Recent: select * from users where…` for the last 20 history entries (`history`, T306). Running one loads it into the editor without running it.
   - `Table: public.users`, which runs a preview `SELECT … LIMIT n` once the schema has loaded (`table`, T301).
3. **Parametrised commands** take a typed argument after the command name:
   - `export csv <path>`, `export json <path>`, `export xlsx <path>`, `export tsv <path>`
   - `limit <rows>` (sets the row cap for the session, T305)
   - `goto <row>` (results)
   - `theme <name>` (T201, validated against `Theme::NAMES`)

   `Tab` on a parametrised item fills the input with `export csv ` and the cursor waits for the argument. `Enter` with an argument runs the command, and `Enter` without one shows a coral `needs <path>` in the counter slot instead.

**Fuzzy matching**
- Case-insensitive subsequence matching over `label + " " + category`.
- Score: consecutive-match bonus, word-start bonus (after a space, `·`, `_`, `.` or `:`), a first-character bonus and a gap penalty, with ties broken by a recency rank.
- **Recency:** the last 10 palette items that were run are stored in the UI state and persisted with the history file (T306) if that task has landed. An empty query lists recent items first, then static commands in definition order.
- The matcher is a pure function, so it is easy to unit test and fuzz (T005 can add a proptest twin).

### Mockup at 120x40

```
 qry  1 scratch  2 prod                                                         postgres · app@db.local   COMMAND 
 SCHEMA                        │ QUERY  ln 1, col 1
 ▾ prod     ┌──────────────────────────────────────────────────────────────────────────────────────────────┐
   ▾ public │ › exp█                                                                              5 / 63 │
            │ Export results                      results                                         ctrl-e │
            │ Export connections                  connection                                      ctrl-s │
            │ export csv <path>                   results                                                │
            │ export json <path>                  results                                                │
            │ Import connections                  connection                                      ctrl-o │
            └──────────────────────────────────────────────────────────────────────────────────────────────┘
```

In the first item, "Exp" is bold amber and the whole row is purple on purple_tint.

### Mockup at 80x24

```
    ┌────────────────────────────────────────────────────────────────────────┐
    │ › █                                                           63 / 63 │
    │ Recent: select * from users where city = 'Lis…   history              │
    │ New connection                                    connection   ctrl-n │
    │ ...                                                                   │
    └───────────────────────────────────────────────────────────────────────┘
```

### tachy reference

- `~/Projects/tachy/crates/tachy/src/components/palette.rs`: the palette, `palette_rect` (96 wide or width−8, row 3, ≤20 rows), the fuzzy match with highlighted characters, argument filling with Tab, and the purple border with no title.
- `~/Projects/tachy/crates/tachy/src/theme.rs`: `palette_border`, `palette_item_selected` and `palette_match`.
- `~/Projects/tachy/crates/tachy/src/components/layout.rs`: `palette_rect`.

### Proposed modules

```
crates/qry/src/palette/
  mod.rs        PaletteState, items(), run()           + palette_tests.rs
  fuzzy.rs      score(query, haystack) -> Option<Match> + fuzzy_tests.rs
crates/qry/src/ui/overlay/palette.rs   draw            + palette_view_tests.rs
```

```rust
pub struct PaletteItem {
    pub label: String,
    pub category: &'static str,
    pub key: Option<String>,          // display chord, resolved at open
    pub target: PaletteTarget,
}

pub enum PaletteTarget {
    Action(Action),
    WithArg { prefix: &'static str, make: fn(&str) -> Result<Action, String>, hint: &'static str },
}

pub struct Match { pub score: i32, pub positions: Vec<usize> }

pub fn score(query: &str, haystack: &str) -> Option<Match>;

pub struct PaletteState {
    pub input: TextInput,
    pub items: Vec<PaletteItem>,
    pub matches: Vec<(usize, Match)>, // sorted
    pub selected: usize,
    pub offset: usize,
}
```

Running an item dispatches its `Action` through the normal reducer path, so keybound and palette-run commands behave identically. This is tested by comparing effects.

### Out of scope

- User-defined palette commands or macros.
- Fuzzy search over table data. That belongs to results search (T303).

## Acceptance criteria

- [ ] `ctrl-p` opens the palette from any pane, and `:` opens it from the tree and results but types `:` in the editor.
- [ ] The pill shows COMMAND while the palette is open.
- [ ] Geometry: 96 wide (or width−8), top at row 3, at most 20 rows, purple border and no title, with the backdrop behind it.
- [ ] Every user-facing `Action` with a `palette_label` is listed, and an exhaustive test checks that no user-facing action lacks one.
- [ ] Each item's key column shows the live chord, so a remap updates it.
- [ ] Dynamic items list every stored connection and open session, plus recent queries and tables when T306/T301 are present.
- [ ] Matched characters are bold amber, and the selected row is purple on purple_tint.
- [ ] The counter shows `matches / total`.
- [ ] The ranking is deterministic. Word-start and consecutive matches outrank scattered ones, and with an empty query recent items come first.
- [ ] `Tab` fills parametrised commands. A missing argument shows a coral `needs <…>` without closing, and an invalid argument (`limit abc`) shows the parse error.
- [ ] Running from the palette produces the same effects as the bound key.
- [ ] Esc closes the palette with no side effects, and the previous mode and focus are restored.
- [ ] Renders at 80x24 and 300x100 without a panic, and no raw colours are used.

## Tests

`crates/qry/src/palette/fuzzy_tests.rs`:
- `subsequence_required`: `xpt` matches `Export` and `zz` doesn't.
- `case_insensitive`.
- `word_start_beats_middle`: `ec` ranks `Export connections` above `Select columns`.
- `consecutive_beats_scattered`.
- `positions_are_char_indices_for_unicode_labels`.
- A proptest twin, `score_never_panics_and_positions_are_increasing_and_in_bounds`, matching sverb's fuzz-twin pattern and wired into T005's fuzz targets.

`crates/qry/src/palette/palette_tests.rs`:
- `every_user_action_has_a_palette_label`: an exhaustive `strum::IntoEnumIterator` loop with an explicit internal-actions allow list.
- `dynamic_items_include_connections_and_sessions`.
- `empty_query_lists_recent_first`.
- `tab_fills_parametrised_command`.
- `missing_argument_keeps_palette_open_with_error`.
- `invalid_limit_argument_shows_parse_error`.
- `palette_run_equals_keybound_effects`: the harness runs `Export results` both ways and compares the effects.
- `key_column_follows_remap`.

Reducer tests, in `crates/qry/src/app/palette_flow_tests.rs`:
- `ctrl_p_opens_everywhere_colon_not_in_editor`.
- `esc_restores_mode_and_focus`.
- `pill_shows_command_while_open`.

`crates/qry/src/ui/overlay/palette_view_tests.rs`, insta snapshots plain and styled:
- `palette_empty_80x24`, `palette_query_exp_120x40`, `palette_arg_needed_120x40`, `palette_scrolled_120x40` (selection past 17 items).
- `palette_rect_geometry`: widths 80, 104 and 200 give the expected rects.
