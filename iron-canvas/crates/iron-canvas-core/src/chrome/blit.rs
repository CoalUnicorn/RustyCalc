//! Single-axis scroll blit fast-path: detection, plan construction, and
//! Chrome reuse around a `Painter::blit` shift.
//!
//! `Chrome::screen_for_blit` (mod.rs) screens the disqualifiers (sheet / freeze /
//! canvas / theme / active-cell mismatch, two-axis scroll); on a viable
//! scroll it delegates to `try_blit_rows` / `try_blit_cols` here, which
//! return a `BlitPlan` if the geometry checks out. The orchestrator then
//! calls `Chrome::next(.., FramePath::Blit(plan))`, which routes through
//! `try_blit_reuse` to construct the next frame in place — kept band
//! carries forward, only the strip hits the model.

use std::cell::Cell;
use std::rc::Rc;

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point};
use crate::geometry::slot::{AxisSlots, RowSlot, scroll_first};
use crate::theme::CanvasTheme;
use crate::{CanvasModel, CanvasSize};

use super::blit_rebuild::ShiftDir;
use super::{Chrome, FrameKindTag, PaneRegion, PaneRegionMask, PaneSet, measure_row_header_width};

/// One pane's contribution to a scroll-blit. A row-axis scroll emits a
/// `PaneShift` for `BottomRight` and, when `frozen_cols > 0`, another
/// for `BottomLeft`; a column-axis scroll mirrors with `TopRight` when
/// `frozen_rows > 0`. `src` and `dst` have identical `width`/`height`;
/// only the offset along the scroll axis differs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneShift {
    pub pane: PaneRegion,
    pub src: PixelRect,
    pub dst: PixelRect,
}

impl PaneShift {
    /// Build a sibling shift in a cross-axis pane (the frozen-cross band).
    /// The main-axis range carries over from `self`; only the cross-axis
    /// origin/size are replaced. Row scrolls produce a BottomLeft sibling
    /// (cross axis = X); column scrolls produce a TopRight sibling
    /// (cross axis = Y).
    fn for_frozen_band(
        &self,
        pane: PaneRegion,
        scroll_axis: Axis,
        cross_origin: i32,
        cross_size: i32,
    ) -> PaneShift {
        let band = |rect: PixelRect| match scroll_axis {
            Axis::Row => PixelRect {
                top_left: Point {
                    x: cross_origin,
                    y: rect.top_left.y,
                },
                width: cross_size,
                height: rect.height,
            },
            Axis::Column => PixelRect {
                top_left: Point {
                    x: rect.top_left.x,
                    y: cross_origin,
                },
                width: rect.width,
                height: cross_size,
            },
        };
        PaneShift {
            pane,
            src: band(self.src),
            dst: band(self.dst),
        }
    }
}

/// 1D pixel range along a single axis (origin + size). Used by
/// `BlitPlan::for_axis_scroll` to thread main-axis and cross-axis extents
/// through axis-agnostic code without committing to X-vs-Y until the rect
/// is assembled.
#[derive(Clone, Copy)]
struct AxisRange {
    origin: i32,
    size: i32,
}

/// Pure-canvas-pixel description of a scroll-blit. `shifts` lists every
/// pane the painter must copy (1 entry without frozen cross-axis lines,
/// 2 when a frozen band crosses the scroll axis); `repaint_strip` is the
/// shared band the renderer must paint over to fill in newly-revealed
/// content. Axis tells the orchestrator which header strip to repaint
/// (the cross-axis header is untouched by the scroll).
///
/// All rects are in CSS pixels relative to the canvas origin — the
/// `Painter::blit` backend handles DPR.
#[derive(Clone)]
#[must_use = "a BlitPlan represents a committed viewport-shift decision; dropping it means the blit never happens"]
pub struct BlitPlan {
    pub axis: Axis,
    pub shifts: Vec<PaneShift>,
    pub repaint_strip: PixelRect,
}

