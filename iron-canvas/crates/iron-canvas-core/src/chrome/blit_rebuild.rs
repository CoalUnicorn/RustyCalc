//! Single-axis blit machinery: qualification probes (kept-band extent
//! verification + strip-size compute) and slot-Vec rebuilds for the
//! scrolled axis. `Chrome::screen_for_blit` calls the probes; the
//! `FramePath::Blit` arm of `Chrome::next` calls the rebuilds.
//!
//! Lives next to `chrome/blit.rs`. Splits off `chrome/pane_set.rs` so
//! the latter only carries pure-axis geometry.

use crate::geometry::constants::{LAST_COLUMN, LAST_ROW};
use crate::geometry::slot::{AxisSlot, ColSlot, RowSlot, col_width, fill_axis, row_height};
use crate::{CanvasModel, CanvasSize};

use super::pane_set::PaneSet;

/// Direction of a single-axis viewport shift between `prev.pane_set.top_row()`
/// (or `left_column()`) and the new effective scroll start.
///
/// `Forward`: new > old; kept band moves toward smaller coordinate, strip
/// lands at the far edge of the pane.
/// `Backward`: new < old; kept band moves toward larger coordinate, strip
/// lands at the near edge.
#[derive(Copy, Clone)]
pub enum ShiftDir {
    Forward,
    Backward,
}

/// Verify every slot's cached extent still matches what the model reports.
/// `measure(id)` returns `None` when the model has no data for that id;
/// `None` rejects the match (the kept band would no longer survive the blit).
fn overlaps_match<S: AxisSlot>(slots: &[S], measure: impl Fn(i32) -> Option<i32>) -> bool {
    slots.iter().all(|s| measure(s.id()) == Some(s.extent()))
}

/// Generic single-axis blit-probe. Returns `Some((shift_px, dir))` when
/// the kept band would still match the model after shifting from
/// `prev_slots[0].id()` to `new_first_idx`; `None` when geometry,
/// extent, or overlap rejects the shift. `pane_origin` / `pane_extent`
/// are the scroll pane's canvas-pixel bounds along the scroll axis.
/// `measure(id)` is the model accessor; `None` returns reject overlap
/// (rebuild treats `None` as zero, but probe rejects).
fn probe_axis_shift<S: AxisSlot>(
    prev_slots: &[S],
    new_first_idx: i32,
    pane_origin: i32,
    pane_extent: i32,
    measure: impl Fn(i32) -> Option<i32>,
) -> Option<(i32, ShiftDir)> {
    let first = prev_slots.first()?;
    let old_first_idx = first.id();
    if new_first_idx > old_first_idx {
        let d = (new_first_idx - old_first_idx) as usize;
        if d >= prev_slots.len() {
            return None;
        }
        let leaving = prev_slots[d].start() - pane_origin;
        if leaving <= 0 || leaving >= pane_extent {
            return None;
        }
        if !overlaps_match(&prev_slots[d..], &measure) {
            return None;
        }
        Some((leaving, ShiftDir::Forward))
    } else {
        let d = (old_first_idx - new_first_idx) as usize;
        let strip: i32 = (0..d)
            .map(|i| measure(new_first_idx + i as i32).unwrap_or(0))
            .fold(0, i32::saturating_add);
        if strip <= 0 || strip >= pane_extent {
            return None;
        }
        if !overlaps_match(prev_slots, &measure) {
            return None;
        }
        Some((strip, ShiftDir::Backward))
    }
}

