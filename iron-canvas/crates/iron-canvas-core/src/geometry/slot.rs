//! Per-axis visible slots on the painted frame.
//!
//! A slot carries the index, the absolute canvas coordinate of its leading
//! edge, and its extent. `PaneSet` holds four vecs (frozen/scrollable ×
//! row/column); every pixel↔cell query reads them directly, no prefix-sum
//! decoding.

use crate::CanvasModel;
use crate::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};

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
/// one implementation regardless of axis. `end` defaults to `start + extent`
/// to subsume `RowSlot::bottom` / `ColSlot::right` for generic callers.
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

/// Snap `pixel` to a slot's trailing edge when it falls within `hit_zone`.
/// Breaks once a slot's end is past the hit zone — slot vecs are monotonic
/// so no later slot can match.
pub fn boundary_at<S: AxisSlot>(
    frozen: &[S],
    scroll: &[S],
    pixel: i32,
    hit_zone: i32,
) -> Option<i32> {
    for s in frozen.iter().chain(scroll.iter()) {
        if (s.end() - pixel).abs() <= hit_zone {
            return Some(s.id());
        }
        if s.end() > pixel + hit_zone {
            break;
        }
    }
    None
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