impl BlitPlan {
    /// Panes whose cached pane-buffer data shifts along `axis` and which
    /// therefore need `apply_blit_shift` + strip-fetch + a repaint pass.
    /// Cross-axis panes (TopLeft on either scroll; TopRight on row scroll;
    /// BottomLeft on column scroll) are stable across the blit and are
    /// excluded.
    pub fn shift_panes(&self) -> PaneRegionMask {
        match self.axis {
            Axis::Row => PaneRegionMask::EMPTY
                .with(PaneRegion::BottomLeft)
                .with(PaneRegion::BottomRight),
            Axis::Column => PaneRegionMask::EMPTY
                .with(PaneRegion::TopRight)
                .with(PaneRegion::BottomRight),
        }
    }

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
        pane: PaneRegion,
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
            shifts: vec![PaneShift {
                pane,
                src: make_rect(src_main, kept),
                dst: make_rect(dst_main, kept),
            }],
            repaint_strip: make_rect(strip_main_origin, strip_main_size),
        }
    }
}

/// Dispatch input for `Chrome::next` — which construction regime the
/// orchestrator selected for this frame. Exhaustive: adding a variant
/// breaks every regime arm at compile time.
#[derive(Clone, Copy)]
#[must_use = "FramePath dispatches Chrome::next; dropping it skips the chosen construction regime"]
pub enum FramePath<'a> {
    /// Full rebuild walk. `prev = Some` recycles slot Vec allocations;
    /// `prev = None` is the first-frame path.
    Fresh,
    /// Reuse prev's slot vecs verbatim; refresh per-frame state only
    /// (theme + pane_fingerprints rotation). `stale_panes` is caller-
    /// supplied so a `SlotsReuse` following a `Blit` doesn't inherit the
    /// blit's narrow strip mask and silently skip a content repaint.
    /// Requires `prev = Some`.
    SlotsReuse { stale_panes: PaneRegionMask },
    /// Blit fast-path. Scroll-axis slot vec rebuilt around the plan;
    /// cross-axis cloned from prev. Falls back to `Fresh` on the
    /// row_header_thickness digit-boundary case. Requires `prev = Some`.
    /// Borrows the plan from the orchestrator's `paint_viewport` stack
    /// frame so no per-frame clone is needed.
    Blit(&'a BlitPlan),
}

// Scroll-blit helpers
//
// `screen_for_blit` already disqualified anything that isn't a pure single-axis
// scroll. These helpers compute the canvas-pixel src/dst/strip rects and
// verify the kept band's row heights (col widths) match what the model
// still reports — that is the final qualification that the shifted pixels
// will land where the new chrome would paint them.

/// Row-header thickness implied by the last visible row's label — the value
/// the blit gate compares against `prev`. `scroll_rows` is the scrolled axis's
/// band (rebuilt or unchanged); an empty band falls back to the first scroll id.
fn blit_row_header_thickness(scroll_rows: &[RowSlot], frozen_rows_count: i32, new_top: i32) -> i32 {
    let last_visible_row = scroll_rows
        .last()
        .map(|s| s.row)
        .unwrap_or((frozen_rows_count + 1).max(new_top));
    measure_row_header_width(last_visible_row)
}

