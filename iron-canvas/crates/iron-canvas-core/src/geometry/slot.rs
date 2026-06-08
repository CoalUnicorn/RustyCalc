//! Per-axis visible slots on the painted frame.
//!
//! A slot carries the index, the absolute canvas coordinate of its leading
//! edge, and its extent. `PaneSet` holds four vecs (frozen/scrollable ×
//! row/column); every pixel↔cell query reads them directly, no prefix-sum
//! decoding.

use crate::CanvasModel;
use crate::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP};

#[derive(Clone, Copy, Debug)]
pub struct RowSlot {
    pub row: i32,
    /// Absolute canvas Y, not relative to any pane.
    pub top: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColSlot {
    pub col: i32,
    /// Absolute canvas X, not relative to any pane.
    pub left: i32,
    pub width: i32,
}

/// Axis-symmetric slot view. Lets every walk over `PaneSet`'s slot vecs share
/// one implementation regardless of axis. `end` defaults to `start + extent`,
/// giving generic callers the far edge (row bottom / col right) without an
/// axis-specific accessor.
pub trait AxisSlot: Sized {
    fn new(id: i32, start: i32, extent: i32) -> Self;
    fn id(&self) -> i32;
    fn start(&self) -> i32;
    fn extent(&self) -> i32;
    #[inline]
    fn end(&self) -> i32 {
        self.start() + self.extent()
    }
}

impl AxisSlot for RowSlot {
    #[inline]
    fn new(id: i32, start: i32, extent: i32) -> Self {
        RowSlot {
            row: id,
            top: start,
            height: extent,
        }
    }
    #[inline]
    fn id(&self) -> i32 {
        self.row
    }
    #[inline]
    fn start(&self) -> i32 {
        self.top
    }
    #[inline]
    fn extent(&self) -> i32 {
        self.height
    }
}

impl AxisSlot for ColSlot {
    #[inline]
    fn new(id: i32, start: i32, extent: i32) -> Self {
        ColSlot {
            col: id,
            left: start,
            width: extent,
        }
    }
    #[inline]
    fn id(&self) -> i32 {
        self.col
    }
    #[inline]
    fn start(&self) -> i32 {
        self.left
    }
    #[inline]
    fn extent(&self) -> i32 {
        self.width
    }
}

/// Walk an inclusive `range` starting at canvas coordinate `start`, push one
/// slot per id, break **post-push** when the slot's leading edge has reached
/// `max_cursor`. Returns the cursor past the last accepted slot (where the
/// next slot would sit) — used by the frozen pass to compute the band offset.
///
/// `max_cursor = i32::MAX` disables the break, used for the frozen band which
/// always paints regardless of viewport size. For the scroll band callers
/// pass `canvas_extent.ceil() as i32` — exactly equivalent to the original
/// `f64::from(cursor) >= canvas_extent` test, for any positive extent.
pub fn fill_axis<S: AxisSlot>(
    slots: &mut Vec<S>,
    range: std::ops::RangeInclusive<i32>,
    start: i32,
    max_cursor: i32,
    mut measure: impl FnMut(i32) -> i32,
) -> i32 {
    let mut cursor = start;
    for id in range {
        let extent = measure(id);
        slots.push(S::new(id, cursor, extent));
        if cursor >= max_cursor {
            break;
        }
        cursor += extent;
    }
    cursor
}

/// First non-frozen visible id along an axis: the larger of `frozen_count + 1`
/// (the id immediately past the frozen band) and the viewport's scrolled-to
/// id. Encodes the "scroll band starts where frozen ends or where the user
/// scrolled to, whichever is further" invariant — used by both `fill_axis`
/// callers and `Chrome::is_still_valid`.
#[inline]
pub fn scroll_first(frozen_count: i32, view_first: i32) -> i32 {
    (frozen_count + 1).max(view_first)
}

/// Locate a slot by `id` across the frozen+scroll pair. Frozen ids index
/// from 1; scroll ids index from the first slot's id (the scroll band starts
/// past whatever has been scrolled off-screen).
pub fn slot_at<'a, S: AxisSlot>(frozen: &'a [S], scroll: &'a [S], id: i32) -> Option<&'a S> {
    let frozen_n = frozen.len() as i32;
    if id <= frozen_n {
        frozen.get((id - 1) as usize)
    } else {
        let first = scroll.first()?.id();
        scroll.get((id - first) as usize)
    }
}

/// First scroll-band id, or `1` when the band is empty (matches the fresh-
/// frame default before any scroll happens).
pub fn top_id<S: AxisSlot>(scroll: &[S]) -> i32 {
    scroll.first().map(|s| s.id()).unwrap_or(1)
}

/// Last scroll-band id visible, falling back to [`top_id`] when the band is
/// empty so callers always get a valid id.
pub fn last_visible_id<S: AxisSlot>(scroll: &[S]) -> i32 {
    scroll
        .last()
        .map(|s| s.id())
        .unwrap_or_else(|| top_id(scroll))
}

/// Linear-scan frozen-then-scroll for the slot covering `pixel`. Used to map
/// a canvas Y/X back to a row/column.
pub fn pixel_to_id<S: AxisSlot>(frozen: &[S], scroll: &[S], pixel: i32) -> Option<i32> {
    for s in frozen.iter().chain(scroll.iter()) {
        if pixel >= s.start() && pixel < s.end() {
            return Some(s.id());
        }
    }
    None
}

/// Snap `pixel` to a slot's trailing edge when it falls within `tolerance`.
/// Breaks once a slot's end is past the tolerance band — slot vecs are
/// monotonic so no later slot can match.
///
/// Tie-break: when two adjacent edges both fall within `tolerance` (thin slots
/// at high zoom-out, or a generous `tolerance`), the **first** in iteration
/// order wins — not the nearest. Correct for normal zoom and tolerances; a
/// nearest-edge scan would be needed if that stops holding.
///
/// The frozen-then-scroll chain is treated as one ascending pixel space, so
/// the post-`tolerance` break in the frozen leg must not cut off a still-
/// reachable scroll slot. That holds only while the scroll band starts at or
/// after the frozen band ends; the `debug_assert!` guards that seam invariant.
pub fn boundary_at<S: AxisSlot>(
    frozen: &[S],
    scroll: &[S],
    pixel: i32,
    tolerance: i32,
) -> Option<i32> {
    debug_assert!(
        match (frozen.last(), scroll.first()) {
            (Some(f), Some(s)) => s.start() >= f.end(),
            _ => true,
        },
        "boundary_at seam: scroll band must start at or after the frozen band ends"
    );
    for s in frozen.iter().chain(scroll.iter()) {
        if (s.end() - pixel).abs() <= tolerance {
            return Some(s.id());
        }
        if s.end() > pixel + tolerance {
            break;
        }
    }
    None
}

/// Frozen + scroll slot pair for one axis, plus the canvas coordinate where
/// the scroll band begins (past the frozen band and its separator — `y` for
/// rows, `x` for cols). Owns the axis-generic queries `PaneSet` previously
/// delegated through the free fns above *twice*, once per axis. `PaneSet`
/// composes one `AxisSlots` per axis instead of four flat vecs.
#[derive(Clone, Debug)]
pub struct AxisSlots<S: AxisSlot> {
    pub frozen: Vec<S>,
    pub scroll: Vec<S>,
    pub frozen_offset: i32,
}

impl<S: AxisSlot> AxisSlots<S> {
    #[inline]
    pub fn frozen_count(&self) -> i32 {
        self.frozen.len() as i32
    }

