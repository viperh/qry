//! Where each region of the main screen goes, as plain `Rect`s that tests
//! can check without drawing, ported from tachy's `layout.rs`.
//!
//! Top to bottom: the top bar, the body (tree, `│`, editor over results,
//! and the inspector behind another `│` when it is on), the jobs drawer,
//! the status line and the key-hint line. Below [`MIN_SIZE`] nothing fits
//! and only [`render_too_small`] is drawn.

use ratatui::{
    layout::Flex,
    prelude::*,
    widgets::{Block, Paragraph},
};

use crate::theme::Theme;

/// The smallest terminal the main screen is drawn in.
pub(crate) const MIN_SIZE: (u16, u16) = (80, 24);
/// From this width the tree takes a quarter of the screen instead of
/// [`TREE_WIDTH`] columns.
pub(crate) const WIDE: u16 = 160;
pub(crate) const TREE_WIDTH: u16 = 30;
/// The inspector is docked from this width; below it, it is drawn over
/// the right half of the body.
pub(crate) const INSPECTOR_MIN_WIDTH: u16 = 140;
pub(crate) const INSPECTOR_WIDTH: u16 = 40;
/// The fewest body rows the editor and the results each keep.
pub(crate) const MIN_REGION_ROWS: u16 = 3;
/// The editor's share of the right-hand side, in percent, and how far one
/// `GrowEditor` / `ShrinkEditor` moves it.
pub(crate) const DEFAULT_SPLIT: u16 = 40;
pub(crate) const SPLIT_STEP: u16 = 5;
pub(crate) const SPLIT_RANGE: (u16, u16) = (10, 90);

/// A right-hand region that can be maximised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Editor,
    Results,
}

/// What the layout depends on besides the terminal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayoutInput {
    pub area: Rect,
    /// The editor's share of the right-hand body, `SPLIT_RANGE` inclusive.
    pub split_percent: u16,
    pub maximised: Option<Region>,
    pub inspector: bool,
    /// 0 while the jobs drawer is closed.
    pub jobs_rows: u16,
    pub hints: bool,
}

/// Every region of the main screen. Together (all but `toast`) they tile
/// the area: no gaps and no overlaps. A maximised region's neighbour has no
/// height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenLayout {
    pub top_bar: Rect,
    /// The tree, its title row included.
    pub tree: Rect,
    pub tree_divider: Rect,
    pub editor_title: Rect,
    pub editor: Rect,
    pub results_title: Rect,
    pub results: Rect,
    pub inspector_divider: Option<Rect>,
    pub inspector: Option<Rect>,
    pub jobs: Option<Rect>,
    pub status: Rect,
    pub hints: Option<Rect>,
    /// Where a toast is drawn over whatever is there, so the layout never
    /// jumps: the last row of the right-hand side, or of the jobs drawer
    /// when it is open.
    pub toast: Rect,
}

impl ScreenLayout {
    /// Every region that takes room, for tests that check the tiling.
    #[cfg(test)]
    pub(crate) fn regions(&self) -> Vec<Rect> {
        let mut regions = vec![
            self.top_bar,
            self.tree,
            self.tree_divider,
            self.editor_title,
            self.editor,
            self.results_title,
            self.results,
            self.status,
        ];
        regions.extend(self.inspector_divider);
        regions.extend(self.inspector);
        regions.extend(self.jobs);
        regions.extend(self.hints);
        regions
    }
}

