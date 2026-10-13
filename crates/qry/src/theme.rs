//! qry colour theme, ported from tachy (`crates/tachy/src/theme.rs`).
//!
//! All colours are 24-bit (`Color::Rgb`). Drawing code never uses raw
//! colours: it calls the style helpers below (`tests/no_raw_colors.rs`
//! enforces this). Accents keep meaning: amber = "you are here / act",
//! teal = "file / data info", purple = "command / jobs", coral = "problem".
//! Every helper picks its accent by that rule.
//!
//! The theme is resolved once at startup ([`resolve`]) and handed to drawing
//! code as `&Theme`, already [adapted](Theme::adapt) to what the terminal can
//! display, so no per-draw work happens.

use std::fmt;

use color_eyre::eyre::{Result, eyre};
use ratatui::style::{Color, Modifier, Style};

use crate::connections::Driver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    // Backgrounds and surfaces
    pub bg: Color,
    pub bg_alt_row: Color,
    pub surface: Color,
    pub surface_raised: Color,
    pub status_bg: Color,
    pub selection_bg: Color,
    pub dialog_title_bg: Color,
    pub track: Color,

    // Borders
    pub border: Color,
    pub border_inner: Color,
    pub border_strong: Color,

    // Text
    pub fg: Color,
    pub fg_muted: Color,
    pub fg_dim: Color,

    // Accents
    pub amber: Color,
    pub amber_bright: Color,
    pub amber_tint: Color,
    pub teal: Color,
    pub purple: Color,
    pub purple_tint: Color,
    pub green: Color,
    pub coral: Color,

    /// `NO_COLOR`: every helper returns a modifier-only style.
    pub monochrome: bool,
}

/// What the terminal can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    /// 24-bit colour (`COLORTERM=truecolor` / `24bit`).
    TrueColor,
    /// The xterm-256 palette.
    Ansi256,
    /// `NO_COLOR` is set: modifiers only.
    NoColor,
}

/// The terminal environment the theme reads: `NO_COLOR` and `COLORTERM`.
/// [`ColorSupport::detect`] reads it from the process; tests build one, and
/// the app harness takes one with `with_theme_env`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeEnv {
    /// `NO_COLOR` is set to anything non-empty.
    pub no_color: bool,
    /// The value of `COLORTERM`, such as `truecolor`.
    pub colorterm: Option<String>,
}

/// What a dialog is for, which picks its border and title colour: teal for
/// information and data, amber to act or confirm, coral for what cannot be
/// undone, purple for commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Info,
    Act,
    Danger,
    Command,
}

/// The mode pill in the top bar (T202).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pill {
    Normal,
    Edit,
    Command,
}

impl Pill {
    /// What the pill says.
    pub fn label(self) -> &'static str {
        match self {
            Pill::Normal => "NORMAL",
            Pill::Edit => "EDIT",
            Pill::Command => "COMMAND",
        }
    }
}

/// Where an open connection stands, for its dot in the tree and the status
/// line. The app state re-exports it as `app::state::SessionStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Connecting,
    Open { label: String },
    Failed(String),
    Closed,
}

/// How loud a toast is (T205).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// What a job does (T305, which moves this next to its registry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Query,
    FetchMore,
    Export,
    Schema,
}

/// Where a job is (T305, which moves this next to its registry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running { rows: u64 },
    Done { rows: u64 },
    Failed(String),
    Cancelled,
}

impl Theme {
    /// Names accepted by [`Theme::by_name`] (and `--theme`). Only `dark` exists for now.
    pub const NAMES: &[&str] = &["dark"];