    #[inline]
    pub fn slot(&self, id: i32) -> Option<&S> {
        slot_at(&self.frozen, &self.scroll, id)
    }

    /// First scroll-band id (top row / left column), or 1 on an empty band.
    #[inline]
    pub fn top(&self) -> i32 {
        top_id(&self.scroll)
    }

    #[inline]
    pub fn last_visible(&self) -> i32 {
        last_visible_id(&self.scroll)
    }

    #[inline]
    pub fn pixel_to_id(&self, pixel: i32) -> Option<i32> {
        pixel_to_id(&self.frozen, &self.scroll, pixel)
    }

    #[inline]
    pub fn boundary_at(&self, pixel: i32, tolerance: i32) -> Option<i32> {
        boundary_at(&self.frozen, &self.scroll, pixel, tolerance)
    }

    /// Leading-edge coordinate of `id`'s slot (row top / col left); 0 off-frame.
    #[inline]
    pub fn to_pixel(&self, id: i32) -> i32 {
        self.slot(id).map(|s| s.start()).unwrap_or(0)
    }

    /// Extent of `id`'s slot (row height / col width); 0 off-frame.
    #[inline]
    pub fn extent_at(&self, id: i32) -> i32 {
        self.slot(id).map(|s| s.extent()).unwrap_or(0)
    }

