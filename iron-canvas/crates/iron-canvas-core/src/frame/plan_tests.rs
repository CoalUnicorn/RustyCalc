//! `plan_frame` table coverage: every `PendingWork` category x `FrameDelta`
//! outcome cell, plus the rules the doc comment calls out by name.
//!
//! These are unit tests over the pure `plan_frame` function directly, not
//! `Orchestrator::render_pending`. `GridWork`/`OverlayWork`/`FramePlan` are
//! crate-private with crate-visible fields, so this sibling test module can
//! construct and inspect them.

use crate::chrome::merge::MergeImpact;
use crate::frame::delta::{BlitPlan, FrameDelta, RebuildReason, Shift};
use crate::frame::plan::{GridWork, OverlayWork, RenderStrategy, plan_frame};
use crate::frame::work::{PendingWork, RowSpan};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point};

const SHEET: u32 = 0;
const OTHER_SHEET: u32 = 7;

fn work_with(f: impl FnOnce(&mut PendingWork)) -> PendingWork {
    let mut work = PendingWork::default();
    f(&mut work);
    work
}

/// The planner never inspects a `BlitPlan`'s contents; it only wraps
/// whatever `Chrome::classify` handed it into `GridWork::Blit`.
fn stub_scroll() -> FrameDelta {
    FrameDelta::Scroll(BlitPlan {
        axis: Axis::Row,
        shift: Shift {
            src: PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 10,
                height: 10,
            },
            dst: PixelRect {
                top_left: Point { x: 0, y: 1 },
                width: 10,
                height: 10,
            },
        },
        pixel_strip: PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: 10,
            height: 10,
        },
    })
}

fn stub_rebuild() -> FrameDelta {
    FrameDelta::Rebuild(RebuildReason::Sheet)
}

// ── Strategy derivation: `GridWork` is the single strategy authority ──

/// The one-to-one `GridWork -> RenderStrategy` mapping, pinned directly.
/// The planner tests below assert the same mapping through `plan_frame`;
/// this test guards `GridWork::strategy` itself so a future variant
/// cannot drift from the tag `render_pending` stamps into `last_strategy`
/// (there is no stored `FramePlan.selected_strategy` left to disagree).
#[test]
fn grid_work_derives_the_strategy_tag() {
    assert_eq!(GridWork::None.strategy(), RenderStrategy::OverlayOnly);
    assert_eq!(GridWork::Fresh.strategy(), RenderStrategy::FullRebuild);
    assert_eq!(
        GridWork::AllContent.strategy(),
        RenderStrategy::ChangedCells
    );
    assert_eq!(
        GridWork::Rows {
            sheet: 0,
            spans: vec![RowSpan::new(1, 2)],
        }
        .strategy(),
        RenderStrategy::DamagedRows
    );
    let FrameDelta::Scroll(plan) = stub_scroll() else {
        unreachable!("stub_scroll always plans a scroll");
    };
    assert_eq!(GridWork::Blit(plan).strategy(), RenderStrategy::ScrollBlit);
}

// ── Required hot-path assertion ──

/// `view + overlay, FrameDelta::Stable -> selected Overlay ->
/// GridWork::None -> zero grid operations`. The single most important
/// regression to pin: a stable, no-shift view/overlay-only attempt must
/// plan zero grid work, or every arrow-key press regresses to a
/// full-grid repaint.
#[test]
fn view_and_overlay_stable_selects_overlay_only_with_no_grid_work() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::OverlayOnly);
    assert!(
        matches!(plan.grid, GridWork::None),
        "a stable, no-shift view+overlay attempt must plan zero grid work"
    );
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

// ── Category: overlay/view only, no content, no geometry ──

#[test]
fn overlay_only_stable_selects_overlay_only() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::OverlayOnly);
    assert!(matches!(plan.grid, GridWork::None));
}

/// The no-shift view fallback with `view` as the *only* mark (no
/// `overlay`) — proves the Overlay guard's `work.has_view()` disjunct
/// specifically. Regressing this to require `has_overlay()` too would
/// turn ordinary arrow-key navigation into a full-grid repaint.
#[test]
fn view_only_no_shift_still_selects_overlay_only() {
    let work = work_with(|w| w.mark_view());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(
        plan.grid.strategy(),
        RenderStrategy::OverlayOnly,
        "view alone, with no pixel shift, must still fall back to Overlay"
    );
    assert!(matches!(plan.grid, GridWork::None));
}

#[test]
fn view_and_overlay_scroll_selects_scroll_blit() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
    });
    let plan = plan_frame(work, stub_scroll(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::ScrollBlit);
    assert!(matches!(plan.grid, GridWork::Blit(_)));
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

/// Legacy overlay-only scroll discovery: no `view` mark at all, only
/// `overlay` — the probe must still claim a real geometric scroll. This
/// is also the renderer's own correctness fallback for a host that moved
/// the view without calling `view_changed`.
#[test]
fn overlay_only_scroll_still_selects_scroll_blit() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(work, stub_scroll(), SHEET, true, false, MergeImpact::None);

    assert_eq!(
        plan.grid.strategy(),
        RenderStrategy::ScrollBlit,
        "an overlay-only wakeup must still discover a real geometric scroll"
    );
    assert!(matches!(plan.grid, GridWork::Blit(_)));
}

