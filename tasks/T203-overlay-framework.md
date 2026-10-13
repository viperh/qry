# T203 — Overlay framework and dialog restyle

**Phase:** 2 — Visual system · **Depends on:** T201, T202 · **Blocks:** T204, T206, T307, T308

## Description

### Current state

Every modal implements the `Form` trait (`crates/qry/src/components/form.rs`):

- It draws a `Block::bordered().title(self.title()).title_bottom(footer)`, with an optional right-aligned subtitle.
- Each field is its own bordered box whose border is `Style::new().cyan()` when focused and `dark_gray()` otherwise (`form.rs:226`).
- The popup is sized from the field count and scrolls to the focused field on short terminals.

`Home` owns `modal: Option<Box<dyn Form>>`, picks it from `Mode` (`modal_for`) and keeps the "submit waits for the worker's answer" logic:

- `awaiting: Option<Awaiting>`, `set_pending("Connecting…")`.
- `fail_modal` keeps the typed values.

This behaviour is good and tested (`home.rs` tests `a_failed_connection_keeps_the_modal_and_everything_typed`, `an_answer_meant_for_another_modal_is_ignored`, …). Only the look changes, along with the plumbing needed for overlays in general.

The concrete modals are `ConnForm` (new/edit connection), `ExportForm`, `PasswordForm`, `ImportForm` and `ConnExportForm`. Nothing dims the screen behind a modal, and there is no confirmation dialog, so deleting a connection from the tree happens immediately.

### Target design

A single overlay framework that dialogs (this task), the help overlay (T204), the command palette (T206) and the messages log (T205) all use.

**Frame.** `dialog_frame(frame, theme, rect, title, accent) -> Rect` (returns the inner rect):
1. `Clear` the rect.
2. Fill it with `theme.dialog()` (fg on `surface_raised`).
3. Draw a plain one-cell border in `theme.dialog_border(accent)`.
4. Put ` Title ` in the top border in `theme.dialog_title(accent)`.

Content starts one cell in from the border.

**Backdrop.** `apply_backdrop(buf, area, theme)` restyles every cell already drawn with `theme.backdrop()` (fg_dim + DIM) without clearing it, so the main screen shows through as a dimmed layer.

**Z-order.** It is fixed in the root draw: base regions, then the backdrop (only if an overlay is open), then the overlay (dialog, palette, help or messages), then the toast. Only one overlay is open at a time, and opening one replaces any other except the password prompt, which can sit on top of a waiting connection dialog.

**Accent by purpose:**

| Accent | Dialogs |
|---|---|
| teal (info/data) | New/Edit Connection, Import, Password, Messages |
| amber (act/confirm) | Export results, Export connections, Confirm |
| coral | Destructive confirm (delete connection, forget password, close a session with an open transaction) |
| purple | Palette (T206) |

**Fields.**
- Each field is one row: a fixed-width label column (the longest label + 2, capped at 18) in `dim`, then the value.
- The focused row gets a `› ` marker in amber before the label, and its label switches to `inspector_label_focused` (bold amber).
- The other fields have two spaces in place of the marker.
- There are no per-field boxes.

**Inputs.**
- Text is drawn in fg, and the cursor is a block cell in `cursor_cell` (dark text on amber), only in the focused field.
- Placeholders are shown in `dim`.
- Masked inputs show `•`.
- Long values scroll horizontally so the cursor stays visible, with a leading `…` when scrolled.

**Choice fields.** These render `‹ postgres ›`. The arrows are amber when the field is focused, and `←`/`→`/`Space` cycle the options.

**Checkbox fields.** These render `[x]`/`[ ]`, and `Space` toggles them. They are needed by T308, for example "save password in keychain".

**Footer** (the last inner row):
- Key/label pairs such as `Enter save · Esc cancel · Tab next`, with keys in `hint_key` and labels in `hint`, resolved from the `Dialog` key context (T102).
- While pending, the footer is replaced by a teal braille spinner `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`, advanced every 100 ms on the virtual clock, followed by `Connecting…`.
- An error replaces the footer with coral text, truncated with `…`. The full message also goes to a toast (T205).

**Sizing.** Dialogs are centred with `centered_rect(w, h, area)`. Each dialog declares a preferred width (Connection 72, Export 64, Import 72, Password 50, Confirm between 40 and 72 depending on the message), clamped to `area.width - 4`. The height is fields + 4. When the terminal is shorter than the dialog, the field list scrolls to keep the focused field visible, which preserves today's behaviour (`a_short_terminal_scrolls_to_the_focused_field`).

