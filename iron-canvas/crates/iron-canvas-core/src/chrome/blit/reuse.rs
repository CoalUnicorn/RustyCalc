use std::rc::Rc;

use crate::CanvasModel;
use crate::frame::{AxisRange, BlitPlan, FrameInputs, Shift};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point};
use crate::geometry::slot::{AxisSlots, scroll_first};

use crate::chrome::pane_set::{PaneSet, ScrollAxisSlots, row_header_thickness_for};
use crate::chrome::{Chrome, FrameKindTag};

use super::rebuild::ShiftDir;
use super::{BlitRollback, PreparedBlitFrame, PreparedBlitOutcome};

impl BlitPlan {
    /// Compose an axis-scroll `BlitPlan`. `main_pane` covers the pane along
    /// `scroll_axis`; `cross_pane` covers the perpendicular axis. `canvas_main`
    /// caps the repaint strip so it covers the entire untouched zone past the
    /// previous paint, in case `main_pane.size` was short of the canvas edge.
    ///
    /// The blit's cross-axis range matches `cross_pane` exactly: with
    /// `CELL_AREA_INSET` baked into `cell_origin`, the chrome border line
    /// sits at `header_thickness + 0.5` (pixel = `header_thickness`), which
    /// is OUTSIDE the cell area entirely (cells start at `pane_origin =
    /// header_thickness + CELL_AREA_INSET`). `draw_corner_box` repaints
    /// the chrome border every frame regardless, so no inset is needed.
    fn for_axis_scroll(
        scroll_axis: Axis,
        main_pane: AxisRange,
        cross_pane: AxisRange,
        shift_px: i32,
        dir: ShiftDir,
        canvas_main: i32,
    ) -> BlitPlan {
        let kept = main_pane.size - shift_px;
        let cross = cross_pane;
        let (src_main, dst_main, strip_main_origin, strip_main_size) = match dir {
            ShiftDir::Forward => {
                // Source is past the leaving band, dest sits at pane origin.
                // Strip covers everything past the shifted band, through the
                // canvas edge.
                let strip_origin = main_pane.origin + kept;
                (
                    main_pane.origin + shift_px,
                    main_pane.origin,
                    strip_origin,
                    (canvas_main - strip_origin).max(shift_px),
                )
            }
            ShiftDir::Backward => {
                // Source at pane origin, dest shifted forward. Strip fills
                // the newly-revealed near band.
                (
                    main_pane.origin,
                    main_pane.origin + shift_px,
                    main_pane.origin,
                    shift_px,
                )
            }
        };

        // Assemble a PixelRect from main/cross axis ranges. `scroll_axis`
        // selects which dimension is X and which is Y.
        let make_rect = |main_origin: i32, main_size: i32| match scroll_axis {
            Axis::Row => PixelRect {
                top_left: Point {
                    x: cross.origin,
                    y: main_origin,
                },
                width: cross.size,
                height: main_size,
            },
            Axis::Column => PixelRect {
                top_left: Point {
                    x: main_origin,
                    y: cross.origin,
                },
                width: main_size,
                height: cross.size,
            },
        };

        BlitPlan {
            axis: scroll_axis,
            shift: Shift {
                src: make_rect(src_main, kept),
                dst: make_rect(dst_main, kept),
            },
            pixel_strip: make_rect(strip_main_origin, strip_main_size),
        }
    }
}

// Scroll-blit helpers
//
// `Chrome::classify` already disqualified anything that isn't a pure single-axis
// scroll. These helpers compute the canvas-pixel src/dst/strip rects and
// verify the kept band's row heights (col widths) match what the model
// still reports — that is the final qualification that the shifted pixels
// will land where the new chrome would paint them.

