# T204 — Help overlay (tachy-style)

**Phase:** 2 — Visual system · **Depends on:** T102, T203 · **Blocks:** T401

## Description

The user asked for qry's help menu to be like tachy's. This task replaces qry's help with a port of tachy's full-screen, multi-column, keymap-generated help overlay.

### Current state

`crates/qry/src/components/help.rs`:

- `Help::from_config` builds `sections: Vec<(String, Vec<(String, String)>)>` in a fixed order from `config.keybindings` (modes) and `config.panes` (Editor, Results, Tree, Form, Help).
- `lines()` renders the sections as one long list: a `"{title}:"` line in bold, then indented `keys  description` lines.
- `draw()` puts the list in a centred `Block::bordered().title(" Help ")` with a bottom-border hint line, and scrolls it with `HelpCommand::{ScrollUp, ScrollDown, PageUp, PageDown}`.
- It is toggled by `Action::Help` (`<Ctrl-h>` in `Home` mode). `Home::handle_key_event` gives it every key while it is visible, and `Esc` closes it.
- Descriptions come from `describe_action`, a hardcoded match.
- Existing tests: `sections_come_from_the_config_in_a_fixed_order`, `a_rebound_key_shows_up_in_the_help` and `a_tall_help_scrolls_and_stops_at_the_end`.

The problems:
- One column wastes wide terminals.
- There are no colours.
- Descriptions are not tied to the actions themselves.
- Duplicate chords are listed separately.
- There is no indication of the scroll position.

### Target design

**Opening and closing**
- `?` (outside the editor) or `F1` (everywhere) toggles the overlay, through the `Help` action. `Esc` or `Help` again closes it.
- While it is open, every other action is swallowed except help scrolling: the `Help` key context (T102) is the only active context.
- Opening sets `ui.overlay = Some(Overlay::Help)` and saves `pill_mode`, so the top-bar pill (T202) keeps showing the previous mode.
- Opening always starts at scroll 0.

**Frame**
- The overlay covers the **whole screen**, drawn over the backdrop (T203) with `dialog_frame` and the teal accent.
- Title: ` Help — qry {CARGO_PKG_VERSION} `.
- Content area = inner rect minus a 1-cell margin on each side, so the usable width is `width − 4`.

**Columns**
- `column_count(width)` is 3 when width ≥160, 2 when ≥100, otherwise 1.
- `COLUMN_GAP = 3`.
- Column width = `(usable − gap*(n−1)) / n`.

**Packing**
- Groups flow top to bottom, then across columns, in their defined order.
- `pack(groups, n, height)` starts the height limit at `ceil(total_lines / n)` and raises it until all groups fit in `n` columns, so the tallest column is as short as possible.
- A group never splits across columns, unless it is taller than the limit on its own, in which case it overflows into the scroll.
- A continuation group (`continues: true`) attaches to the previous group with no blank line between them, and is never separated from it.
- There is one blank line between other groups.

**Lines**
- **Group title:** bold teal (`theme.dialog_border(teal) + BOLD`).
- **Entry:** keys in `hint_key` (bold amber), padded to the key column width, then two spaces, then the description in `hint` (dim).
  - The key column width is the widest key string in that *screen column*, capped at half the column width.
  - Over-long keys and descriptions are truncated with `…` (`text::truncate_end`, which is display-width aware).
- **Note:** dim, word-wrapped to the column width.

**Footer cheat-sheet.** It is pinned to the rows just above the bottom border and is not part of the scroll. Each item is a run of pieces: the label piece (`run:`) in the title style, key pieces in `hint_key`, and other text dim. Items wrap as whole units, separated by two spaces. Initial items:
- `run:  ctrl-enter statement  F8 all  ctrl-enter on a selection runs the selection`
- `results:  / search  y copy cell  Y copy row  enter inspect value`
- `tree:  enter connect / expand  e edit  del delete  / filter`

The keys in the footer are resolved from the keymap too, not hardcoded.

**Bottom border**
- On the left, the help's own keys come from the `Help` context in tachy's `HELP_KEYS` style: `k/j scroll  PgUp/PgDn page  Home/G top/bottom  ?/Esc close`.
- When the content is taller than the area, a position indicator ` 1–35 of 66 ` sits at the right of the border.

**Scrolling**
- One shared offset applies to all columns.
- `j`/`↓` move 1 line, `k`/`↑` move back 1 line, `PgDn`/`PgUp`/`ctrl-d`/`ctrl-u` move one page, and `Home`/`G` jump to the top or bottom.
- The offset is clamped to `max_scroll`, which is computed during draw from the packed height minus the visible height and stored back into the help state.
- A resize re-clamps the offset.

