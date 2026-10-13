//! Unit tests for `layout.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

const SIZES: [(u16, u16); 4] = [(80, 24), (120, 40), (160, 48), (300, 100)];

fn input(w: u16, h: u16) -> LayoutInput {
    LayoutInput {
        area: Rect::new(0, 0, w, h),
        split_percent: DEFAULT_SPLIT,
        maximised: None,
        inspector: false,
        jobs_rows: 0,
        hints: true,
    }
}

fn layout(input: &LayoutInput) -> ScreenLayout {
    compute_layout(input).unwrap()
}

/// Every pair of regions is disjoint, and together they cover the area.
fn assert_tiles(input: &LayoutInput) {
    let screen = layout(input);
    let regions = screen.regions();
    for (i, a) in regions.iter().enumerate() {
        for b in &regions[i + 1..] {
            assert!(
                a.intersection(*b).is_empty(),
                "{a:?} overlaps {b:?} in {input:?}"
            );
        }
    }
    let area: u32 = regions
        .iter()
        .map(|r| u32::from(r.width) * u32::from(r.height))
        .sum();
    assert_eq!(
        area,
        u32::from(input.area.width) * u32::from(input.area.height),
        "{input:?}"
    );
}

#[test]
fn too_small_below_80x24_returns_none() {
    for (w, h) in [(79, 24), (80, 23), (0, 0)] {
        assert_eq!(compute_layout(&input(w, h)), None, "{w}x{h}");
    }
    assert!(compute_layout(&input(80, 24)).is_some());
}

#[test]
fn regions_tile_the_screen_without_overlap() {
    for (w, h) in SIZES {
        assert_tiles(&input(w, h));
        let mut all = input(w, h);
        all.inspector = true;
        all.jobs_rows = 5;
        all.hints = false;
        assert_tiles(&all);
        for region in [Region::Editor, Region::Results] {
            let mut maximised = input(w, h);
            maximised.maximised = Some(region);
            assert_tiles(&maximised);
        }
    }
}

#[test]
fn tree_width_is_30_below_160_and_quarter_above() {
    assert_eq!(layout(&input(80, 24)).tree.width, 30);
    assert_eq!(layout(&input(159, 40)).tree.width, 30);
    assert_eq!(layout(&input(160, 48)).tree.width, 40);
    assert_eq!(layout(&input(300, 100)).tree.width, 75);
}

#[test]
fn inspector_slot_only_when_toggled_and_wide_enough() {
    let mut on = input(139, 40);
    on.inspector = true;
    assert_eq!(layout(&on).inspector, None);
    on.area.width = 140;
    let screen = layout(&on);
    assert_eq!(screen.inspector.unwrap().width, INSPECTOR_WIDTH);
    assert_eq!(screen.inspector.unwrap().right(), 140);
    assert_eq!(layout(&input(140, 40)).inspector, None, "toggled off");
}

#[test]
fn split_percent_steps_and_keeps_three_rows_each() {
    let mut grow = input(80, 24);
    for _ in 0..20 {
        grow.split_percent = (grow.split_percent + SPLIT_STEP).min(SPLIT_RANGE.1);
        let screen = layout(&grow);
        assert!(screen.results.height >= MIN_REGION_ROWS, "{screen:?}");
        assert!(screen.editor.height >= MIN_REGION_ROWS, "{screen:?}");
    }
    let mut shrink = input(80, 24);
    for _ in 0..20 {
        shrink.split_percent = shrink
            .split_percent
            .saturating_sub(SPLIT_STEP)
            .max(SPLIT_RANGE.0);
        assert!(layout(&shrink).editor.height >= MIN_REGION_ROWS);
    }
    // 5% of the 36 shared rows at 120x40 is about 2 rows a step.
    let at = |split| {
        let mut i = input(120, 40);
        i.split_percent = split;
        layout(&i).editor.height
    };
    assert_eq!((at(40), at(45), at(50)), (14, 16, 18));
}

#[test]
fn maximise_gives_the_focused_region_the_whole_right_side_and_restores() {
    let normal = layout(&input(120, 40));
    let mut maximised = input(120, 40);
    maximised.maximised = Some(Region::Results);
    let screen = layout(&maximised);
    assert_eq!(screen.editor.height + screen.editor_title.height, 0);
    assert_eq!(screen.results_title.y, normal.editor_title.y);
    assert_eq!(screen.results.bottom(), normal.results.bottom());

    maximised.maximised = Some(Region::Editor);
    let screen = layout(&maximised);
    assert_eq!(screen.results.height + screen.results_title.height, 0);
    assert_eq!(screen.editor.bottom(), normal.results.bottom());

    maximised.maximised = None;
    assert_eq!(layout(&maximised), normal);
}

#[test]
fn jobs_drawer_and_hints_take_rows_from_the_body() {
    let base = layout(&input(120, 40));
    let mut no_hints = input(120, 40);
    no_hints.hints = false;
    let screen = layout(&no_hints);
    assert_eq!(screen.hints, None);
    assert_eq!(screen.tree.height, base.tree.height + 1);
    assert_eq!(screen.status.y, 39);

    let mut jobs = input(120, 40);
    jobs.jobs_rows = 4;
    let screen = layout(&jobs);
    assert_eq!(screen.jobs.unwrap().height, 4);
    assert_eq!(screen.jobs.unwrap().bottom(), screen.status.y);
    assert_eq!(screen.tree.height, base.tree.height - 4);
    // The toast sits on the last body row.
    assert_eq!(screen.toast.y, screen.tree.bottom() - 1);
}

#[test]
fn hit_test_maps_clicks_to_panes_and_tabs() {
    let screen = layout(&input(120, 40));
    let tabs = vec![(0, Rect::new(5, 0, 11, 1)), (1, Rect::new(16, 0, 8, 1))];
    assert_eq!(hit_test(&screen, &tabs, 6, 0), Some(Hit::Tab(0)));
    assert_eq!(hit_test(&screen, &tabs, 20, 0), Some(Hit::Tab(1)));
    assert_eq!(
        hit_test(&screen, &tabs, 60, 0),
        None,
        "the bar between tabs"
    );
    assert_eq!(hit_test(&screen, &tabs, 3, 10), Some(Hit::Tree));
    assert_eq!(
        hit_test(&screen, &tabs, 50, screen.editor.y + 1),
        Some(Hit::Editor)
    );
    assert_eq!(
        hit_test(&screen, &tabs, 50, screen.results_title.y),
        Some(Hit::Results)
    );
    assert_eq!(hit_test(&screen, &tabs, 30, 10), None, "the divider");
    assert_eq!(hit_test(&screen, &tabs, 5, screen.status.y), None);
}