**Confirm dialog.** A generic `Confirm { title, message, confirm_label, accent, on_confirm: Action }`:
- The message is word-wrapped, followed by two buttons ` Cancel ` and ` Delete `.
- The primary button uses `button_primary` (dark on teal), or a coral variant when the confirm is destructive. The other button uses `button`.
- `←`/`→`/`Tab` move between buttons, `Enter` activates the selected one, `Esc` cancels, and `y`/`n` are shortcuts.
- The default selection is **Cancel** for destructive confirms.

### Mockup at 120x40 (New Connection, focused on Host, waiting)

```
 (dimmed main screen)
              ┌ New Connection ─────────────────────────────────────────────── postgres ┐
              │                                                                          │
              │   Name        prod                                                       │
              │   Driver      ‹ postgres ›                                               │
              │ › Host        db.local█                                                  │
              │   Port        5432                                                       │
              │   User        app                                                        │
              │   Password    ••••••••                                                   │
              │   Save pass   [x] in keychain                                            │
              │   Database    app                                                        │
              │   SSL         ‹ prefer ›                                                 │
              │                                                                          │
              │ ⠹ Connecting…                                                            │
              └──────────────────────────────────────────────────────────────────────────┘
```

### Mockup at 80x24 (destructive confirm)

```
                 ┌ Delete connection ────────────────────────┐
                 │ Delete "prod" and forget its stored        │
                 │ password? This cannot be undone.           │
                 │                                            │
                 │               [ Cancel ]   Delete          │
                 └────────────────────────────────────────────┘
```

### tachy reference

- `~/Projects/tachy/crates/tachy/src/components/dialogs/mod.rs`: `dialog_frame` and `draw_input`. They are verbatim-adaptable:
  ```rust
  let block = Block::bordered()
      .style(theme.dialog())
      .border_style(theme.dialog_border(accent))
      .title(format!(" {title} "))
      .title_style(theme.dialog_title(accent));
  let inner = block.inner(rect);
  frame.render_widget(Clear, rect);
  frame.render_widget(block, rect);
  ```
- `~/Projects/tachy/crates/tachy/src/components/layout.rs`: `centered_rect` and `apply_backdrop`.
- `~/Projects/tachy/crates/tachy/src/app.rs` `draw_frame`: the z-order.
- The tachy Export dialog snapshot shows the `› ` marker and the fixed label column.

### Proposed modules

```
crates/qry/src/ui/overlay/
  mod.rs          Overlay enum, draw order, backdrop
  frame.rs        dialog_frame, centered_rect, apply_backdrop       + frame_tests.rs
  fields.rs       FieldRow rendering: text, masked, choice, checkbox + fields_tests.rs
  spinner.rs      Spinner { frame } advanced on Tick (100 ms)
  confirm.rs      Confirm dialog                                    + confirm_tests.rs
crates/qry/src/components/form.rs   Form trait kept; render() rewritten on top of overlay::*
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent { Info, Act, Danger, Command }   // teal, amber, coral, purple

pub fn dialog_frame(frame: &mut Frame, theme: &Theme, rect: Rect, title: &str, accent: Accent) -> Rect;
pub fn apply_backdrop(buf: &mut Buffer, area: Rect, theme: &Theme);
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect;

pub enum Overlay {
    Dialog(Box<dyn Form>),
    Confirm(Confirm),
    Help,        // T204
    Palette,     // T206
    Messages,    // T205
}

pub trait Form {
    fn title(&self) -> &str;
    fn accent(&self) -> Accent { Accent::Info }
    fn width(&self) -> u16 { 64 }
    // existing: labels, fields, focus, error, pending, submit, …
}
```

`Field` gains a `Checkbox(bool)` variant. `Field::choice` renders the `‹ … ›` form.

Destructive tree actions (`TreeCommand::Delete`) now open a coral `Confirm`. Only its `on_confirm` action performs the delete. The `Awaiting` logic stays where it is, moved into the reducer state (T101) alongside `Overlay`.

### Out of scope

- Content and validation changes to the connection form, such as test-connection or SSH (T308).
- The export dialog's new options (T307).
- The help, palette and messages contents (T204, T206, T205). This task only gives them the frame and backdrop.

## Acceptance criteria