**Content model** (`help_model.rs`, built from the **live merged keymap** in `register_config_handler` and rebuilt on config reload):

```rust
pub struct HelpGroup {
    pub title: &'static str,
    pub entries: Vec<(String, String)>, // (keys, description)
    pub notes: Vec<&'static str>,
    pub continues: bool,
}

pub const GROUPS: &[GroupSpec] = &[
    GroupSpec::context("Global", KeyContext::Global),
    GroupSpec::context("Tree", KeyContext::Tree),
    GroupSpec::subset("Editor · Editing", KeyContext::Editor, EDITOR_EDITING),
    GroupSpec::subset("Editor · Running", KeyContext::Editor, EDITOR_RUNNING),
    GroupSpec::subset("Results · Navigation", KeyContext::Results, RESULTS_NAV),
    GroupSpec::subset("Results · Copy & search", KeyContext::Results, RESULTS_COPY),
    GroupSpec::context("Inspector", KeyContext::Inspector),
    GroupSpec::context("Sessions", KeyContext::Sessions),
    GroupSpec::context("Dialogs", KeyContext::Dialog),
    GroupSpec::context("Command palette", KeyContext::Palette),
    GroupSpec::context("Jobs", KeyContext::Jobs),
];

pub fn build(keymap: &Keymap) -> Vec<HelpGroup>;
pub fn description(context: KeyContext, action: &Action) -> &'static str; // CONTEXT_DESCRIPTIONS override, else action.description()
```

Rules applied by `build`:
1. Every chord bound to an action in a context is joined with spaces, plain keys first and then shorter chords (`display_chord` ordering from T102): `j ↓`, `q ctrl-q`.
2. `SwitchSession(1)` to `SwitchSession(9)` collapse into one row: `ctrl-1–9  switch to session N`.
3. Within a group, entries with identical descriptions merge their keys: `Tab Esc  back to the editor`.
4. Actions in an Editor or Results context that are in no subset fall into `"Editor · Other"` / `"Results · Other"`, so nothing is silently hidden.
5. Empty groups are dropped, for example Jobs when the user unbinds every jobs key.
6. Unbound actions do not appear.

Descriptions come from `Action::description()` (added in T102, one short lowercase phrase per variant) with per-context overrides in `CONTEXT_DESCRIPTIONS`. This removes `describe_action`.

### Mockup at 120x40 (2 columns, abridged)

```
┌ Help — qry 0.2.0 ────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ Global                                                     Results · Navigation                                      │
│ ctrl-q            quit                                     j ↓           next row                                    │
│ tab               focus the next pane                      k ↑           previous row                                │
│ shift-tab         focus the previous pane                  h ←           previous column                             │
│ ctrl-p :          open the command palette                 g G           first / last row                            │
│ ? F1              toggle this help                                                                                   │
│                                                            Results · Copy & search                                   │
│ Tree                                                       y             copy the cell                               │
│ enter             connect / expand                         Y             copy the row as TSV                         │
│ e                 edit the connection                      /             search the results                          │
│ del               delete the connection                    n N           next / previous match                       │
│ ...                                                        ...                                                       │
│ run:  ctrl-enter statement  F8 all  ctrl-enter on a selection runs the selection                                     │
│ results:  / search  y copy cell  Y copy row  enter inspect value                                                     │
└─ k/j scroll  PgUp/PgDn page  Home/G top/bottom  ?/Esc close ──────────────────────────────────────── 1–35 of 66 ─┘
```

### Mockup at 80x24 (1 column, top)

```
┌ Help — qry 0.2.0 ────────────────────────────────────────────────────────────┐
│ Global                                                                       │
│ ctrl-q        quit                                                           │
│ tab           focus the next pane                                            │
│ ...                                                                          │
│ run:  ctrl-enter statement  F8 all                                           │
│ ctrl-enter on a selection runs the selection                                 │
└─ k/j scroll  PgUp/PgDn page  Home/G top/bottom  ?/Esc close ── 1–14 of 120 ─┘
```

### tachy reference

- `~/Projects/tachy/crates/tachy/src/components/help.rs`: `column_count`, `COLUMN_GAP`, `MAX_KEY_SHARE`, `pack`, `draw_line`, footer and bottom border. Port it nearly verbatim.
- `~/Projects/tachy/crates/tachy/src/help_model.rs`: `HelpGroup`, `HELP_KEYS`, `description`, `context_entries`, tab collapse and same-description merge.
- `~/Projects/tachy/crates/tachy/src/app.rs` lines 1077–1086: the open/close dispatch.
- The snapshots `tachy__components__help__tests__help_{120x40,160x48,80x24_top,80x24_bottom}.snap` are the visual reference.