    pub const DARK: Theme = Theme {
        bg: Color::Rgb(0x0E, 0x11, 0x16),
        bg_alt_row: Color::Rgb(0x10, 0x14, 0x1A),
        surface: Color::Rgb(0x13, 0x18, 0x20),
        surface_raised: Color::Rgb(0x16, 0x1C, 0x25),
        status_bg: Color::Rgb(0x1A, 0x21, 0x2B),
        selection_bg: Color::Rgb(0x1B, 0x25, 0x33),
        dialog_title_bg: Color::Rgb(0x1E, 0x26, 0x32),
        track: Color::Rgb(0x1E, 0x25, 0x30),

        border: Color::Rgb(0x23, 0x2A, 0x34),
        border_inner: Color::Rgb(0x2C, 0x35, 0x42),
        border_strong: Color::Rgb(0x3A, 0x44, 0x52),

        fg: Color::Rgb(0xD3, 0xD9, 0xE0),
        fg_muted: Color::Rgb(0xA9, 0xB4, 0xC2),
        fg_dim: Color::Rgb(0x8A, 0x94, 0xA3),

        amber: Color::Rgb(0xF2, 0xB0, 0x4A),
        amber_bright: Color::Rgb(0xFF, 0xD0, 0x8A),
        amber_tint: Color::Rgb(0x3A, 0x2F, 0x17),
        teal: Color::Rgb(0x4F, 0xC1, 0xC9),
        purple: Color::Rgb(0xC7, 0xA6, 0xFF),
        purple_tint: Color::Rgb(0x2A, 0x23, 0x40),
        green: Color::Rgb(0x9F, 0xD3, 0x8A),
        coral: Color::Rgb(0xF2, 0x8B, 0x6B),

        monochrome: false,
    };

    /// The theme called `name` (`--theme`, the `theme` config key), if it exists.
    pub fn by_name(name: &str) -> Option<Theme> {
        match name {
            "dark" => Some(Self::DARK),
            _ => None,
        }
    }

    /// This theme mapped to what the terminal can display. Called once at
    /// startup; drawing code only ever sees an adapted theme, so no per-draw
    /// work happens.
    ///
    /// - `TrueColor`: unchanged.
    /// - `Ansi256`: every `Color::Rgb` field becomes the nearest xterm-256
    ///   index ([`nearest_xterm256`]). The near-black surfaces collapse onto
    ///   one or two grey-ramp steps; `selection_bg` and `status_bg` are kept
    ///   distinct from `bg` ([`Theme::keep_distinct`]). `bg` and `bg_alt_row`
    ///   map to the same index, so **striped rows disappear in 256-colour
    ///   mode**; that is accepted.
    /// - `NoColor`: every colour becomes `Color::Reset` and `monochrome` is
    ///   set, so each helper returns a modifier-only style (bold, reverse,
    ///   underline, dim, italic).
    pub fn adapt(self, support: ColorSupport) -> Theme {
        match support {
            ColorSupport::TrueColor => self,
            ColorSupport::Ansi256 => {
                let mut t = self.map_colors(|c| match c {
                    Color::Rgb(r, g, b) => Color::Indexed(nearest_xterm256(r, g, b)),
                    other => other,
                });
                t.keep_distinct(&self);
                t
            }
            ColorSupport::NoColor => Theme {
                monochrome: true,
                ..self.map_colors(|_| Color::Reset)
            },
        }
    }

    /// Applies `f` to every colour field.
    fn map_colors(self, f: impl Fn(Color) -> Color) -> Theme {
        Theme {
            bg: f(self.bg),
            bg_alt_row: f(self.bg_alt_row),
            surface: f(self.surface),
            surface_raised: f(self.surface_raised),
            status_bg: f(self.status_bg),
            selection_bg: f(self.selection_bg),
            dialog_title_bg: f(self.dialog_title_bg),
            track: f(self.track),
            border: f(self.border),
            border_inner: f(self.border_inner),
            border_strong: f(self.border_strong),
            fg: f(self.fg),
            fg_muted: f(self.fg_muted),
            fg_dim: f(self.fg_dim),
            amber: f(self.amber),
            amber_bright: f(self.amber_bright),
            amber_tint: f(self.amber_tint),
            teal: f(self.teal),
            purple: f(self.purple),
            purple_tint: f(self.purple_tint),
            green: f(self.green),
            coral: f(self.coral),
            monochrome: self.monochrome,
        }
    }