/// Where everything goes, or `None` when the area is smaller than
/// [`MIN_SIZE`].
pub(crate) fn compute_layout(input: &LayoutInput) -> Option<ScreenLayout> {
    let area = input.area;
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        return None;
    }
    let row = |y: u16, height: u16| Rect::new(area.x, y, area.width, height);

    let top_bar = row(area.y, 1);
    let bottom = area.bottom();
    let hints = input.hints.then(|| row(bottom - 1, 1));
    let status_y = bottom - 1 - u16::from(input.hints);
    let status = row(status_y, 1);
    let body_top = area.y + 1;
    // The drawer never takes the body below what the panes need.
    let max_jobs = (status_y - body_top).saturating_sub(2 * MIN_REGION_ROWS + 2);
    let jobs_rows = input.jobs_rows.min(max_jobs);
    let jobs = (jobs_rows > 0).then(|| row(status_y - jobs_rows, jobs_rows));
    let body = Rect::new(
        area.x,
        body_top,
        area.width,
        status_y - jobs_rows - body_top,
    );

    let tree_width = if area.width >= WIDE {
        area.width / 4
    } else {
        TREE_WIDTH
    };
    let tree = Rect::new(body.x, body.y, tree_width, body.height);
    let tree_divider = Rect::new(tree.right(), body.y, 1, body.height);
    let mut right = Rect::new(
        tree_divider.right(),
        body.y,
        body.right() - tree_divider.right(),
        body.height,
    );

    let (inspector_divider, inspector) = if input.inspector && area.width >= INSPECTOR_MIN_WIDTH {
        right.width -= INSPECTOR_WIDTH + 1;
        let divider = Rect::new(right.right(), body.y, 1, body.height);
        let inspector = Rect::new(divider.right(), body.y, INSPECTOR_WIDTH, body.height);
        (Some(divider), Some(inspector))
    } else {
        (None, None)
    };

    // Two title rows, and the rest shared by the split.
    let rows = body.height.saturating_sub(2);
    let (editor_title_rows, editor_rows, results_title_rows, results_rows) = match input.maximised {
        Some(Region::Editor) => (1, body.height - 1, 0, 0),
        Some(Region::Results) => (0, 0, 1, body.height - 1),
        None => {
            let split = input.split_percent.clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
            let editor = (u32::from(rows) * u32::from(split) + 50) / 100;
            let editor = u16::try_from(editor)
                .unwrap_or(rows)
                .clamp(MIN_REGION_ROWS, rows.saturating_sub(MIN_REGION_ROWS));
            (1, editor, 1, rows - editor)
        }
    };
    let mut y = right.y;
    let mut take = |height: u16| {
        let rect = Rect::new(right.x, y, right.width, height);
        y += height;
        rect
    };
    let editor_title = take(editor_title_rows);
    let editor = take(editor_rows);
    let results_title = take(results_title_rows);
    let results = take(results_rows);

    Some(ScreenLayout {
        top_bar,
        tree,
        tree_divider,
        editor_title,
        editor,
        results_title,
        results,
        inspector_divider,
        inspector,
        jobs,
        status,
        hints,
        toast: Rect::new(
            right.x,
            jobs.map_or(body.bottom(), |jobs| jobs.bottom()) - 1,
            right.width,
            1,
        ),
    })
}

/// What a click landed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    Tree,
    Editor,
    Results,
    /// The tab at this position in the tab order.
    Tab(usize),
}

/// Where each session tab was drawn in the top bar, by position.
pub(crate) type TabHits = Vec<(usize, Rect)>;

/// What is at `(col, row)`. Titles belong to their region.
pub(crate) fn hit_test(layout: &ScreenLayout, tabs: &TabHits, col: u16, row: u16) -> Option<Hit> {
    let at = Position::new(col, row);
    if let Some((i, _)) = tabs.iter().find(|(_, rect)| rect.contains(at)) {
        return Some(Hit::Tab(*i));
    }
    if layout.tree.contains(at) {
        Some(Hit::Tree)
    } else if layout.editor_title.contains(at) || layout.editor.contains(at) {
        Some(Hit::Editor)
    } else if layout.results_title.contains(at) || layout.results.contains(at) {
        Some(Hit::Results)
    } else {
        None
    }
}

/// A `width`x`height` rect centred in `area`, shrunk to fit.
pub(crate) fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [rect] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [rect] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(rect);
    rect
}

/// A `│` down the whole of `area`.
pub(crate) fn render_vertical_divider(frame: &mut Frame, area: Rect, style: Style) {
    let lines: Vec<Line> = (0..area.height).map(|_| Line::raw("│")).collect();
    frame.render_widget(Paragraph::new(lines).style(style), area);
}

/// All there is to draw in a terminal smaller than [`MIN_SIZE`].
pub(crate) fn render_too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(Block::new().style(theme.base()), area);
    let message = format!(
        "terminal too small — {}x{} needed, have {}x{}",
        MIN_SIZE.0, MIN_SIZE.1, area.width, area.height
    );
    let rect = centered_rect(area, area.width, 1);
    frame.render_widget(
        Paragraph::new(message)
            .style(theme.status_warning())
            .centered(),
        rect,
    );
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod layout_tests;