    #[inline]
    pub fn contains(&self, id: i32) -> bool {
        self.slot(id).is_some()
    }

    /// Populate `frozen` + `scroll` and record `frozen_offset`. Walks the
    /// frozen band first (always painted — `i32::MAX` disables the viewport
    /// break), notes where the scroll band starts, then walks the scroll band
    /// from `view_first` to `last`, breaking at the canvas edge.
    ///
    /// `frozen_offset` is the seam invariant `boundary_at` relies on: the
    /// scroll band must begin at or after the frozen band ends.
    // Mirrors `fill_axis`'s walker shape — each arg is an independent axis
    // input (counts, origin, viewport bound, measure); bundling them would
    // only add an indirection struct.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin: i32,
        view_first: i32,
        last: i32,
        canvas_extent: i32,
        mut measure: impl FnMut(&dyn CanvasModel, i32) -> i32,
    ) {
        self.frozen.reserve(frozen_count as usize);
        let after_frozen = fill_axis(&mut self.frozen, 1..=frozen_count, origin, i32::MAX, |id| {
            measure(model, id)
        });
        self.frozen_offset = after_frozen + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let _ = fill_axis(
            &mut self.scroll,
            scroll_first(frozen_count, view_first)..=last,
            self.frozen_offset,
            canvas_extent,
            |id| measure(model, id),
        );
    }
}

pub fn row_height(model: &dyn CanvasModel, row: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_row_height(sheet, row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
        .round() as i32
}

pub fn col_width(model: &dyn CanvasModel, col: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_column_width(sheet, col)
        .unwrap_or(DEFAULT_COL_WIDTH)
        .round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two frozen rows (ids 1,2) at y=0,20 then a scroll band starting at the
    /// recorded `frozen_offset`, ids 5.. (the user scrolled past 3,4).
    fn rows() -> AxisSlots<RowSlot> {
        AxisSlots {
            frozen: vec![RowSlot::new(1, 0, 20), RowSlot::new(2, 20, 20)],
            scroll: vec![RowSlot::new(5, 48, 20), RowSlot::new(6, 68, 20)],
            frozen_offset: 48,
        }
    }

    #[test]
    fn slot_lookup_spans_frozen_then_scroll() {
        let r = rows();
        assert_eq!(r.frozen_count(), 2);
        assert_eq!(r.slot(2).map(|s| s.start()), Some(20)); // frozen
        assert_eq!(r.slot(5).map(|s| s.start()), Some(48)); // scroll, id-indexed
        assert!(r.slot(3).is_none()); // scrolled off, between the bands
    }

    #[test]
    fn pixel_and_boundary_walk_one_ascending_space() {
        let r = rows();
        assert_eq!(r.pixel_to_id(25), Some(2)); // inside frozen row 2
        assert_eq!(r.pixel_to_id(50), Some(5)); // inside first scroll row
        assert_eq!(r.boundary_at(40, 3), Some(2)); // frozen row 2's trailing edge
        assert_eq!(r.boundary_at(88, 3), Some(6)); // last scroll row's edge
    }

    #[test]
    fn pixel_accessors_resolve_edges_and_extents() {
        let r = rows();
        assert_eq!(r.top(), 5);
        assert_eq!(r.last_visible(), 6);
        assert_eq!(r.to_pixel(6), 68);
        assert_eq!(r.extent_at(6), 20);
        assert_eq!(r.to_pixel(99), 0); // off-frame falls back to 0
        assert!(r.contains(1) && !r.contains(99));
    }
}