    /// After 256-colour mapping: the selected row and the status line must
    /// stay visible against `bg`. When one of them collapsed onto `bg`'s
    /// index, it moves one step (index ± 1): up when it is lighter than `bg`
    /// in `original` (the usual case: the next grey-ramp step), down
    /// otherwise. `bg` itself never moves, since it is shared by both pairs.
    fn keep_distinct(&mut self, original: &Theme) {
        let Color::Indexed(bg) = self.bg else {
            return;
        };
        for (mapped, orig) in [
            (&mut self.selection_bg, original.selection_bg),
            (&mut self.status_bg, original.status_bg),
        ] {
            if *mapped != Color::Indexed(bg) {
                continue;
            }
            let up = luma(orig) >= luma(original.bg);
            let nudged = match (up, bg) {
                (true, 255) | (false, 16) => bg ^ 1, // 254 or 17: the only step left
                (true, _) => bg + 1,
                (false, _) => bg - 1,
            };
            *mapped = Color::Indexed(nudged);
        }
    }

    /// `colored` normally; `mono` modifiers only under `NO_COLOR`.
    fn pick(&self, colored: Style, mono: Modifier) -> Style {
        if self.monochrome {
            Style::new().add_modifier(mono)
        } else {
            colored
        }
    }

    // ---- base ----------------------------------------------------------

    /// Default text on the app background (`bg`, `fg`).
    pub fn base(&self) -> Style {
        self.pick(Style::new().fg(self.fg).bg(self.bg), Modifier::empty())
    }

    /// Default text on `surface`: top bar, grid header, sidebar.
    pub fn surface(&self) -> Style {
        self.pick(Style::new().fg(self.fg).bg(self.surface), Modifier::empty())
    }

    /// Secondary text (`fg_muted`).
    pub fn muted(&self) -> Style {
        self.pick(Style::new().fg(self.fg_muted), Modifier::empty())
    }

    /// Labels, types, hints (`fg_dim`).
    pub fn dim(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// Default dividers and borders (`border`).
    pub fn border(&self) -> Style {
        self.pick(Style::new().fg(self.border), Modifier::empty())
    }

    /// A stronger divider (`border_strong`).
    pub fn border_strong(&self) -> Style {
        self.pick(Style::new().fg(self.border_strong), Modifier::BOLD)
    }

    /// Border of the focused panel while panes still have boxes (until
    /// T202 replaces them with titles and dividers): amber.
    pub fn focused_border(&self) -> Style {
        self.pick(Style::new().fg(self.amber), Modifier::BOLD)
    }

    /// Spinner and "working…" text while waiting for an answer: teal.
    pub fn spinner(&self) -> Style {
        self.pick(Style::new().fg(self.teal), Modifier::empty())
    }

    /// Something that worked: green.
    pub fn value_ok(&self) -> Style {
        self.pick(Style::new().fg(self.green), Modifier::empty())
    }

    /// Something that failed: coral.
    pub fn value_error(&self) -> Style {
        self.pick(Style::new().fg(self.coral), Modifier::BOLD)
    }

    // ---- top bar -------------------------------------------------------

    /// `qry` in the top bar: bold amber.
    pub fn app_name(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// Inactive session tab: dim on `surface`.
    pub fn tab(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim).bg(self.surface), Modifier::DIM)
    }