/// Rebuild a scroll-axis slot vec for a single-axis blit. The kept
/// band's slots carry forward verbatim with only their `start()`
/// shifted; the strip (newly-revealed band) is walked from the model
/// via `fill_axis`. Trims trailing overflow slots so at most one slot
/// straddles `max_cursor` (matches `fill_axis`'s invariant), then
/// tops up if the kept-band shift left the last slot inside the
/// canvas edge.
///
/// Returns `None` when `prev_slots` is empty, `delta == 0`, the shift
/// exceeds the kept-band capacity, or the backward strip would extend
/// past the previous first slot.
fn rebuild_axis_slots<S: AxisSlot>(
    prev_slots: &[S],
    frozen_offset: i32,
    max_cursor: i32,
    new_first_idx: i32,
    last_idx_limit: i32,
    measure: impl Fn(i32) -> i32,
) -> Option<Vec<S>> {
    let first = prev_slots.first()?;
    let old_first_idx = first.id();
    let delta = new_first_idx - old_first_idx;
    if delta == 0 {
        return None;
    }
    let d = delta.unsigned_abs() as usize;
    if d >= prev_slots.len() {
        return None;
    }

    let mut new_slots: Vec<S> = Vec::with_capacity(prev_slots.len() + d);

    if delta > 0 {
        // Forward: drop leading d slots; top up at the far edge below.
        let leaving = prev_slots[d].start() - frozen_offset;
        for slot in &prev_slots[d..] {
            new_slots.push(S::new(slot.id(), slot.start() - leaving, slot.extent()));
        }
    } else {
        // Backward: strip enters at the near edge; kept band shifts by strip_size.
        let strip_last = old_first_idx - 1;
        if new_first_idx > strip_last {
            return None;
        }
        let strip_cursor_end = fill_axis(
            &mut new_slots,
            new_first_idx..=strip_last,
            frozen_offset,
            i32::MAX,
            &measure,
        );
        let strip_size = strip_cursor_end - frozen_offset;
        for slot in &prev_slots[..prev_slots.len() - d] {
            new_slots.push(S::new(slot.id(), slot.start() + strip_size, slot.extent()));
        }
    }

    // Trim back to at most one overflow slot past max_cursor.
    while new_slots.len() >= 2
        && new_slots[new_slots.len() - 1].start() >= max_cursor
        && new_slots[new_slots.len() - 2].start() >= max_cursor
    {
        new_slots.pop();
    }
    // Top up if the kept-band shift left the last slot short of the edge.
    if new_slots.last().is_some_and(|s| s.start() < max_cursor) {
        let cursor = new_slots.last().map(|s| s.end()).unwrap_or(frozen_offset);
        let next_id = new_slots
            .last()
            .map(|s| s.id() + 1)
            .unwrap_or(new_first_idx);
        let _ = fill_axis(
            &mut new_slots,
            next_id..=last_idx_limit,
            cursor,
            max_cursor,
            &measure,
        );
    }

    Some(new_slots)
}

impl PaneSet {
    pub fn probe_row_shift(
        &self,
        model: &dyn CanvasModel,
        sheet: u32,
        new_top: i32,
        pane_y: i32,
        pane_h: i32,
    ) -> Option<(i32, ShiftDir)> {
        probe_axis_shift(&self.rows.scroll, new_top, pane_y, pane_h, |r| {
            model.get_row_height(sheet, r).map(|h| h.round() as i32)
        })
    }

    pub fn probe_col_shift(
        &self,
        model: &dyn CanvasModel,
        sheet: u32,
        new_left: i32,
        pane_x: i32,
        pane_w: i32,
    ) -> Option<(i32, ShiftDir)> {
        probe_axis_shift(&self.cols.scroll, new_left, pane_x, pane_w, |c| {
            model.get_column_width(sheet, c).map(|w| w.round() as i32)
        })
    }

    pub fn rebuild_rows_for_row_scroll(
        &self,
        model: &dyn CanvasModel,
        new_top: i32,
        canvas: CanvasSize,
    ) -> Option<Vec<RowSlot>> {
        rebuild_axis_slots(
            &self.rows.scroll,
            self.rows.frozen_offset,
            canvas.h.ceil() as i32,
            new_top,
            LAST_ROW,
            |r| row_height(model, r),
        )
    }

    pub fn rebuild_cols_for_col_scroll(
        &self,
        model: &dyn CanvasModel,
        new_left: i32,
        canvas: CanvasSize,
    ) -> Option<Vec<ColSlot>> {
        rebuild_axis_slots(
            &self.cols.scroll,
            self.cols.frozen_offset,
            canvas.w.ceil() as i32,
            new_left,
            LAST_COLUMN,
            |c| col_width(model, c),
        )
    }
}