### Proposed modules

```
crates/qry/src/help_model.rs        + help_model_tests.rs
crates/qry/src/ui/overlay/help.rs   + help_tests.rs     (replaces components/help.rs)
```

```rust
pub struct HelpState { pub scroll: u16, pub max_scroll: u16 }
pub fn column_count(width: u16) -> u16;
pub fn pack(groups: &[HelpGroup], columns: u16, col_width: u16) -> Packed; // Vec<Vec<Line>>
pub fn draw(frame: &mut Frame, area: Rect, theme: &Theme, model: &[HelpGroup], footer: &[FooterItem], keys: &str, state: &mut HelpState);
```

### Out of scope

- Search or filter inside the help; tachy has none either.
- Context-sensitive help. The hint bar (T202) covers that.
- The generated `docs/keybindings.md`. T401 reuses `help_model::build` for it.

## Acceptance criteria

- [ ] `components/help.rs` and `describe_action` are removed, and help content comes only from `help_model::build(&keymap)`.
- [ ] `?` (outside the editor) and `F1` (everywhere) open the overlay. `?`, `F1` and `Esc` close it. While it is open, no non-help action reaches any pane, which the harness checks.
- [ ] The overlay fills the whole screen, uses the teal accent, and its title contains `qry` and the crate version.
- [ ] The column count is 1, 2 or 3 at widths 80, 120 and 160, with a column gap of exactly 3 cells.
- [ ] Packing is balanced: no column is more than one group taller than necessary, and a continuation group is never separated from its parent.
- [ ] Group titles are bold teal, keys are bold amber, and descriptions and notes are dim. All of these are checked in the styled snapshots.
- [ ] The key column is the widest key in that column, capped at half the column width. Overflow is truncated with `…`.
- [ ] Every action bound in the default config appears in the help exactly once per context.
- [ ] Remapping a key changes the help text, and unbinding every key of an action removes its entry.
- [ ] Chords are ordered plain-first, then by length. `ctrl-1` to `ctrl-9` collapse into one row, and entries with the same description merge.
- [ ] The footer cheat-sheet is pinned above the bottom border and does not scroll. Its keys come from the keymap.
- [ ] The bottom border shows the help keys from the `Help` context. The ` a–b of n ` indicator appears only when the content scrolls, and its numbers are correct at the top and the bottom.
- [ ] Scrolling is clamped at 0 and `max_scroll`. `G` lands exactly on the last page, and a resize re-clamps the offset.
- [ ] The mode pill keeps showing the pre-help mode while the help is open.
- [ ] The overlay renders without a panic at sizes 80x24 up to 300x100. Below 80x24 the too-small screen wins.

## Tests

`crates/qry/src/help_model_tests.rs`:
- `every_bound_action_appears_once_per_context`: iterate the default keymap and assert each `(context, action)` pair is found.
- `remapped_key_shows_and_unbound_action_disappears`: rebind `RunStatement` to `alt-r` and unbind `CopyRow`.
- `chords_order_plain_then_shorter`: `j ↓`, then `q ctrl-q`.
- `session_switch_collapses_to_one_row`.
- `same_description_entries_merge`.
- `unlisted_actions_fall_into_other_group`: add a test-only binding not in any subset.
- `empty_groups_are_dropped`.
- `context_description_overrides_default`.

`crates/qry/src/ui/overlay/help_tests.rs`:
- `column_count_thresholds`: 99 → 1, 100 → 2, 159 → 2, 160 → 3.
- `pack_balances_columns`: synthetic groups of sizes [10, 3, 3, 3, 10, 1] in 2 columns give a limit of 16.
- `pack_keeps_continuation_with_parent`.
- `key_column_capped_at_half_and_truncated_with_ellipsis`.
- `scroll_clamps_and_g_reaches_last_page`.
- `position_indicator_only_when_scrollable`: at 160x48 expect no indicator, at 80x24 expect `1–` at the top and `of N` with the last line at the bottom.
- `footer_does_not_scroll`: scroll to the bottom and check the footer rows are unchanged.
- `bottom_border_keys_follow_help_keymap`.

Reducer tests, in `crates/qry/src/app/help_tests.rs`:
- `question_mark_opens_and_esc_closes`.
- `f1_opens_from_the_editor_while_question_mark_types`.
- `help_swallows_other_actions`: with help open, `ctrl-enter` sends no query effect.
- `pill_keeps_previous_mode_under_help`.
- `opening_resets_scroll`.

insta snapshots, plain and styled (the styled ones use T002's `styled()` format):
- `help_80x24_top`, `help_80x24_bottom`, `help_120x40`, `help_160x48`.
- `help_120x40_remapped`: shows a user remap.