- [x] All five existing modals and `Confirm` render through `dialog_frame`. No modal draws a per-field `Block`, and `form.rs` contains no `Block::bordered()` apart from the one inside `dialog_frame`.
- [x] When an overlay is open, every cell outside it carries the backdrop style. A buffer check confirms this for the corners of the screen.
- [x] With no overlay open, no cell is dimmed.
- [x] Draw order is base, backdrop, overlay, toast. A toast emitted while a dialog is open is visible above the backdrop.
- [x] Each dialog's border and title use its documented accent. The delete-connection confirm is coral and defaults to Cancel.
- [x] Only the focused field shows `› ` and the block cursor. Choice fields show `‹ value ›`, checkboxes show `[x]`/`[ ]`, and masked text shows `•` only.
- [x] The footer keys come from the `Dialog` key context, so a remap shows up in the footer.
- [x] While a submit waits, the footer shows the spinner and pending text, and the spinner frame advances once per 100 ms of virtual time.
- [x] A worker failure shows a coral inline error and keeps every typed value.
- [x] Existing behaviour tests from `home.rs` still pass after being moved into the reducer tests: submit waits, failure keeps values, a late answer for another modal is ignored, and a password prompt is opened by `NeedPassword`.
- [x] Short terminals: at 80x24 the New Connection dialog scrolls so the focused field is always visible.
- [x] Deleting a connection from the tree requires confirmation. `Esc`, `n` and Cancel leave the connection in place, while `y` or Delete removes it and forgets its keychain secret.
- [x] No raw colours are used (`no_raw_colors` passes).

## Tests

`crates/qry/src/ui/overlay/frame_tests.rs`:
- `centered_rect_is_centred_and_clamped`: odd and even sizes, and a dialog wider than the area.
- `dialog_frame_returns_inner_rect_one_cell_in`.
- `backdrop_dims_every_cell_outside_the_overlay`: draw a filled base, apply the backdrop, then assert the modifier on sampled cells and that cell symbols are unchanged.
- `no_backdrop_without_overlay`.

`crates/qry/src/ui/overlay/fields_tests.rs`:
- `only_the_focused_field_has_marker_and_cursor`.
- `long_text_scrolls_to_keep_cursor_visible_with_leading_ellipsis`.
- `masked_field_never_shows_plaintext`: type `hunter2`, then assert the buffer contains no `h`.
- `choice_and_checkbox_render_and_cycle`.

`crates/qry/src/ui/overlay/confirm_tests.rs`:
- `destructive_confirm_defaults_to_cancel`.
- `y_confirms_n_and_esc_cancel`: assert the emitted effects or actions.
- `message_wraps_within_width`.

`crates/qry/src/components/form_tests.rs`, moved from the inline tests and extended:
- Keep the existing tests `typing_and_editing_go_to_the_focused_text_field`, `tab_cycles_fields_and_choices_ignore_typing`, `editing_clears_the_error` and `a_short_terminal_scrolls_to_the_focused_field`.
- `footer_keys_follow_the_dialog_keymap`.
- `pending_spinner_advances_every_100ms`: `AppHarness::advance(250)` should give frame 2.

Reducer tests (T101 harness), in `crates/qry/src/app/overlay_tests.rs`:
- The moved `home.rs` scenarios, unchanged in intent.
- `delete_from_tree_asks_first`, `confirmed_delete_removes_and_forgets_secret` and `toast_draws_above_dialog`.

insta snapshots, plain and styled:
- `dialog_new_connection_120x40`, `dialog_new_connection_80x24_scrolled`, `dialog_new_connection_pending_120x40`, `dialog_new_connection_error_120x40`.
- `dialog_export_120x40`, `dialog_import_120x40`, `dialog_password_80x24`.
- `confirm_delete_80x24`.

## Implementation notes

