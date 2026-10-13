# T201 — Theme system ported from tachy

**Phase:** 2 — Visual system · **Depends on:** T001 · **Blocks:** T202, T203, T204, T205, T206, T302, T303, T304

## Description

### Current state

Colours are hard-coded in every component:
- `components/editor.rs` `restyle`: `Style::new().cyan()` for the focused border, `dark_gray()` otherwise, `reversed()` for the selection.
- `components/results.rs`: a green focused border and a `black().on_green()` cursor cell; its test `the_selected_cell_is_highlighted` asserts `Color::Green`.
- `components/form.rs`: cyan/dark_gray field borders.
- `components/help.rs`: bold titles.
- `statuspanel.rs`: its own colours.

`config.rs` still carries the template's `Styles(HashMap<Mode, HashMap<String, Style>>)` and `parse_style`/`parse_color`, and `.config/config.json` has an empty `"styles": {"Home": {}}`. None of it is used. There is no `NO_COLOR` support and no 256-colour fallback.

### Target design

Port tachy's theme module (`~/Projects/tachy/crates/tachy/src/theme.rs`) to `crates/qry/src/theme.rs`. Keep its structure and add helpers for qry's surfaces.

**Palette** (`Theme::DARK`, exact values from tachy; 256-colour indices from tachy's own tests):

| field | hex | xterm-256 | role in qry |
|---|---|---|---|
| bg | #0E1116 | 233 | app background, even grid rows, text on accent pills and cursor |
| bg_alt_row | #10141A | 233 | odd striped grid rows (stripes vanish in 256 mode) |
| surface | #131820 | 234 | top bar, grid header, sidebar, pane title rows |
| surface_raised | #161C25 | 234 | dialog, palette and help bodies, buttons |
| status_bg | #1A212B | 235 | status line |
| selection_bg | #1B2533 | 235 | selected grid row, selected tree row, editor selection |
| dialog_title_bg | #1E2632 | 235 | dialog title background |
| track | #1E2530 | 235 | gauge and progress empty part |
| border | #232A34 | 236 | dividers, column `│` |
| border_inner | #2C3542 | 237 | inner borders (input fields) |
| border_strong | #3A4452 | 238 | divider after frozen columns, focused pane divider |
| fg | #D3D9E0 | 253 | main text |
| fg_muted | #A9B4C2 | 249 | secondary text, inspector values |
| fg_dim | #8A94A3 | 246 | labels, types, hints, gutter, NULL, comments, backdrop |
| amber | #F2B04A | 215 | "you are here / act": cursor cell, focus, keys, active tab, EDIT pill |
| amber_bright | #FFD08A | 222 | reserved |
| amber_tint | #3A2F17 | 235 | search-match background |
| teal | #4FC1C9 | 74 | data and info: NORMAL pill, primary button, help accent, spinner, SQL keywords and operators, open-session dot |
| purple | #C7A6FF | 183 | command and jobs: COMMAND pill, palette border, jobs count, SQL numbers |
| purple_tint | #2A2340 | 236 | selected palette item |
| green | #9FD38A | 150 | success `✓`, SQL strings |
| coral | #F28B6B | 209 | errors and warnings (one colour for both), failed session `✗`, SQL error underline |

Keep tachy's doc comment on accent meanings at the top of the file. Every new helper must pick its accent by that rule.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme { /* the 22 colour fields above */, pub monochrome: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport { TrueColor, Ansi256, NoColor }

impl ColorSupport {
    /// NO_COLOR non-empty → NoColor; COLORTERM=truecolor|24bit → TrueColor; else Ansi256.
    pub fn detect() -> Self;
    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>) -> Self; // testable core
}