#[test]
fn view_and_overlay_rebuild_selects_full_rebuild() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
    });
    let plan = plan_frame(work, stub_rebuild(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
    assert_eq!(plan.overlay, OverlayWork::Paint);
    assert_eq!(plan.rebuild_reason, Some(RebuildReason::Sheet));
}

// ── Category: row content only — both row-sheet outcomes ──

#[test]
fn row_content_stable_matching_sheet_selects_damaged_rows() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 4)));
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::DamagedRows);
    let GridWork::Rows { sheet, spans } = plan.grid else {
        panic!("expected GridWork::Rows");
    };
    assert_eq!(sheet, SHEET);
    assert_eq!(spans, vec![RowSpan::new(2, 4)]);
}

#[test]
fn row_content_stable_mismatched_sheet_falls_back_to_changed_cells_all() {
    let work = work_with(|w| w.mark_rows(OTHER_SHEET, RowSpan::new(2, 4)));
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(
        plan.grid.strategy(),
        RenderStrategy::ChangedCells,
        "row work recorded against a sheet that isn't on screen can't clip to bands"
    );
    assert!(matches!(plan.grid, GridWork::AllContent));
}

#[test]
fn row_content_scroll_selects_full_rebuild() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 4)));
    let plan = plan_frame(work, stub_scroll(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

#[test]
fn row_content_rebuild_selects_full_rebuild() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 4)));
    let plan = plan_frame(work, stub_rebuild(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

// ── Category: whole-grid content only ──

#[test]
fn all_content_stable_selects_changed_cells() {
    let work = work_with(PendingWork::mark_all_content);
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::ChangedCells);
    assert!(matches!(plan.grid, GridWork::AllContent));
}

#[test]
fn all_content_scroll_selects_full_rebuild() {
    let work = work_with(PendingWork::mark_all_content);
    let plan = plan_frame(work, stub_scroll(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

#[test]
fn all_content_rebuild_selects_full_rebuild() {
    let work = work_with(PendingWork::mark_all_content);
    let plan = plan_frame(work, stub_rebuild(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

// ── Category: content plus view ──

#[test]
fn content_rows_plus_view_stable_selects_damaged_rows() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
        w.mark_rows(SHEET, RowSpan::new(1, 3));
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::DamagedRows);
    let GridWork::Rows { sheet, spans } = plan.grid else {
        panic!("expected GridWork::Rows");
    };
    assert_eq!(sheet, SHEET);
    assert_eq!(spans, vec![RowSpan::new(1, 3)]);
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

#[test]
fn all_content_plus_view_stable_selects_changed_cells() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
        w.mark_all_content();
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::ChangedCells);
    assert!(matches!(plan.grid, GridWork::AllContent));
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

#[test]
fn content_rows_wrong_sheet_plus_view_stable_selects_changed_cells_all() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
        w.mark_rows(OTHER_SHEET, RowSpan::new(1, 3));
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::ChangedCells);
    assert!(matches!(plan.grid, GridWork::AllContent));
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

#[test]
fn content_plus_view_scroll_selects_full_rebuild() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_all_content();
    });
    let plan = plan_frame(work, stub_scroll(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

#[test]
fn content_plus_view_rebuild_selects_full_rebuild() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_rows(SHEET, RowSpan::new(1, 1));
    });
    let plan = plan_frame(work, stub_rebuild(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
    assert_eq!(plan.rebuild_reason, Some(RebuildReason::Sheet));
}

#[test]
fn geometry_plus_content_view_stable_selects_full_rebuild() {
    let work = work_with(|w| {
        w.mark_geometry();
        w.mark_view();
        w.mark_overlay();
        w.mark_rows(SHEET, RowSpan::new(1, 1));
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

// ── Category: any geometry — always Fresh, any delta ──

#[test]
fn geometry_alone_stable_selects_full_rebuild() {
    let work = work_with(|w| w.mark_geometry());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

#[test]
fn geometry_alone_rebuild_selects_full_rebuild() {
    let work = work_with(|w| w.mark_geometry());
    let plan = plan_frame(work, stub_rebuild(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

/// Mirrors `orchestrator_strategies.rs`'s
/// `geometry_plus_real_scroll_never_dispatches_viewport`: geometry work
/// concurrent with a real shift must never dispatch `ScrollBlit`.
#[test]
fn geometry_with_everything_else_still_selects_full_rebuild() {
    let work = work_with(|w| {
        w.mark_geometry();
        w.mark_view();
        w.mark_overlay();
        w.mark_all_content();
    });
    let plan = plan_frame(work, stub_scroll(), SHEET, true, false, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
}

// ── OverlayWork policy ──

#[test]
fn damaged_rows_preserves_overlay_when_selection_hidden_and_no_overlay_mark() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 2)));
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        false,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::DamagedRows);
    assert_eq!(
        plan.overlay,
        OverlayWork::Preserve,
        "content-only work must preserve the overlay when selection painting is disabled"
    );
}

#[test]
fn damaged_rows_paints_overlay_when_selection_is_visible() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 2)));
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.overlay, OverlayWork::Paint);
}

#[test]
fn changed_cells_preserves_overlay_when_selection_hidden_and_no_overlay_mark() {
    let work = work_with(PendingWork::mark_all_content);
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        false,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::ChangedCells);
    assert_eq!(plan.overlay, OverlayWork::Preserve);
}