- **The toast box was ticked by T205**, which draws the toast after the overlays (`toast_draws_above_dialog`). Before it, the draw order is the main screen, then the backdrop (only while an overlay is open), then the overlays (`views::draw_overlays`). There are no toasts yet, so "a toast emitted while a dialog is open is visible" and `toast_draws_above_dialog` wait for T205, which draws its toast after `draw_overlays`. For the same reason the full error text goes only to the dialog footer and the status line for now, not to a toast.
- **Where things live.** The spec's `ui/overlay/` is `views/overlay.rs` (`dialog_frame`, `apply_backdrop`, `SPINNER`). `centered_rect` stayed in `views/layout.rs` (T202), with the spec's argument order `(area, width, height)` swapped to match its other helpers. `views/form.rs` draws every form and `views/confirm.rs` the confirm dialog. `Accent { Info, Act, Danger, Command }` lives in `theme.rs` with `Theme::accent()`, so dialog state can name it; `Theme::button_danger()` (dark on coral) was added and the swatch snapshots re-accepted.
- **`Form` trait.** `hint()` became `footer() -> &[(Action, label)]` (keys looked up in the `Dialog` context with `display_chord`; an unbound action is left out). New `accent()` (Info by default) and `width()` (64 by default): Connection 72, Import 72, Export 64 (amber), Export Connections 64 (amber), Password 50. The popup is `width.min(area - 4)` wide and `fields + 4` tall (fields, a blank row, the footer, the border). `is_typing()` and `act()` tolerate a form with no fields. `Field::Checkbox { checked, caption }` (`[x] caption`, toggled by Space or ←/→) and `Form::checked()` exist for T308; nothing builds one yet, hence an `allow(dead_code)` naming T308.
- **Rows.** `› ` (amber) on the focused row and two spaces elsewhere, then the label padded to the longest label + 2 (capped at 18), in `dim` or `inspector_label_focused`. Text is `field`; the focused input has a block cursor in `cursor_cell` on the character under it (or a space at the end). A long value scrolls with a leading dim `…`. Placeholders are `field_placeholder`. Choices are `‹ value ›` with amber arrows when focused. The subtitle sits right-aligned in the top border. Short terminals keep T101's behaviour: the rows scroll so the focused one shows. At 80x24 the whole New Connection form fits (12 fields + 4 rows), so `dialog_new_connection_80x24_scrolled` shows it focused on its last field rather than scrolled.
- **Spinner.** `Timer::Spinner(n)` and `State::{spinner, spinner_timer}`. A submit that waits restarts it at frame 0 and schedules a 100 ms timer; each one that fires while the dialog still waits moves a frame and schedules the next, and a stale one changes nothing. `AppHarness` delivers spinner timers on its virtual clock but does not record them as effects, so tests that match a submit's effects are not cluttered by pacing. `advance(250)` gives frame 2.
- **Confirm.** `app/dialogs/confirm.rs` replaces T103's `ConfirmForm`: `Confirm { title, message, confirm_label, destructive, on_confirm: OnConfirm, … }` with `OnConfirm::{CloseSession(SessionId), DeleteConnection(id)}` (an id, not an `Action`, because the action needs to know which). It implements `Form` with no fields (so `DialogState::form()` keeps working) and is drawn by `views::confirm`: the message word-wrapped, then `Cancel` and the confirm button, the selected one in brackets and `button_primary` (`button_danger` when destructive). ←/→/Tab move, Enter chooses, Esc cancels, and `y`/`n` are taken in `reducer::keys::unbound` for the `Dialog` context. Destructive confirms start on Cancel and are coral. `Submission::Confirmed(OnConfirm)` and `Submission::Dismissed` replace `Submission::CloseSession`. Closing a session with a running query now uses it, so Enter alone keeps the session (`right enter` or `y` closes).
- **Delete asks first.** Tree `Delete` opens `Confirm::delete_connection` (the message mentions the stored password only for keychain records). Only its confirm runs `tree::delete`, which forgets the keychain secret and saves.
- **Tests.** `views/overlay_tests.rs` (`centered_rect_is_centred_and_clamped`, `dialog_frame_returns_inner_rect_one_cell_in`, `backdrop_dims_every_cell_outside_the_overlay`, `spinner_frames_are_braille`), `app/dialogs/confirm_tests.rs` (`destructive_confirm_defaults_to_cancel`, `moving_between_buttons_picks_what_enter_does`, `message_wraps_within_width`), `form_tests.rs` (the four kept tests updated for one-row fields plus `footer_keys_follow_the_dialog_keymap`, `only_the_focused_field_has_marker_and_cursor`, `long_text_scrolls_to_keep_cursor_visible_with_leading_ellipsis`, `choice_and_checkbox_render_and_cycle`), and `app/overlay_tests.rs` (`an_open_overlay_dims_the_screen_and_none_dims_nothing`, `each_dialog_uses_its_accent`, `pending_spinner_advances_every_100ms`, `a_failure_shows_a_coral_error_and_keeps_every_value`, `delete_from_tree_asks_first` (Esc, n and Enter-on-Cancel), `confirmed_delete_removes_and_forgets_secret`, `masked_field_never_shows_plaintext`, and `dialog_snapshots` with every listed snapshot). The `home.rs` behaviour tests listed already lived in `app/reducer_tests.rs` (T101) and still pass. The tree tests confirm their deletes with `y`.
- **Checks run locally:** `cargo fmt --all`, clippy with `-D warnings`, `QRY_KEYRING=off CI=true cargo test --workspace --all-features --locked` (314 `qry` unit tests, 31 in `qry-core`) and the CI docs command.