/// Build the next-frame Chrome by reusing as much of `prev` as the blit
/// plan guarantees is unchanged: the cross-axis slot Vec and both frozen
/// Vecs are *moved* out of `prev` (their heap allocation transfers to the
/// new frame — no per-scroll-frame clone), the scroll-axis kept band
/// carries forward heights/widths, only the strip touches the model.
///
/// Takes `prev` by value and returns `Err(prev)` — handing it back intact —
/// on the one cross-axis-affecting edge case (row_header_thickness changes
/// across a digit boundary) or any model anomaly, so the caller can fall
/// through to a full `Chrome::next`. Every `Err` return happens *before* the
/// first move out of `prev`, so the returned `prev` is always whole.
// `Chrome` is large and intentionally returned by value on *both* arms (the
// zero-copy give-back); boxing the `Err` wouldn't shrink the equally-large
// `Ok(Chrome)` and would add a heap alloc on the rare fallback.
#[allow(clippy::result_large_err)]
pub(super) fn try_blit_reuse(
    mut prev: Chrome,
    model: &dyn CanvasModel,
    canvas: CanvasSize,
    theme: &Rc<CanvasTheme>,
    plan: &BlitPlan,
) -> Result<Chrome, Chrome> {
    let Some(view) = model.get_selected_view() else {
        return Err(prev);
    };
    let frozen_rows_count = prev.pane_set.rows.frozen_count();
    let frozen_cols_count = prev.pane_set.cols.frozen_count();
    let new_top = scroll_first(frozen_rows_count, view.top_row);
    let new_left = scroll_first(frozen_cols_count, view.left_column);

    // Rebuild the scrolled axis band, gate on row-header thickness, *then* move
    // the unchanged cross-axis band out of `prev`. The gate runs before the
    // first `mem::take`, so every `Err` below hands `prev` back whole — the
    // invariant the caller's `Chrome::next` fallback relies on.
    //
    // Thickness gates cross-axis reuse: if the new last visible row label grew
    // (e.g. row 99 → 100), origin_x shifts and every col slot's `.left` is off,
    // so we fall back to a full rebuild. It reads the rebuilt rows band (row
    // scroll) or the still-unchanged cross-axis band (column scroll) — neither
    // taken yet. Once it passes, the cross-axis Vec is moved, not cloned.
    let (scroll_rows, scroll_cols, row_header_thickness) = match plan.axis {
        Axis::Row => {
            let rows = match prev
                .pane_set
                .rebuild_rows_for_row_scroll(model, new_top, canvas)
            {
                Some(rows) => rows,
                None => return Err(prev),
            };
            let thickness = blit_row_header_thickness(&rows, frozen_rows_count, new_top);
            if thickness != prev.row_header_thickness {
                return Err(prev);
            }
            let cols = std::mem::take(&mut prev.pane_set.cols.scroll);
            (rows, cols, thickness)
        }
        Axis::Column => {
            let cols = match prev
                .pane_set
                .rebuild_cols_for_col_scroll(model, new_left, canvas)
            {
                Some(cols) => cols,
                None => return Err(prev),
            };
            // Cross-axis rows band is unchanged across a column scroll; read it
            // (not taken yet) for the gate.
            let thickness =
                blit_row_header_thickness(&prev.pane_set.rows.scroll, frozen_rows_count, new_top);
            if thickness != prev.row_header_thickness {
                return Err(prev);
            }
            let rows = std::mem::take(&mut prev.pane_set.rows.scroll);
            (rows, cols, thickness)
        }
    };

    // The scroll-axis vec changed under the blit, so its labels must be
    // re-resolved; rebuilding both keeps the parallel-vec invariant trivially
    // correct. Shares resolution with Chrome::build via PaneSet::resolve_*.
    let sheet = prev.sheet;
    let row_header_labels =
        PaneSet::resolve_row_labels(model, sheet, &prev.pane_set.rows.frozen, &scroll_rows);
    let col_header_labels =
        PaneSet::resolve_col_labels(model, sheet, &prev.pane_set.cols.frozen, &scroll_cols);

    // Frozen bands are unchanged across a scroll, and their labels are now
    // resolved — move the Vecs out of `prev` (this is the last read of them).
    let pane_set = PaneSet {
        rows: AxisSlots {
            frozen: std::mem::take(&mut prev.pane_set.rows.frozen),
            scroll: scroll_rows,
            frozen_offset: prev.pane_set.rows.frozen_offset,
        },
        cols: AxisSlots {
            frozen: std::mem::take(&mut prev.pane_set.cols.frozen),
            scroll: scroll_cols,
            frozen_offset: prev.pane_set.cols.frozen_offset,
        },
        row_header_labels,
        col_header_labels,
    };

    let stale = plan.shift_panes();
    // Seed next frame's per-pane fingerprints: carry prev's forward, then
    // zero out the regions the blit touched so their next paint refetches
    // and re-fingerprints. Untouched regions short-circuit on the next
    // frame's cache check via fingerprint match.
    let prev_fps = prev.pane_fingerprints.get();
    let mut seeded_fps = prev_fps;
    for region in stale.regions() {
        seeded_fps[region as usize] = 0;
    }

    Ok(Chrome {
        sheet: prev.sheet,
        pane_set,
        row_header_thickness,
        col_header_thickness: prev.col_header_thickness,
        cell_origin: prev.cell_origin,
        canvas_size: canvas,
        theme: Rc::clone(theme),
        prev_pane_fingerprints: prev_fps,
        pane_fingerprints: Cell::new(seeded_fps),
        kind: FrameKindTag::Blitted,
        stale_panes: stale,
    })
}