    /// Active session tab: default text on the app background, amber underline.
    pub fn tab_active(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.fg)
                .bg(self.bg)
                .underline_color(self.amber)
                .add_modifier(Modifier::UNDERLINED),
            Modifier::UNDERLINED | Modifier::BOLD,
        )
    }

    /// A tab whose session is running a statement: teal on `surface`.
    pub fn tab_running(&self) -> Style {
        self.pick(
            Style::new().fg(self.teal).bg(self.surface),
            Modifier::ITALIC,
        )
    }

    /// The mode pill in the top-right corner: dark bold text on the mode's
    /// accent. NORMAL is teal, EDIT amber, COMMAND purple.
    pub fn mode_pill(&self, pill: Pill) -> Style {
        let bg = match pill {
            Pill::Normal => self.teal,
            Pill::Edit => self.amber,
            Pill::Command => self.purple,
        };
        self.pick(
            Style::new().fg(self.bg).bg(bg).add_modifier(Modifier::BOLD),
            Modifier::REVERSED | Modifier::BOLD,
        )
    }

    // ---- panes ---------------------------------------------------------

    /// Title row of a pane without focus: dim on `surface`. Panes are told
    /// apart by background and `│` dividers, and focus by the title.
    pub fn pane_title(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim).bg(self.surface), Modifier::DIM)
    }

    /// Title row of the focused pane: bold amber on `surface`.
    pub fn pane_title_focused(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.amber)
                .bg(self.surface)
                .add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// The `│` divider between panes: `border`.
    pub fn divider(&self) -> Style {
        self.pick(Style::new().fg(self.border), Modifier::empty())
    }

    /// The divider next to the focused pane: `border_strong`.
    pub fn divider_focused(&self) -> Style {
        self.pick(Style::new().fg(self.border_strong), Modifier::BOLD)
    }

    // ---- tree ----------------------------------------------------------

    /// A connection in the tree: default text.
    pub fn tree_connection(&self) -> Style {
        self.pick(Style::new().fg(self.fg), Modifier::empty())
    }

    /// The selected row of the tree: bold amber on `selection_bg`.
    pub fn tree_connection_selected(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.amber)
                .bg(self.selection_bg)
                .add_modifier(Modifier::BOLD),
            Modifier::REVERSED | Modifier::BOLD,
        )
    }

    /// A table, view or other object under a connection: muted.
    pub fn tree_object(&self) -> Style {
        self.pick(Style::new().fg(self.fg_muted), Modifier::empty())
    }

    /// The kind next to an object (`table`, `view`): dim.
    pub fn tree_kind(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// Style and glyph of a session's state: `●` teal while open, `◌` dim
    /// while connecting or closed, `✗` coral when it failed. The glyphs
    /// differ, so the state reads without colour too.
    pub fn session_dot(&self, status: &SessionStatus) -> (Style, &'static str) {
        match status {
            SessionStatus::Open { .. } => (self.spinner(), "●"),
            SessionStatus::Connecting | SessionStatus::Closed => (self.dim(), "◌"),
            SessionStatus::Failed(_) => (self.value_error(), "✗"),
        }
    }

    /// A short dim label for the driver. No brand colours.
    pub fn driver_badge(&self, driver: Driver) -> (Style, &'static str) {
        let label = match driver {
            Driver::Postgres => "pg",
            Driver::Mysql => "my",
            Driver::Mariadb => "ma",
            Driver::Oracle => "ora",
            Driver::Sqlite => "lite",
        };
        (self.dim(), label)
    }

    // ---- grid ----------------------------------------------------------

    /// Column name in the grid header: bold, on `surface`.
    pub fn header(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.fg)
                .bg(self.surface)
                .add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// Type label under the column name: dim, on `surface`.
    pub fn header_type(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim).bg(self.surface), Modifier::DIM)
    }

    /// Header of the column the cursor is in: amber.
    pub fn header_active(&self) -> Style {
        self.pick(
            self.header().fg(self.amber),
            Modifier::BOLD | Modifier::UNDERLINED,
        )
    }

    /// `▲` / `▼` after the name of a sorted column: bold amber.
    pub fn sort_indicator(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// `→ N more cols` in the last header cell: dim on `surface`.
    pub fn overflow_hint(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim).bg(self.surface), Modifier::DIM)
    }

    /// Row numbers in the gutter: dim.
    pub fn gutter(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// Row number of the selected row: amber.
    pub fn gutter_selected(&self) -> Style {
        self.pick(Style::new().fg(self.amber), Modifier::BOLD)
    }

    /// Divider between columns and next to the gutter: `border`.
    pub fn column_divider(&self) -> Style {
        self.pick(Style::new().fg(self.border), Modifier::empty())
    }

    /// Divider after the frozen columns: `border_strong`.
    pub fn frozen_divider(&self) -> Style {
        self.pick(Style::new().fg(self.border_strong), Modifier::BOLD)
    }

    /// Striped rows, alternating `bg` / `bg_alt_row`: pass the row's index in the view.
    pub fn row(&self, index: usize) -> Style {
        let bg = if index % 2 == 1 {
            self.bg_alt_row
        } else {
            self.bg
        };
        self.pick(Style::new().fg(self.fg).bg(bg), Modifier::empty())
    }

    /// The selected row: `selection_bg`.
    pub fn row_selected(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg).bg(self.selection_bg),
            Modifier::BOLD,
        )
    }

    /// The cursor cell: dark bold text on amber.
    pub fn cursor_cell(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.bg)
                .bg(self.amber)
                .add_modifier(Modifier::BOLD),
            Modifier::REVERSED | Modifier::BOLD,
        )
    }

    /// `NULL` in a cell: dim italic, so it never reads as the text "NULL".
    pub fn null_value(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg_dim).add_modifier(Modifier::ITALIC),
            Modifier::DIM | Modifier::ITALIC,
        )
    }

    /// A cell (or substring) matching the active search: `amber_tint` background.
    pub fn match_highlight(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).bg(self.amber_tint),
            Modifier::UNDERLINED,
        )
    }

    // ---- inspector -----------------------------------------------------

    /// Section label (`COLUMN`, `RECORD`): dim; the caller uppercases.
    pub fn inspector_label(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// Section label of the focused inspector section: bold amber.
    pub fn inspector_label_focused(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// Field name in the RECORD section: dim.
    pub fn record_field(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// Field value in the RECORD section: muted.
    pub fn record_value(&self) -> Style {
        self.pick(Style::new().fg(self.fg_muted), Modifier::empty())
    }

    /// A value the user changed: amber.
    pub fn value_changed(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    // ---- status + hints -----------------------------------------------

    /// The status line: default text on `status_bg`.
    pub fn status_line(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg).bg(self.status_bg),
            Modifier::empty(),
        )
    }

    /// The session name in the status line: bold teal on `status_bg`.
    pub fn status_session(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.teal)
                .bg(self.status_bg)
                .add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// Number of running jobs in the status line: bold purple on `status_bg`.
    pub fn status_jobs(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.purple)
                .bg(self.status_bg)
                .add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// Errors and warnings in the status line (one colour for both): coral
    /// on `status_bg`.
    pub fn status_warning(&self) -> Style {
        self.pick(
            Style::new().fg(self.coral).bg(self.status_bg),
            Modifier::BOLD,
        )
    }

    /// A success message in the status line: green on `status_bg`.
    pub fn status_ok(&self) -> Style {
        self.pick(
            Style::new().fg(self.green).bg(self.status_bg),
            Modifier::empty(),
        )
    }

    /// Secondary status text: dim on `status_bg`.
    pub fn status_dim(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg_dim).bg(self.status_bg),
            Modifier::DIM,
        )
    }

    /// Action names in the key-hint line: dim.
    pub fn hint(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// Keys in the key-hint line (and the help): bold amber.
    pub fn hint_key(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// A toast: errors and warnings coral, info and success in the normal
    /// text colour, on the app background. Under `NO_COLOR` errors and
    /// warnings are bold.
    pub fn toast(&self, level: ToastLevel) -> Style {
        let (fg, mono) = match level {
            ToastLevel::Error | ToastLevel::Warning => (self.coral, Modifier::BOLD),
            ToastLevel::Info | ToastLevel::Success => (self.fg, Modifier::empty()),
        };
        self.pick(Style::new().fg(fg).bg(self.bg), mono)
    }

    // ---- progress / gauges --------------------------------------------

    /// For `Gauge` / `LineGauge`: fg = filled part, bg = track. Gauges are
    /// drawn with `█` (fill) and `░` (track) glyphs, so progress stays
    /// visible under `NO_COLOR`, where this style has no colour.
    pub fn gauge(&self, fill: Color) -> Style {
        self.pick(Style::new().fg(fill).bg(self.track), Modifier::empty())
    }

    // ---- SQL -----------------------------------------------------------

    /// SQL keywords (`select`, `from`, …): teal.
    pub fn sql_keyword(&self) -> Style {
        self.pick(Style::new().fg(self.teal), Modifier::BOLD)
    }

    /// SQL operators (`=`, `<>`, `||`, …): teal, like keywords.
    pub fn sql_operator(&self) -> Style {
        self.pick(Style::new().fg(self.teal), Modifier::empty())
    }

    /// String literals: green.
    pub fn sql_string(&self) -> Style {
        self.pick(Style::new().fg(self.green), Modifier::empty())
    }

    /// Number literals: purple.
    pub fn sql_number(&self) -> Style {
        self.pick(Style::new().fg(self.purple), Modifier::empty())
    }

    /// Comments: dim italic.
    pub fn sql_comment(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg_dim).add_modifier(Modifier::ITALIC),
            Modifier::DIM | Modifier::ITALIC,
        )
    }

    /// Bare identifiers: default text.
    pub fn sql_identifier(&self) -> Style {
        self.pick(Style::new().fg(self.fg), Modifier::empty())
    }

    /// Punctuation (`,`, `;`, parentheses): default text.
    pub fn sql_punct(&self) -> Style {
        self.pick(Style::new().fg(self.fg), Modifier::empty())
    }

    /// Quoted identifiers (`"Name"`, `` `name` ``): muted.
    pub fn sql_quoted_ident(&self) -> Style {
        self.pick(Style::new().fg(self.fg_muted), Modifier::empty())
    }

    /// The span of a SQL error: coral, with a coral underline.
    pub fn sql_error(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.coral)
                .underline_color(self.coral)
                .add_modifier(Modifier::UNDERLINED),
            Modifier::BOLD | Modifier::UNDERLINED,
        )
    }

    /// The editor line the cursor is on: `surface` background.
    pub fn editor_cursor_line(&self) -> Style {
        self.pick(Style::new().bg(self.surface), Modifier::empty())
    }

    /// The editor's text cursor: dark text on amber.
    pub fn editor_cursor(&self) -> Style {
        self.pick(Style::new().fg(self.bg).bg(self.amber), Modifier::REVERSED)
    }

    /// Selected text in the editor: `selection_bg`.
    pub fn editor_selection(&self) -> Style {
        self.pick(Style::new().bg(self.selection_bg), Modifier::REVERSED)
    }

    /// Line numbers in the editor: dim.
    pub fn line_number(&self) -> Style {
        self.pick(Style::new().fg(self.fg_dim), Modifier::DIM)
    }

    /// The number of the line the cursor is on: amber.
    pub fn line_number_current(&self) -> Style {
        self.pick(Style::new().fg(self.amber), Modifier::BOLD)
    }

    // ---- overlays ------------------------------------------------------

    /// Dialog, palette and help body: `surface_raised`.
    pub fn dialog(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg).bg(self.surface_raised),
            Modifier::empty(),
        )
    }

    /// The colour of a dialog's accent.
    pub fn accent(&self, accent: Accent) -> Color {
        match accent {
            Accent::Info => self.teal,
            Accent::Act => self.amber,
            Accent::Danger => self.coral,
            Accent::Command => self.purple,
        }
    }

    /// 1-cell dialog border in the dialog's accent colour.
    pub fn dialog_border(&self, accent: Color) -> Style {
        self.pick(
            Style::new().fg(accent).bg(self.surface_raised),
            Modifier::empty(),
        )
    }

    /// Dialog title bar: bold accent on `dialog_title_bg`.
    pub fn dialog_title(&self, accent: Color) -> Style {
        self.pick(
            Style::new()
                .fg(accent)
                .bg(self.dialog_title_bg)
                .add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// A heading inside a dialog (a help group title): bold accent on
    /// `surface_raised`.
    pub fn dialog_heading(&self, accent: Color) -> Style {
        self.pick(
            Style::new()
                .fg(accent)
                .bg(self.surface_raised)
                .add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// A form field's label and value: default text.
    pub fn field(&self) -> Style {
        self.pick(Style::new().fg(self.fg), Modifier::empty())
    }

    /// The focused field's label, its `› ` marker and a focused choice: bold amber.
    pub fn field_focused(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// What an empty field shows until something is typed: dim italic.
    pub fn field_placeholder(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg_dim).add_modifier(Modifier::ITALIC),
            Modifier::DIM | Modifier::ITALIC,
        )
    }

    /// The box around a field: `border_inner`.
    pub fn field_border(&self) -> Style {
        self.pick(Style::new().fg(self.border_inner), Modifier::empty())
    }

    /// The box around the focused field: amber.
    pub fn field_border_focused(&self) -> Style {
        self.pick(Style::new().fg(self.amber), Modifier::BOLD)
    }

    /// A dialog button: `surface_raised`.
    pub fn button(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg).bg(self.surface_raised),
            Modifier::empty(),
        )
    }

    /// The primary dialog button: dark bold text on teal.
    pub fn button_primary(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.bg)
                .bg(self.teal)
                .add_modifier(Modifier::BOLD),
            Modifier::REVERSED | Modifier::BOLD,
        )
    }

    /// The primary button of a destructive confirm: dark bold text on coral.
    pub fn button_danger(&self) -> Style {
        self.pick(
            Style::new()
                .fg(self.bg)
                .bg(self.coral)
                .add_modifier(Modifier::BOLD),
            Modifier::REVERSED | Modifier::BOLD,
        )
    }

    /// Command palette border: purple on `surface_raised`.
    pub fn palette_border(&self) -> Style {
        self.pick(
            Style::new().fg(self.purple).bg(self.surface_raised),
            Modifier::empty(),
        )
    }

    /// Selected palette item: purple on `purple_tint`.
    pub fn palette_item_selected(&self) -> Style {
        self.pick(
            Style::new().fg(self.purple).bg(self.purple_tint),
            Modifier::REVERSED,
        )
    }

    /// Characters matched by the palette's fuzzy search: bold amber.
    pub fn palette_match(&self) -> Style {
        self.pick(
            Style::new().fg(self.amber).add_modifier(Modifier::BOLD),
            Modifier::BOLD,
        )
    }

    /// Content behind a modal, applied to every cell without clearing it.
    /// Terminals have no alpha, so the dimmed layer is approximated with DIM.
    pub fn backdrop(&self) -> Style {
        self.pick(
            Style::new().fg(self.fg_dim).add_modifier(Modifier::DIM),
            Modifier::DIM,
        )
    }

    // ---- jobs ----------------------------------------------------------

    /// A job's kind colour: reading data (Query, FetchMore, Schema) is
    /// teal, writing a file (Export) amber, as in tachy.
    pub fn job(&self, kind: JobKind) -> Style {
        let fg = match kind {
            JobKind::Query | JobKind::FetchMore | JobKind::Schema => self.teal,
            JobKind::Export => self.amber,
        };
        self.pick(Style::new().fg(fg), Modifier::empty())
    }

    /// Style and glyph of a job's state: `✓` green, `✗` coral, `–` / `…`
    /// dim, `▸` in the kind's colour while running.
    pub fn job_state(&self, kind: JobKind, state: &JobState) -> (Style, &'static str) {
        match state {
            JobState::Done { .. } => (self.value_ok(), "✓"),
            JobState::Failed(_) => (self.value_error(), "✗"),
            JobState::Cancelled => (self.dim(), "–"),
            JobState::Queued => (self.dim(), "…"),
            JobState::Running { .. } => (self.job(kind), "▸"),
        }
    }
}