/// Build the next-frame Chrome by reusing as much of `prev` as the blit
/// plan guarantees is unchanged: the cross-axis slot Vec and both frozen
/// Vecs are *moved* out of `prev` (their heap allocation transfers to the
/// new frame — no per-scroll-frame clone), the scroll-axis kept band
/// carries forward heights/widths, only the strip touches the model.
///
/// Takes `prev` by value and returns it intact inside
/// [`PreparedBlitOutcome::FreshFallback`] on the one cross-axis-affecting edge
/// case (row_header_thickness changes across a digit boundary) or any model
/// anomaly, so the caller can fall through to a full `Chrome::next`. Every
/// reject happens *before* the first move out of `prev`, so the returned
/// `prev` is always whole.
///
/// On success, returns [`PreparedBlitOutcome::Ready`] wrapping a
/// [`PreparedBlitFrame`] rather than a bare `Chrome`:
/// the caller may still need to reconstruct `prev` if a later step (the
/// strip-prefetch bridge check, run against the returned candidate) fails —
/// see that type's doc. Building `Ready`'s `BlitRollback` costs only moves and
/// `Copy` reads out of fields of `prev` that `candidate` was already about
/// to replace or abandon; nothing is cloned to make rollback possible.
// `Chrome`/`PreparedBlitFrame` are large and intentionally returned by value
// on both arms (the zero-copy give-back on `FreshFallback`); boxing either
// would add a heap alloc to a path that must stay allocation-free on the
// steady-state blit.
pub(super) fn try_blit_reuse(
    mut prev: Chrome,
    model: &dyn CanvasModel,
    inputs: &FrameInputs,
    plan: &BlitPlan,
) -> PreparedBlitOutcome {
    // Hidden row headers previously always used Fresh. Fractional-DPR headers
    // and frozen separators can blend again over copied pixels. Keep Fresh
    // for those cases, and require aligned copy/strip edges in the others.
    let dpr = inputs.dpr();
    let fractional_chrome = dpr.fract() != 0.0
        && (inputs.show_col_headers()
            || !prev.pane_set.rows.frozen.is_empty()
            || !prev.pane_set.cols.frozen.is_empty());
    if !inputs.show_row_headers()
        && (fractional_chrome
            || [plan.shift.src, plan.shift.dst, plan.pixel_strip]
                .into_iter()
                .any(|rect| {
                    let (x, y, w, h) = rect.as_f64_tuple();
                    [x, y, w, h]
                        .into_iter()
                        .any(|value| (value * dpr).fract() != 0.0)
                }))
    {
        return PreparedBlitOutcome::FreshFallback(prev);
    }
    // `inputs.view()` is this attempt's one already-validated read (see
    // `Chrome::build`'s comment) — no `None`/fallback branch needed here.
    let view = inputs.view();
    let canvas = inputs.size();
    // `prev.sheet` is the committed sheet this frame is reused against — used
    // both for the scroll-axis rebuild below and the header-label resolution
    // further down, so it is read once here rather than twice. `Chrome::classify`
    // already proved `inputs.sheet() == prev.sheet` before producing `plan`.
    let sheet = prev.sheet;
    let frozen_rows_count = prev.pane_set.rows.frozen_count();
    let frozen_cols_count = prev.pane_set.cols.frozen_count();
    let new_top = scroll_first(frozen_rows_count, view.top_row);
    let new_left = scroll_first(frozen_cols_count, view.left_column);

    // Rebuild the scrolled axis band, gate on row-header thickness, *then* move
    // anything out of `prev`. The gate runs before the first `mem::take`, so
    // every `Err` below hands `prev` back whole — the invariant the caller's
    // `Chrome::next` fallback (and, since Stage 4, `PreparedBlitFrame::rollback`)
    // relies on.
    //
    // Thickness gates cross-axis reuse: if the new last visible row label grew
    // (e.g. row 99 -> 100), origin_x shifts and every col slot's `.left` is off,
    // so we fall back to a full rebuild. It reads the rebuilt rows band (row
    // scroll) or the still-unchanged cross-axis band (column scroll) — neither
    // taken yet. Once it passes, `old_scroll` captures the scrolled axis's
    // *original* Vec — its last read before `candidate`'s freshly rebuilt
    // replacement takes over — and the cross-axis Vec is moved, not cloned.
    let (scroll_rows, scroll_cols, row_header_thickness, old_scroll) = match plan.axis {
        Axis::Row => {
            let rows = match prev
                .pane_set
                .rebuild_rows_for_row_scroll(model, sheet, new_top, canvas)
            {
                Some(rows) => rows,
                None => return PreparedBlitOutcome::FreshFallback(prev),
            };
            let thickness = row_header_thickness_for(
                &rows,
                frozen_rows_count,
                new_top,
                inputs.show_row_headers(),
            );
            if thickness != prev.row_header_thickness {
                return PreparedBlitOutcome::FreshFallback(prev);
            }
            let old = ScrollAxisSlots::Row(std::mem::take(&mut prev.pane_set.rows.scroll));
            let cols = std::mem::take(&mut prev.pane_set.cols.scroll);
            (rows, cols, thickness, old)
        }
        Axis::Column => {
            let cols = match prev
                .pane_set
                .rebuild_cols_for_col_scroll(model, sheet, new_left, canvas)
            {
                Some(cols) => cols,
                None => return PreparedBlitOutcome::FreshFallback(prev),
            };
            // Cross-axis rows band is unchanged across a column scroll; read it
            // (not taken yet) for the gate.
            let thickness = row_header_thickness_for(
                &prev.pane_set.rows.scroll,
                frozen_rows_count,
                new_top,
                inputs.show_row_headers(),
            );
            if thickness != prev.row_header_thickness {
                return PreparedBlitOutcome::FreshFallback(prev);
            }
            let old = ScrollAxisSlots::Column(std::mem::take(&mut prev.pane_set.cols.scroll));
            let rows = std::mem::take(&mut prev.pane_set.rows.scroll);
            (rows, cols, thickness, old)
        }
    };

    // The scroll-axis vec changed under the blit, so its labels must be
    // re-resolved; rebuilding both keeps the parallel-vec invariant trivially
    // correct. Shares resolution with Chrome::build via PaneSet::resolve_*.
    let row_header_labels =
        PaneSet::resolve_row_labels(model, sheet, &prev.pane_set.rows.frozen, &scroll_rows);
    let col_header_labels =
        PaneSet::resolve_col_labels(model, sheet, &prev.pane_set.cols.frozen, &scroll_cols);

    // Every reject is behind us — snapshot the per-attempt scalars
    // `candidate` is about to refresh from `inputs` instead of from `prev`,
    // plus `prev`'s original header-label Vecs (also about to be replaced),
    // into `rollback`. Moves and `Copy` reads only: `theme` is `prev`'s
    // original `Rc` handle, moved out (not `Rc::clone`d — `prev` has no
    // further use for it); the rest are `Copy`.
    let rollback = BlitRollback {
        scroll: old_scroll,
        row_header_labels: std::mem::take(&mut prev.pane_set.row_header_labels),
        col_header_labels: std::mem::take(&mut prev.pane_set.col_header_labels),
        theme: prev.theme,
        metrics: prev.metrics,
        model_generation: prev.model_generation,
        show_row_headers: prev.show_row_headers,
        show_col_headers: prev.show_col_headers,
        kind: prev.kind,
    };

    // Frozen bands are unchanged across a scroll, and their labels are now
    // resolved — move the Vecs out of `prev` (this is the last read of them).
    let pane_set = PaneSet {
        rows: AxisSlots {
            frozen: std::mem::take(&mut prev.pane_set.rows.frozen),
            scroll: scroll_rows,
            frozen_offset: prev.pane_set.rows.frozen_offset,
            last_id: prev.pane_set.rows.last_id,
        },
        cols: AxisSlots {
            frozen: std::mem::take(&mut prev.pane_set.cols.frozen),
            scroll: scroll_cols,
            frozen_offset: prev.pane_set.cols.frozen_offset,
            last_id: prev.pane_set.cols.last_id,
        },
        row_header_labels,
        col_header_labels,
    };

    // The singular pixel shift stays on `BlitPlan`; exact address strips are
    // derived later from this reversible candidate's `GridLayout`. `Chrome`
    // therefore owns no content-scope or cache state. `GridCache` installs
    // the matching layout/buffer/fingerprint transition only after the
    // renderer has prepared and executed the whole grid transaction.
    let candidate = Chrome {
        sheet: prev.sheet,
        pane_set,
        row_header_thickness,
        col_header_thickness: prev.col_header_thickness,
        cell_origin: prev.cell_origin,
        metrics: inputs.metrics(),
        theme: Rc::clone(inputs.theme()),
        model_generation: inputs.model_generation(),
        show_row_headers: inputs.show_row_headers(),
        show_col_headers: inputs.show_col_headers(),
        kind: FrameKindTag::Blitted,
    };

    PreparedBlitOutcome::Ready(PreparedBlitFrame {
        candidate,
        rollback,
    })
}