impl Theme {
    pub const NAMES: &[&str] = &["dark"];
    pub const DARK: Theme = /* exact values */;
    pub fn by_name(name: &str) -> Option<Theme>;
    /// Rgb to nearest xterm-256 (indices 16–255, redmean distance); nudge selection_bg/status_bg
    /// by ±1 when they collapse onto bg; NoColor → every colour Reset and monochrome = true.
    pub fn adapt(self, support: ColorSupport) -> Theme;
    fn pick(&self, colored: Style, mono: Modifier) -> Style;
}
```

**Helpers.** Drawing code calls only these, each with a monochrome fallback through `pick`:
- **Base:** `base`, `surface`, `muted`, `dim`, `border`, `border_strong`, `spinner`.
- **Top bar (T202):** `app_name` (amber bold), `tab`, `tab_active` (fg on bg, amber underline), `tab_running` (teal), `mode_pill(Pill)` (NORMAL teal, EDIT amber, COMMAND purple; fg is bg, bold).
- **Panes:**
  - `pane_title` (fg_dim on surface) and `pane_title_focused` (amber bold on surface). Panes are separated by background and `│` dividers, not boxes; focus is shown by the title.
  - `divider` and `divider_focused`.
- **Tree (T301):**
  - `tree_connection`, `tree_connection_selected`, `tree_object` (fg_muted), `tree_kind` (fg_dim: `table`, `view`).
  - `session_dot(SessionStatus)`: `●` teal, `◌` dim, `✗` coral.
  - `driver_badge(Driver)`: a short dim label `pg`, `my`, `ma`, `ora`, `lite`. No brand colours.
- **Grid (T303):**
  - `header`, `header_type`, `header_active`, `sort_indicator`.
  - `gutter`, `gutter_selected`.
  - `row(i)`, `row_selected`, `cursor_cell` (bg on amber bold / REVERSED|BOLD).
  - `null_value` (fg_dim italic), `match_highlight`, `column_divider`, `frozen_divider`, `overflow_hint`.
- **Inspector (T304):** `inspector_label`, `inspector_label_focused`, `record_field`, `record_value`, `value_changed`.
- **Status line:** `status_line`, `status_session` (teal bold), `status_dim`, `status_warning` (coral), `status_jobs` (purple bold), `gauge(fill)`.
- **Hints:** `hint` (dim) and `hint_key` (amber bold).
- **SQL highlighting (T302):**
  - `sql_keyword` and `sql_operator` (teal), `sql_string` (green), `sql_number` (purple).
  - `sql_comment` (fg_dim italic), `sql_identifier` and `sql_punct` (fg), `sql_quoted_ident` (fg_muted).
  - `sql_error` (coral, coral underline, UNDERLINED).
  - `editor_cursor_line`, `editor_selection`, `line_number`, `line_number_current`.
- **Overlays (T203–T206):**
  - `dialog`, `dialog_border(accent)`, `dialog_title(accent)`.
  - `field`, `field_focused` (the `› ` marker in amber), `field_placeholder`.
  - `button`, `button_primary` (bg on teal bold).
  - `palette_border`, `palette_item_selected`, `palette_match`.
  - `backdrop`: applied to every cell behind a modal without clearing it.
  - `toast(level)`.
- **Jobs (T305):** `job(kind)` and `job_state(state) -> (Style, &'static str)` with glyphs `✓ ✗ – … ▸`.

**Selection and wiring:**
- Config key `"theme": "dark"` at the top level of `.config/config.json`, plus a `--theme <name>` CLI flag (`cli.rs`, validated against `Theme::NAMES`; the CLI wins).
- An unknown name is a startup error listing the valid names.
- Resolve once at startup: `Theme::by_name(name).adapt(ColorSupport::detect())`. Store the result in `State` (T101); views get `&Theme`.
- `qry --version` prints the detected colour support.

**Removals:**
- `config.rs` `Styles`, `parse_style`, `parse_color` and their tests.
- The `"styles"` config section.
- An existing user config that still has `"styles"` loads, with a one-line warning that it is ignored.

**Guard test.** Copy tachy `tests/no_raw_colors.rs`. It scans `crates/qry/src/**/*.rs` and fails on `Color::Rgb`, `Color::Indexed` or any named `Color::` variant, and on the ratatui `Stylize` colour shorthands (`.cyan()`, `.green()`, `.on_green()`, `.dark_gray()`, `.red()`, …), outside `theme.rs`.

### Out of scope

- Light theme and user-defined palettes. `NAMES` stays `["dark"]`, but the struct and `by_name` leave room for both.
- Layout changes (T202).

## Acceptance criteria

- [ ] `crates/qry/src/theme.rs` defines `Theme::DARK` with exactly the 22 hex values above.
- [ ] `ColorSupport::from_env` implements the NO_COLOR > COLORTERM > Ansi256 precedence; `NO_COLOR=""` (empty) does not disable colour.
- [ ] `adapt(Ansi256)` produces only `Color::Indexed(16..=255)` and keeps selection_bg/status_bg distinct from bg.
- [ ] `adapt(NoColor)` produces only `Color::Reset` and every helper returns a modifier-only style.
- [ ] Every existing component (editor, results, tree, forms, help, status) draws through helpers; `tests/no_raw_colors.rs` passes and fails when a raw colour is added (proved by a fixture).
- [ ] `"theme"` config key and `--theme` flag work; an unknown name errors with the list of names.
- [ ] `Styles`, `parse_style` and the `styles` config section are gone; an old config with `styles` still loads, with a warning.
- [ ] `qry --version` shows the colour support.
- [ ] `results.rs` test `the_selected_cell_is_highlighted` asserts `theme.cursor_cell()` instead of `Color::Green`.

## Tests

`crates/qry/src/theme_tests.rs`:
- `dark_palette_exact_values`: a table test for all 22 fields against the hex values above.
- `color_support_precedence` (table): `(Some("1"), Some("truecolor"))` gives NoColor; `(Some(""), Some("truecolor"))` gives TrueColor; `(None, Some("24bit"))` gives TrueColor; `(None, None)` gives Ansi256; `(None, Some("xterm"))` gives Ansi256.
- `ansi256_indices_match_tachy`: `adapt(Ansi256)` maps bg→233, surface→234, status_bg→235, border→236, fg→253, fg_dim→246, amber→215, teal→74, purple→183, green→150, coral→209.
- `ansi256_only_indexed_16_to_255`: every field is `Indexed(n)` with `n >= 16`.
- `ansi256_keeps_selection_distinct_from_bg`.
- `nocolor_is_reset_and_monochrome`: every field is `Reset`, and `cursor_cell()` has REVERSED|BOLD and no fg or bg.
- `every_helper_has_mono_fallback`: call every helper on the NoColor theme and assert fg and bg are `None` or `Reset` (a list of helper closures).
- `mode_pill_accents`: NORMAL is teal, EDIT amber, COMMAND purple, with fg = bg.
- `job_state_glyphs`.

`crates/qry/tests/no_raw_colors.rs`:
- `no_raw_colors_outside_theme`: the source scan described above.
- `scanner_catches_raw_color_fixture`: runs the scanner on `tests/fixtures/raw_color.rs.txt` and asserts it reports line numbers.

`crates/qry/src/cli_tests.rs` and `config_tests.rs`:
- `unknown_theme_is_an_error_listing_names`.
- `cli_theme_overrides_config`.
- `legacy_styles_section_warns_and_loads`.

Snapshots, styled (sverb `styled()` format: text plus `--- styles ---` runs) in `crates/qry/src/views/snapshots/`:
- `theme_swatch_truecolor_80x24`, `theme_swatch_ansi256_80x24` and `theme_swatch_nocolor_80x24`: a debug view (test-only) that renders one line per helper with a sample. Changes to the palette show up in review.