impl ColorSupport {
    /// What the terminal supports, from the process environment.
    pub fn detect() -> ColorSupport {
        ThemeEnv::from_process().support()
    }

    /// [`ColorSupport::detect`] with an injectable environment:
    /// `NO_COLOR` set and non-empty (no-color.org) → `NoColor`; else
    /// `COLORTERM` = `truecolor` / `24bit` (any case) → `TrueColor`; else
    /// `Ansi256`.
    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>) -> ColorSupport {
        if no_color.is_some_and(|v| !v.is_empty()) {
            return ColorSupport::NoColor;
        }
        match colorterm {
            Some(v) if v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit") => {
                ColorSupport::TrueColor
            }
            _ => ColorSupport::Ansi256,
        }
    }
}

/// How `qry --version` names it.
impl fmt::Display for ColorSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ColorSupport::TrueColor => "24-bit (truecolor)",
            ColorSupport::Ansi256 => "256 colours",
            ColorSupport::NoColor => "none (NO_COLOR)",
        })
    }
}

impl ThemeEnv {
    /// `NO_COLOR` and `COLORTERM` as this process sees them.
    pub fn from_process() -> Self {
        Self {
            no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            colorterm: std::env::var("COLORTERM").ok(),
        }
    }

    /// A 24-bit terminal: `COLORTERM=truecolor`.
    pub fn truecolor() -> Self {
        Self {
            no_color: false,
            colorterm: Some("truecolor".into()),
        }
    }