#[test]
fn changed_cells_paints_overlay_when_overlay_marked_even_with_selection_hidden() {
    let work = work_with(|w| {
        w.mark_all_content();
        w.mark_view();
        w.mark_overlay();
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        false,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::ChangedCells);
    assert_eq!(
        plan.overlay,
        OverlayWork::Paint,
        "an explicit overlay mark must paint regardless of selection visibility"
    );
}

#[test]
fn full_rebuild_always_paints_overlay_even_with_selection_hidden() {
    let work = work_with(|w| w.mark_geometry());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        false,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

// ── FramePlan owns the taken PendingWork ──

#[test]
fn plan_owns_the_taken_work() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert!(plan.consumes.has_overlay());
}

// ── Link metadata escalation ──

/// A link-set change with no content or geometry mark must still repaint the
/// grid: the new underline/color and the new query data commit together.
/// Without this rule a stable view/overlay attempt would plan zero grid work
/// and the changed link would never reach the pixels.
#[test]
fn link_change_escalates_stable_overlay_attempt_to_all_content() {
    let work = work_with(|w| {
        w.mark_view();
        w.mark_overlay();
    });
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        true,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::ChangedCells);
    assert!(matches!(plan.grid, GridWork::AllContent));
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

/// A safe scroll must not carry a changed link set: the blit would keep stale
/// link pixels and publish new link state over them. A real scroll cannot
/// reuse committed slots, so the escalation is a full rebuild.
#[test]
fn link_change_bars_blit_on_a_safe_scroll() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(work, stub_scroll(), SHEET, true, true, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

/// Geometry work keeps its `Fresh` fallback: a full rebuild installs complete
/// fingerprint truth, links included, and needs no separate escalation.
#[test]
fn link_change_does_not_override_geometry_rebuild() {
    let work = work_with(|w| w.mark_geometry());
    let plan = plan_frame(work, stub_rebuild(), SHEET, true, true, MergeImpact::None);

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

/// The escalation is conditional: an unchanged link digest leaves the cheap
/// overlay-only path alone, so ordinary selection movement stays cheap.
#[test]
fn unchanged_links_keep_the_overlay_only_path() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert!(matches!(plan.grid, GridWork::None));
}

// ── Merge escalation ──

/// A merge in the view forces a rebuild even when the work would otherwise be
/// a row-scoped damage paint: a fast path that repaints covered cells without
/// the merge pass would paint cell content over the merged region.
#[test]
fn visible_merge_forces_full_rebuild_over_row_damage() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 4)));
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::Visible,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

/// A merge change bars the blit fast path: the kept band would shift stale
/// merge pixels and publish new merge geometry over them.
#[test]
fn changed_merge_bars_blit_on_a_safe_scroll() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(
        work,
        stub_scroll(),
        SHEET,
        true,
        false,
        MergeImpact::Changed,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
}

/// No merge anywhere leaves every existing strategy untouched — an ordinary
/// sheet keeps its cheap paths.
#[test]
fn no_merge_impact_keeps_the_existing_strategy() {
    let work = work_with(|w| w.mark_rows(SHEET, RowSpan::new(2, 4)));
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::None,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::DamagedRows);
}

/// A *changed* merge table must not ride the overlay-only arm: that arm commits
/// the previous frame unchanged, so the newly captured table would be dropped
/// and its geometry would stay uncommitted until some other attempt rebuilt.
#[test]
fn changed_merge_bars_the_overlay_only_arm() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::Changed,
    );

    assert_eq!(plan.grid.strategy(), RenderStrategy::FullRebuild);
    assert!(matches!(plan.grid, GridWork::Fresh));
    assert_eq!(plan.overlay, OverlayWork::Paint);
}

/// An overlay-only attempt on stable geometry stays overlay-only even with a
/// merge in view: the merge pixels are already committed, and nothing repaints
/// the grid, so a rebuild would be pure waste on every selection move.
#[test]
fn visible_merge_keeps_stable_overlay_only_cheap() {
    let work = work_with(|w| w.mark_overlay());
    let plan = plan_frame(
        work,
        FrameDelta::Stable,
        SHEET,
        true,
        false,
        MergeImpact::Visible,
    );

    assert!(matches!(plan.grid, GridWork::None));
}