pub(crate) fn try_blit_rows(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_top: i32,
) -> Option<BlitPlan> {
    let (canvas_w, canvas_h) = prev.metrics.logical_extent();
    let pane_x = prev.pane_set.cols.frozen_offset;
    let pane_y = prev.pane_set.rows.frozen_offset;
    // pane_h is bounded by the canvas backing store extent, not by
    // `scroll_rows.last().top + height`. `fill_axis` pushes one row past
    // the canvas edge (the "overflow row") whose pixels were never on
    // canvas — using slot-bound pane_h here would send drawImage's
    // source rect past the backing store and the spec's proportional
    // source/dest clip would leave the bottom row stale.
    let pane_w = canvas_w - pane_x;
    let pane_h = canvas_h - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    let (shift_px, dir) = prev
        .pane_set
        .probe_row_shift(model, sheet, new_top, pane_y, pane_h)?;

    Some(BlitPlan::for_axis_scroll(
        Axis::Row,
        AxisRange {
            origin: pane_y,
            size: pane_h,
        },
        AxisRange {
            origin: prev.cell_origin.x,
            size: canvas_w - prev.cell_origin.x,
        },
        shift_px,
        dir,
        canvas_h,
    ))
}

pub(crate) fn try_blit_cols(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_left: i32,
) -> Option<BlitPlan> {
    let (canvas_w, canvas_h) = prev.metrics.logical_extent();
    let pane_x = prev.pane_set.cols.frozen_offset;
    let pane_y = prev.pane_set.rows.frozen_offset;
    // pane_w is bounded by the canvas backing store extent, not by
    // `scroll_cols.last().left + width` — mirror of the comment in
    // try_blit_rows.
    let pane_w = canvas_w - pane_x;
    let pane_h = canvas_h - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    let (shift_px, dir) = prev
        .pane_set
        .probe_col_shift(model, sheet, new_left, pane_x, pane_w)?;

    Some(BlitPlan::for_axis_scroll(
        Axis::Column,
        AxisRange {
            origin: pane_x,
            size: pane_w,
        },
        AxisRange {
            origin: prev.cell_origin.y,
            size: canvas_h - prev.cell_origin.y,
        },
        shift_px,
        dir,
        canvas_w,
    ))
}
