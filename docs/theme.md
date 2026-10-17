# Theme

qry has one theme, `dark`, chosen with the config's `theme` key or
`--theme`. Every colour on screen comes from a named helper in
`crates/qry/src/theme.rs`; no view writes a colour of its own (a test,
`tests/no_raw_colors.rs`, fails if one does).

## Palette

| Name | Colour | Used for |
| --- | --- | --- |
| `bg` | `#0e1116` | the screen |
| `bg_alt_row` | `#10141a` | every other grid row |
| `surface` | `#131820` | pane titles, the grid header, the top bar |
| `surface_raised` | `#161c25` | dialogs |
| `status_bg` | `#1a212b` | the status line |
| `selection_bg` | `#1b2533` | the selected row |
| `border` | `#232a34` | dividers |
| `border_strong` | `#3a4452` | the frozen-column divider, dialog borders |
| `fg` | `#d3d9e0` | text |
| `fg_muted` | `#a9b4c2` | secondary text |
| `fg_dim` | `#8a94a3` | hints, NULL, types, row numbers |
| `amber` | `#f2b04a` | focus, the cursor cell, keys in hints, the TX pill |
| `teal` | `#4fc1c9` | SQL keywords, the NORMAL pill, spinners |
| `purple` | `#c7a6ff` | numbers, the palette, the jobs drawer |
| `green` | `#9fd38a` | strings, success |
| `coral` | `#f28b6b` | errors, the `prod` label |

## Terminals

qry reads two variables at startup:

- **`COLORTERM=truecolor`** (or `24bit`): the colours above, exactly.
- **Without it**: each colour is mapped to the nearest of the xterm 256.
- **`NO_COLOR`** set to anything (see [no-color.org](https://no-color.org)):
  no colour at all. Everything that colour marked is shown with bold,
  underline, reverse, dim or italic instead, so the cursor cell, the
  selection, errors and NULLs stay distinguishable.

To see every helper at once in each mode, look at the swatch the theme
tests render: the `theme_swatch_truecolor`, `theme_swatch_ansi256` and
`theme_swatch_nocolor` snapshots under `crates/qry/src/views/snapshots/`.
