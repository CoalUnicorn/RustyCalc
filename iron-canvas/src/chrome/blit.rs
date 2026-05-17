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

use crate::geometry::constants::{HEADER_OFFSET, LAST_COLUMN, LAST_ROW};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point};
use crate::geometry::slot::{col_width, fill_axis, row_height, scroll_first, ColSlot, RowSlot};
use crate::theme::CanvasTheme;
use crate::{CanvasModel, CanvasSize};

use super::{
    measure_row_header_width, ActiveCellSnapshot, Chrome, FrameKindTag, PaneRegion, PaneRegionMask,
    PaneSet,
};

/// One pane's contribution to a scroll-blit. A row-axis scroll emits a
/// `PaneShift` for `BottomRight` and, when `frozen_cols > 0`, another
/// for `BottomLeft`; a column-axis scroll mirrors with `TopRight` when
/// `frozen_rows > 0`. `src` and `dst` have identical `width`/`height`;
/// only the offset along the scroll axis differs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PaneShift {
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
    fn sibling(
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
pub(crate) struct BlitPlan {
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
    pub(crate) fn shift_panes(&self) -> PaneRegionMask {
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
    /// `HEADER_OFFSET + 1` shifts src/dst/strip one pixel past the cross-axis
    /// header / grid border line so the 1-px chrome stroke is never folded
    /// into the kept band by the blit (`render_headers_base` repaints it on
    /// top each frame, which would otherwise double its density).
    fn for_axis_scroll(
        scroll_axis: Axis,
        main_pane: AxisRange,
        cross_pane: AxisRange,
        shift_px: i32,
        dir: ShiftDir,
        canvas_main: i32,
        pane: PaneRegion,
    ) -> BlitPlan {
        let chrome_inset = HEADER_OFFSET + 1;
        let kept = main_pane.size - shift_px;
        let cross = AxisRange {
            origin: cross_pane.origin + chrome_inset,
            size: cross_pane.size - chrome_inset,
        };
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
/// orchestrator selected for this frame. Replaces the trio of
/// `Chrome::next` dispatch over `FramePath::{Fresh, SlotsReuse, Blit}`
/// and the manual `match prev.kind` exhaustiveness checks at dispatch
/// sites. Adding a variant breaks every regime arm at compile time.
#[derive(Clone, Copy)]
pub(crate) enum FramePath<'a> {
    /// Full rebuild walk. `prev = Some` recycles slot Vec allocations;
    /// `prev = None` is the first-frame path.
    Fresh,
    /// Reuse prev's slot vecs verbatim; refresh per-frame state only
    /// (theme + pane_fingerprints rotation). Requires `prev = Some`.
    SlotsReuse,
    /// Blit fast-path. Scroll-axis slot vec rebuilt around the plan;
    /// cross-axis cloned from prev. Falls back to `Fresh` on the
    /// row_header_thickness digit-boundary case. Requires `prev = Some`.
    /// Borrows the plan from the orchestrator's `paint_viewport` stack
    /// frame so no per-frame clone is needed.
    Blit(&'a BlitPlan),
}

impl Chrome {
    /// Probe whether a pure row-axis scroll from this Chrome to `new_top`
    /// qualifies for a blit: detect direction, measure the shift, and
    /// verify that the kept band's row heights still match the model.
    /// Returns `None` if the geometry would not survive a blit; the caller
    /// then falls through to a full rebuild.
    fn probe_row_shift(
        &self,
        model: &dyn CanvasModel,
        sheet: u32,
        new_top: i32,
        pane_y: i32,
        pane_h: i32,
    ) -> Option<(i32, ShiftDir)> {
        let prev_rows = &self.pane_set.scroll_rows;
        if prev_rows.is_empty() {
            return None;
        }
        let old_top = self.pane_set.top_row();
        if new_top > old_top {
            let drows = (new_top - old_top) as usize;
            if drows >= prev_rows.len() {
                return None;
            }
            let leaving_h = prev_rows[drows].top - pane_y;
            if leaving_h <= 0 || leaving_h >= pane_h {
                return None;
            }
            if !overlap_row_heights_match(model, sheet, &prev_rows[drows..]) {
                return None;
            }
            Some((leaving_h, ShiftDir::Forward))
        } else {
            let drows = (old_top - new_top) as usize;
            let mut strip_h: i32 = 0;
            for i in 0..drows {
                let h = model
                    .get_row_height(sheet, new_top + i as i32)
                    .unwrap_or(0.0)
                    .round() as i32;
                strip_h = strip_h.saturating_add(h);
            }
            if strip_h <= 0 || strip_h >= pane_h {
                return None;
            }
            if !overlap_row_heights_match(model, sheet, prev_rows) {
                return None;
            }
            Some((strip_h, ShiftDir::Backward))
        }
    }

    /// Column-axis mirror of `probe_row_shift`.
    fn probe_col_shift(
        &self,
        model: &dyn CanvasModel,
        sheet: u32,
        new_left: i32,
        pane_x: i32,
        pane_w: i32,
    ) -> Option<(i32, ShiftDir)> {
        let prev_cols = &self.pane_set.scroll_cols;
        if prev_cols.is_empty() {
            return None;
        }
        let old_left = self.pane_set.left_column();
        if new_left > old_left {
            let dcols = (new_left - old_left) as usize;
            if dcols >= prev_cols.len() {
                return None;
            }
            let leaving_w = prev_cols[dcols].left - pane_x;
            if leaving_w <= 0 || leaving_w >= pane_w {
                return None;
            }
            if !overlap_col_widths_match(model, sheet, &prev_cols[dcols..]) {
                return None;
            }
            Some((leaving_w, ShiftDir::Forward))
        } else {
            let dcols = (old_left - new_left) as usize;
            let mut strip_w: i32 = 0;
            for i in 0..dcols {
                let w = model
                    .get_column_width(sheet, new_left + i as i32)
                    .unwrap_or(0.0)
                    .round() as i32;
                strip_w = strip_w.saturating_add(w);
            }
            if strip_w <= 0 || strip_w >= pane_w {
                return None;
            }
            if !overlap_col_widths_match(model, sheet, prev_cols) {
                return None;
            }
            Some((strip_w, ShiftDir::Backward))
        }
    }

    /// Seed the next frame's pane fingerprints: regions touched by the
    /// blit (`stale`) start at zero and force a repaint; untouched regions
    /// carry forward verbatim so the cache check in `is_still_valid`
    /// short-circuits on them.
    fn seed_next_pane_fingerprints(&self, stale: PaneRegionMask) -> [u64; 4] {
        let prev_fps = self.pane_fingerprints.get();
        let mut seeded = [0u64; 4];
        for region in [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ] {
            if !stale.contains_region(region) {
                let idx = region as usize;
                seeded[idx] = prev_fps[idx];
            }
        }
        seeded
    }
}

// Scroll-blit helpers
//
// `screen_for_blit` already disqualified anything that isn't a pure single-axis
// scroll. These helpers compute the canvas-pixel src/dst/strip rects and
// verify the kept band's row heights (col widths) match what the model
// still reports — that is the final qualification that the shifted pixels
// will land where the new chrome would paint them.

/// Build the next-frame Chrome by reusing as much of `prev` as the blit
/// plan guarantees is unchanged: cross-axis slot vec is cloned verbatim,
/// scroll-axis kept band carries forward heights/widths, only the strip
/// touches the model. Returns `None` on the one cross-axis-affecting
/// edge case (row_header_thickness changes across a digit boundary) or
/// any model anomaly — the caller falls through to a full `Chrome::next`.
pub(super) fn try_blit_reuse(
    prev: &Chrome,
    model: &dyn CanvasModel,
    canvas: CanvasSize,
    theme: &CanvasTheme,
    plan: &BlitPlan,
) -> Option<Chrome> {
    let view = model.get_selected_view()?;
    let frozen_rows_count = prev.pane_set.frozen_rows_count();
    let frozen_cols_count = prev.pane_set.frozen_cols_count();
    let new_top = scroll_first(frozen_rows_count, view.top_row);
    let new_left = scroll_first(frozen_cols_count, view.left_column);

    let (scroll_rows, scroll_cols) = match plan.axis {
        Axis::Row => (
            rebuild_rows_for_row_scroll(prev, model, new_top, canvas)?,
            prev.pane_set.scroll_cols.clone(),
        ),
        Axis::Column => (
            prev.pane_set.scroll_rows.clone(),
            rebuild_cols_for_col_scroll(prev, model, new_left, canvas)?,
        ),
    };

    // Row header thickness gates cross-axis reuse. If the new last
    // visible row label grew (e.g. row 99 → 100), origin_x shifts and
    // every col slot's `.left` is off — fall back to full rebuild.
    let last_visible_row = scroll_rows
        .last()
        .map(|s| s.row)
        .unwrap_or((frozen_rows_count + 1).max(new_top));
    let row_header_thickness = measure_row_header_width(last_visible_row);
    if row_header_thickness != prev.row_header_thickness {
        return None;
    }

    let pane_set = PaneSet {
        frozen_rows: prev.pane_set.frozen_rows.clone(),
        scroll_rows,
        frozen_offset_y: prev.pane_set.frozen_offset_y,
        frozen_cols: prev.pane_set.frozen_cols.clone(),
        scroll_cols,
        frozen_offset_x: prev.pane_set.frozen_offset_x,
    };

    let stale = plan.shift_panes();
    let seeded_fps = prev.seed_next_pane_fingerprints(stale);
    let active_cell = ActiveCellSnapshot::capture(model, prev.sheet, view.row, view.column);

    Some(Chrome {
        sheet: prev.sheet,
        pane_set,
        row_header_thickness,
        col_header_thickness: prev.col_header_thickness,
        cell_origin: prev.cell_origin,
        selection_range: view.selection,
        active_cell,
        canvas_size: canvas,
        theme: theme.clone(),
        prev_pane_fingerprints: prev.pane_fingerprints.get(),
        pane_fingerprints: Cell::new(seeded_fps),
        kind: FrameKindTag::Blitted,
        stale_panes: stale,
    })
}

/// Build new `scroll_rows` for a pure row scroll: keep the surviving
/// slots with their `.row` and `.height` intact and only `.top` shifted,
/// then `fill_axis` the strip from the model. Returns `None` if `prev`'s
/// data isn't enough to cover the kept band — screen_for_blit guards make this
/// path unreachable, but cheap defensiveness keeps the fallback open.
fn rebuild_rows_for_row_scroll(
    prev: &Chrome,
    model: &dyn CanvasModel,
    new_top: i32,
    canvas: CanvasSize,
) -> Option<Vec<RowSlot>> {
    let prev_rows = &prev.pane_set.scroll_rows;
    let frozen_offset_y = prev.pane_set.frozen_offset_y;
    let max_cursor = canvas.h.ceil() as i32;
    let delta = new_top - prev.pane_set.top_row();
    if delta == 0 {
        return None;
    }
    let drows = delta.unsigned_abs() as usize;
    if drows >= prev_rows.len() {
        return None;
    }

    let mut new_rows: Vec<RowSlot> = Vec::with_capacity(prev_rows.len() + drows);

    if delta > 0 {
        // Scroll DOWN — drop leading `drows` rows; strip is appended below
        // by the topup fill_axis.
        let leaving_h = prev_rows[drows].top - frozen_offset_y;
        for slot in &prev_rows[drows..] {
            new_rows.push(RowSlot {
                row: slot.row,
                top: slot.top - leaving_h,
                height: slot.height,
            });
        }
    } else {
        // Scroll UP — strip enters at top, kept band shifts down by strip_h.
        let strip_last = prev_rows[0].row - 1;
        if new_top > strip_last {
            return None;
        }
        let strip_cursor_end = fill_axis(
            &mut new_rows,
            new_top..=strip_last,
            frozen_offset_y,
            i32::MAX,
            |r| row_height(model, r),
        );
        let strip_h = strip_cursor_end - frozen_offset_y;
        for slot in &prev_rows[..prev_rows.len() - drows] {
            new_rows.push(RowSlot {
                row: slot.row,
                top: slot.top + strip_h,
                height: slot.height,
            });
        }
    }

    // Slot vec invariant (matches `fill_axis`): at most one row may have
    // `top >= max_cursor` — the overflow row included for partial-edge
    // rendering. After the shift, the inherited overflow plus newly-
    // pushed ones can leave two overflow rows; trim back to one. Then,
    // if no overflow row exists yet, fill_axis pushes exactly one.
    while new_rows.len() >= 2
        && new_rows[new_rows.len() - 1].top >= max_cursor
        && new_rows[new_rows.len() - 2].top >= max_cursor
    {
        new_rows.pop();
    }
    if new_rows.last().is_some_and(|s| s.top < max_cursor) {
        let cursor = new_rows
            .last()
            .map(|s| s.top + s.height)
            .unwrap_or(frozen_offset_y);
        let next_row = new_rows.last().map(|s| s.row + 1).unwrap_or(new_top);
        let _ = fill_axis(
            &mut new_rows,
            next_row..=LAST_ROW,
            cursor,
            max_cursor,
            |r| row_height(model, r),
        );
    }

    Some(new_rows)
}

/// Column-scroll mirror of `rebuild_rows_for_row_scroll`.
fn rebuild_cols_for_col_scroll(
    prev: &Chrome,
    model: &dyn CanvasModel,
    new_left: i32,
    canvas: CanvasSize,
) -> Option<Vec<ColSlot>> {
    let prev_cols = &prev.pane_set.scroll_cols;
    let frozen_offset_x = prev.pane_set.frozen_offset_x;
    let max_cursor = canvas.w.ceil() as i32;
    let delta = new_left - prev.pane_set.left_column();
    if delta == 0 {
        return None;
    }
    let dcols = delta.unsigned_abs() as usize;
    if dcols >= prev_cols.len() {
        return None;
    }

    let mut new_cols: Vec<ColSlot> = Vec::with_capacity(prev_cols.len() + dcols);

    if delta > 0 {
        let leaving_w = prev_cols[dcols].left - frozen_offset_x;
        for slot in &prev_cols[dcols..] {
            new_cols.push(ColSlot {
                col: slot.col,
                left: slot.left - leaving_w,
                width: slot.width,
            });
        }
    } else {
        let strip_last = prev_cols[0].col - 1;
        if new_left > strip_last {
            return None;
        }
        let strip_cursor_end = fill_axis(
            &mut new_cols,
            new_left..=strip_last,
            frozen_offset_x,
            i32::MAX,
            |c| col_width(model, c),
        );
        let strip_w = strip_cursor_end - frozen_offset_x;
        for slot in &prev_cols[..prev_cols.len() - dcols] {
            new_cols.push(ColSlot {
                col: slot.col,
                left: slot.left + strip_w,
                width: slot.width,
            });
        }
    }

    while new_cols.len() >= 2
        && new_cols[new_cols.len() - 1].left >= max_cursor
        && new_cols[new_cols.len() - 2].left >= max_cursor
    {
        new_cols.pop();
    }
    if new_cols.last().is_some_and(|s| s.left < max_cursor) {
        let cursor = new_cols
            .last()
            .map(|s| s.left + s.width)
            .unwrap_or(frozen_offset_x);
        let next_col = new_cols.last().map(|s| s.col + 1).unwrap_or(new_left);
        let _ = fill_axis(
            &mut new_cols,
            next_col..=LAST_COLUMN,
            cursor,
            max_cursor,
            |c| col_width(model, c),
        );
    }

    Some(new_cols)
}

pub(super) fn try_blit_rows(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_top: i32,
) -> Option<BlitPlan> {
    let pane_x = prev.pane_set.frozen_offset_x;
    let pane_y = prev.pane_set.frozen_offset_y;
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
    // gap between row_header_thickness and pane_x is the 1-px chrome
    // stroke alone, NOT a paintable pane.
    let frozen_band_x = prev.row_header_thickness + HEADER_OFFSET;
    let frozen_band_w = if prev.pane_set.frozen_cols_count() > 0 {
        pane_x - frozen_band_x
    } else {
        0
    };

    let (shift_px, dir) = prev.probe_row_shift(model, sheet, new_top, pane_y, pane_h)?;

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
        let sibling = plan.shifts[0].sibling(
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
    let pane_x = prev.pane_set.frozen_offset_x;
    let pane_y = prev.pane_set.frozen_offset_y;
    // pane_w is bounded by the canvas backing store extent, not by
    // `scroll_cols.last().left + width` — mirror of the comment in
    // try_blit_rows.
    let pane_w = (prev.canvas_size.w.round() as i32) - pane_x;
    let pane_h = (prev.canvas_size.h.round() as i32) - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    let frozen_band_y = prev.col_header_thickness + HEADER_OFFSET;
    let frozen_band_h = if prev.pane_set.frozen_rows_count() > 0 {
        pane_y - frozen_band_y
    } else {
        0
    };

    let (shift_px, dir) = prev.probe_col_shift(model, sheet, new_left, pane_x, pane_w)?;

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
        let sibling = plan.shifts[0].sibling(
            PaneRegion::TopRight,
            Axis::Column,
            frozen_band_y,
            frozen_band_h,
        );
        plan.shifts.push(sibling);
    }
    Some(plan)
}

fn overlap_row_heights_match(model: &dyn CanvasModel, sheet: u32, overlap: &[RowSlot]) -> bool {
    overlap.iter().all(|s| {
        model
            .get_row_height(sheet, s.row)
            .map(|h| h.round() as i32 == s.height)
            .unwrap_or(false)
    })
}

fn overlap_col_widths_match(model: &dyn CanvasModel, sheet: u32, overlap: &[ColSlot]) -> bool {
    overlap.iter().all(|s| {
        model
            .get_column_width(sheet, s.col)
            .map(|w| w.round() as i32 == s.width)
            .unwrap_or(false)
    })
}

#[derive(Copy, Clone)]
enum ShiftDir {
    /// Viewport scrolls forward along the axis (new_top > old_top, or
    /// new_left > old_left). Kept band shifts toward smaller coordinate;
    /// repaint strip lands at the far edge.
    Forward,
    /// Viewport scrolls backward. Kept band shifts toward larger coordinate;
    /// repaint strip lands at the near edge.
    Backward,
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn console_log(s: &str);
}

/// One-line debug summary emitted to the browser console after every
/// scroll-blit dispatch. Wasm-only — native builds skip the call site
/// behind `#[cfg(target_arch = "wasm32")]`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn log(&self, prev_top: i32, prev_pane_range: Option<crate::RCRange>, frame: &Chrome) {
    let axis = match self.axis {
        Axis::Row => "Row",
        Axis::Column => "Col",
    };
    let new_top = frame.pane_set_top_row_debug();
    let new_last = frame.pane_set_last_row_debug();
    let scroll_rows_len = frame.scroll_rows_len_debug();
    let cached = match prev_pane_range {
        Some(r) => format!("rows {}..={}", r.r1, r.r2),
        None => "<none>".to_string(),
    };
    let Some(primary) = self.shifts.first() else {
        return;
    };
    let msg = format!(
        "[blit] axis={} prev_top={} new_top={} new_last={} cache.range={} scroll_rows.len()={} shifts={} src=(y={}, h={}) dst=(y={}, h={}) strip=(y={}, h={})",
        axis,
        prev_top,
        new_top,
        new_last,
        cached,
        scroll_rows_len,
        self.shifts.len(),
        primary.src.top_left.y,
        primary.src.height,
        primary.dst.top_left.y,
        primary.dst.height,
        self.repaint_strip.top_left.y,
        self.repaint_strip.height,
    );
    console_log(&msg);
}