pub(super) fn try_blit_rows(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_top: i32,
) -> Option<BlitPlan> {
    let pane_x = prev.pane_set.cols.frozen_offset;
    let pane_y = prev.pane_set.rows.frozen_offset;
    // pane_h is bounded by the canvas backing store extent, not by
    // `scroll_rows.last().top + height`. `fill_axis` pushes one row past
    // the canvas edge (the "overflow row") whose pixels were never on
    // canvas — using slot-bound pane_h here would send drawImage's
    // source rect past the backing store and the spec's proportional
    // source/dest clip would leave the bottom row stale.
    let pane_w = (prev.canvas_size.w.round() as i32) - pane_x;
    let pane_h = (prev.canvas_size.h.round() as i32) - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    // Frozen-cols band only exists when frozen_cols > 0; otherwise the
    // gap between cell_origin.x and pane_x is the 1-px chrome stroke
    // alone, NOT a paintable pane.
    let frozen_band_x = prev.cell_origin.x;
    let frozen_band_w = if prev.pane_set.cols.frozen_count() > 0 {
        pane_x - frozen_band_x
    } else {
        0
    };

    let (shift_px, dir) = prev
        .pane_set
        .probe_row_shift(model, sheet, new_top, pane_y, pane_h)?;

    let mut plan = BlitPlan::for_axis_scroll(
        Axis::Row,
        AxisRange {
            origin: pane_y,
            size: pane_h,
        },
        AxisRange {
            origin: pane_x,
            size: pane_w,
        },
        shift_px,
        dir,
        prev.canvas_size.h.round() as i32,
        PaneRegion::BottomRight,
    );
    if frozen_band_w > 0 {
        let sibling = plan.shifts[0].for_frozen_band(
            PaneRegion::BottomLeft,
            Axis::Row,
            frozen_band_x,
            frozen_band_w,
        );
        plan.shifts.push(sibling);
    }
    Some(plan)
}

pub(super) fn try_blit_cols(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_left: i32,
) -> Option<BlitPlan> {
    let pane_x = prev.pane_set.cols.frozen_offset;
    let pane_y = prev.pane_set.rows.frozen_offset;
    // pane_w is bounded by the canvas backing store extent, not by
    // `scroll_cols.last().left + width` — mirror of the comment in
    // try_blit_rows.
    let pane_w = (prev.canvas_size.w.round() as i32) - pane_x;
    let pane_h = (prev.canvas_size.h.round() as i32) - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    let frozen_band_y = prev.cell_origin.y;
    let frozen_band_h = if prev.pane_set.rows.frozen_count() > 0 {
        pane_y - frozen_band_y
    } else {
        0
    };

    let (shift_px, dir) = prev
        .pane_set
        .probe_col_shift(model, sheet, new_left, pane_x, pane_w)?;

    let mut plan = BlitPlan::for_axis_scroll(
        Axis::Column,
        AxisRange {
            origin: pane_x,
            size: pane_w,
        },
        AxisRange {
            origin: pane_y,
            size: pane_h,
        },
        shift_px,
        dir,
        prev.canvas_size.w.round() as i32,
        PaneRegion::BottomRight,
    );
    if frozen_band_h > 0 {
        let sibling = plan.shifts[0].for_frozen_band(
            PaneRegion::TopRight,
            Axis::Column,
            frozen_band_y,
            frozen_band_h,
        );
        plan.shifts.push(sibling);
    }
    Some(plan)
}