    /// A terminal with `NO_COLOR` set.
    pub fn no_color() -> Self {
        Self {
            no_color: true,
            colorterm: None,
        }
    }

    /// What a terminal with this environment can display.
    pub fn support(&self) -> ColorSupport {
        ColorSupport::from_env(self.no_color.then_some("1"), self.colorterm.as_deref())
    }

    /// `theme` adapted to this environment.
    pub fn adapt(&self, theme: Theme) -> Theme {
        theme.adapt(self.support())
    }
}

/// The theme to draw with: `--theme` if given, else the `theme` config key,
/// else `dark`, adapted to `support`. An unknown name is an error that lists
/// the names there are.
pub fn resolve(cli: Option<&str>, config: Option<&str>, support: ColorSupport) -> Result<Theme> {
    let name = cli.or(config).unwrap_or("dark");
    let theme = Theme::by_name(name).ok_or_else(|| {
        eyre!(
            "unknown theme `{name}`; the themes are: {}",
            Theme::NAMES.join(", ")
        )
    })?;
    Ok(theme.adapt(support))
}

/// The six levels of each channel in the xterm-256 colour cube (16–231).
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// The RGB value of xterm-256 index `i`, for 16–255.
fn xterm256_rgb(i: u8) -> (u8, u8, u8) {
    debug_assert!(i >= 16);
    if i >= 232 {
        let v = 8 + 10 * (i - 232);
        (v, v, v)
    } else {
        let n = usize::from(i - 16);
        (
            CUBE_LEVELS[n / 36],
            CUBE_LEVELS[n / 6 % 6],
            CUBE_LEVELS[n % 6],
        )
    }
}

/// "Redmean" weighted squared distance between two colours: cheap and much
/// closer to perceived difference than plain RGB distance.
fn redmean_distance(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let rmean = (f64::from(a.0) + f64::from(b.0)) / 2.0;
    let dr = f64::from(a.0) - f64::from(b.0);
    let dg = f64::from(a.1) - f64::from(b.1);
    let db = f64::from(a.2) - f64::from(b.2);
    (2.0 + rmean / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rmean) / 256.0) * db * db
}

/// The closest xterm-256 index to `#rrggbb` among the 6×6×6 cube (16–231)
/// and the grey ramp (232–255), by redmean distance. Indices 0–15 are
/// skipped: users theme those.
pub fn nearest_xterm256(r: u8, g: u8, b: u8) -> u8 {
    (16..=255u8)
        .min_by(|&x, &y| {
            redmean_distance((r, g, b), xterm256_rgb(x))
                .total_cmp(&redmean_distance((r, g, b), xterm256_rgb(y)))
        })
        .unwrap_or(16)
}

/// Rough brightness of an RGB colour, to tell which of two is lighter.
fn luma(c: Color) -> u32 {
    match c {
        Color::Rgb(r, g, b) => 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b),
        _ => 0,
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::DARK
    }
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod theme_tests;
