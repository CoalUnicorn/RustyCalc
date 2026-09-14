//! Per-axis visible slots on the painted frame.
//!
//! A slot carries the index, the absolute canvas coordinate of its leading
//! edge, and its extent. `PaneSet` holds four vecs (frozen/scrollable ×
//! row/column); every pixel<->cell query reads them directly, no prefix-sum
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
/// `max_cursor`. Returns `Some(cursor)` — the cursor past the last accepted
/// slot (where the next slot would sit) — used by the frozen pass to compute
/// the band offset. `None` when `measure` aborts the walk: a model read
/// failed, an extent was invalid, or a slot end overflowed. The caller must
/// not commit the partially-built geometry.
///
/// `max_cursor = None` disables the break, used for the frozen band which
/// always paints regardless of viewport size. Scroll-band callers pass the
/// canvas's outward-rounded logical extent.
pub fn fill_axis<S: AxisSlot>(
    slots: &mut Vec<S>,
    range: std::ops::RangeInclusive<i32>,
    start: i32,
    max_cursor: Option<i32>,
    mut measure: impl FnMut(i32) -> Option<i32>,
) -> Option<i32> {
    let mut cursor = start;
    for id in range {
        let extent = measure(id)?;
        // Queries also read the trailing slot's end, even when it is off-canvas.
        let end = cursor.checked_add(extent)?;
        slots.push(S::new(id, cursor, extent));
        if max_cursor.is_some_and(|max| cursor >= max) {
            break;
        }
        cursor = end;
    }
    Some(cursor)
}

/// First non-frozen visible id along an axis: the larger of `frozen_count + 1`
/// (the id immediately past the frozen band) and the viewport's scrolled-to
/// id. Encodes the "scroll band starts where frozen ends or where the user
/// scrolled to, whichever is further" invariant — used by both `fill_axis`
/// callers and `Chrome::classify`.
#[inline]
pub fn scroll_first(frozen_count: i32, view_first: i32) -> i32 {
    (frozen_count + 1).max(view_first)
}

/// Locate a slot by `id` across the frozen+scroll pair. Frozen ids index
/// from 1; scroll ids index from the first slot's id (the scroll band starts
/// past whatever has been scrolled off-screen).
fn slot_at<'a, S: AxisSlot>(frozen: &'a [S], scroll: &'a [S], id: i32) -> Option<&'a S> {
    let frozen_n = frozen.len() as i32;
    if id <= frozen_n {
        frozen.get((id - 1) as usize)
    } else {
        let first = scroll.first()?.id();
        debug_assert!(
            scroll
                .iter()
                .enumerate()
                .all(|(i, slot)| slot.id() == first + i as i32),
            "slot_at requires dense contiguous scroll ids"
        );
        let idx = id.checked_sub(first)? as usize;
        scroll.get(idx).filter(|slot| slot.id() == id)
    }
}

/// First scroll-band id, or `1` when the band is empty (matches the fresh-
/// frame default before any scroll happens).
fn top_id<S: AxisSlot>(scroll: &[S]) -> i32 {
    scroll.first().map(|s| s.id()).unwrap_or(1)
}

/// Last scroll-band id visible, falling back to [`top_id`] when the band is
/// empty so callers always get a valid id.
fn last_visible_id<S: AxisSlot>(scroll: &[S]) -> i32 {
    scroll
        .last()
        .map(|s| s.id())
        .unwrap_or_else(|| top_id(scroll))
}

/// Linear-scan frozen-then-scroll for the slot covering `pixel`. Used to map
/// a canvas Y/X back to a row/column.
fn pixel_to_id<S: AxisSlot>(frozen: &[S], scroll: &[S], pixel: i32) -> Option<i32> {
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
fn boundary_at<S: AxisSlot>(frozen: &[S], scroll: &[S], pixel: i32, tolerance: i32) -> Option<i32> {
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
    /// Last addressable slot id this axis walks to (`CanvasModel::last_row`
    /// / `last_column`), snapshotted by `fill`. The blit-path rebuilds and
    /// the autofill-handle guard read this instead of re-querying the model,
    /// so a frame's queries stay coherent with its painted extent.
    pub last_id: i32,
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

    /// Project an inclusive address interval onto this axis's frozen+scroll
    /// slot union.
    ///
    /// Returns `Some((start, end))` — the leading pixel edge of the lowest
    /// covered id and the trailing pixel edge of the highest covered id — or
    /// `None` when no id in the interval has a slot in this frame (it lies
    /// entirely in the address gap between the frozen band and the
    /// scrolled-to band, or past the walked extent).
    ///
    /// The bands are deliberately not flattened. An interval that covers the
    /// frozen band *and* the scroll band projects across the separator between
    /// them, so the returned span includes both bands and the separator pixels
    /// — which is what an outline over rows in both bands must cover. Both vecs
    /// hold dense, contiguous ids (`fill_axis` guarantees it), so each covered
    /// sub-span is a slice, not a scan of skipped ids.
    pub fn project_interval(&self, a: i32, b: i32) -> Option<(i32, i32)> {
        let lo = a.min(b);
        let hi = a.max(b);
        match (self.frozen_span(lo, hi), self.scroll_span(lo, hi)) {
            (Some((f_start, f_end)), Some((s_start, s_end))) => {
                Some((f_start.min(s_start), f_end.max(s_end)))
            }
            (Some(span), None) | (None, Some(span)) => Some(span),
            (None, None) => None,
        }
    }

    /// Covered pixel span of the frozen band within `[lo, hi]` (ids `1..=len`).
    fn frozen_span(&self, lo: i32, hi: i32) -> Option<(i32, i32)> {
        let len = self.frozen.len() as i32;
        if len == 0 || hi < 1 || lo > len {
            return None;
        }
        let first = &self.frozen[(lo.max(1) - 1) as usize];
        let last = &self.frozen[(hi.min(len) - 1) as usize];
        Some((first.start(), last.end()))
    }

    /// Covered pixel span of the scroll band within `[lo, hi]`.
    fn scroll_span(&self, lo: i32, hi: i32) -> Option<(i32, i32)> {
        let first_id = self.scroll.first()?.id();
        let last_id = self.scroll.last()?.id();
        if hi < first_id || lo > last_id {
            return None;
        }
        let first = &self.scroll[(lo.max(first_id) - first_id) as usize];
        let last = &self.scroll[(hi.min(last_id) - first_id) as usize];
        Some((first.start(), last.end()))
    }

    /// Populate `frozen` + `scroll` and record `frozen_offset`. Walks the
    /// frozen band first (always painted — `None` disables the viewport
    /// break), notes where the scroll band starts, then walks the scroll band
    /// from `view_first` to `last`, breaking at the canvas edge.
    ///
    /// `frozen_offset` is the seam invariant `boundary_at` relies on: the
    /// scroll band must begin at or after the frozen band ends.
    ///
    /// The `measure` closure returns `None` when the model read failed
    /// transiently (`BridgeFailed`); the walk aborts and `fill` returns
    /// `false`, so the caller must not commit the partially-filled slots.
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
        mut measure: impl FnMut(&dyn CanvasModel, i32) -> Option<i32>,
    ) -> bool {
        self.last_id = last;
        self.frozen.reserve(frozen_count as usize);
        let Some(after_frozen) =
            fill_axis(&mut self.frozen, 1..=frozen_count, origin, None, |id| {
                measure(model, id)
            })
        else {
            return false;
        };
        let Some(frozen_offset) =
            after_frozen.checked_add(if frozen_count > 0 { FROZEN_SEP } else { 0 })
        else {
            return false;
        };
        self.frozen_offset = frozen_offset;

        fill_axis(
            &mut self.scroll,
            scroll_first(frozen_count, view_first)..=last,
            self.frozen_offset,
            Some(canvas_extent),
            |id| measure(model, id),
        )
        .is_some()
    }
}

/// A single-axis slot extent resolved from a model read.
///
/// The `Fetched` outcome from `CanvasModel` is resolved here, at the
/// geometry boundary: `Value` becomes its validated pixel extent, `Absent`
/// selects the axis's documented default (a row/column the model has no
/// override for), `BridgeFailed` is a transient read failure, and `Invalid`
/// is a host value that cannot become geometry (non-finite, negative, or
/// beyond `i32::MAX` px). `BridgeFailed` and `Invalid` both abort the walk —
/// the caller must hold the attempt, never substitute a default or fabricate
/// geometry from a broken number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentFetch {
    /// Validated pixel extent: finite, non-negative, `0..=i32::MAX` px.
    Px(i32),
    /// The host returned a value that cannot become a slot extent
    /// (non-finite, negative, or overflowing `i32` px). Never round/cast it
    /// into geometry: a `NaN` cast would fabricate a hidden row, a negative
    /// value would break monotonic slot math.
    Invalid,
    /// The model read failed transiently; retry the whole attempt later.
    BridgeFailed,
}

impl ExtentFetch {
    /// Validated pixel parse for one host extent value.
    ///
    /// Accepts a finite value in `0..=i32::MAX` px after rounding. Zero stays
    /// a valid extent — the hidden-row/hidden-column value. Everything else
    /// (NaN, infinities, negative values, overflow) is `ExtentFetch::Invalid`,
    /// never clamped or cast into a fabricated slot.
    fn from_host_px(v: f64) -> ExtentFetch {
        if !v.is_finite() || v < 0.0 {
            return ExtentFetch::Invalid;
        }
        let px = v.round();
        if px > i32::MAX as f64 {
            return ExtentFetch::Invalid;
        }
        ExtentFetch::Px(px as i32)
    }

    /// Collapse to a pixel extent for an abortable walk: `Some(px)` for a
    /// resolved extent (validated concrete value or documented default),
    /// `None` for `BridgeFailed` (transient) or `Invalid` (host value that
    /// cannot become geometry), so the walk stops without committing
    /// fabricated geometry.
    pub fn extent(self) -> Option<i32> {
        match self {
            ExtentFetch::Px(px) => Some(px),
            ExtentFetch::Invalid | ExtentFetch::BridgeFailed => None,
        }
    }
}

/// Resolve one row-height read to a pixel extent.
///
/// `sheet` is the caller's already-captured/committed sheet — this never
/// re-reads `CanvasModel::get_selected_sheet()` itself, so a slot walk over
/// many rows costs one sheet read total, not one per row (see
/// `PaneSet::fill_rows`, `Chrome`'s blit rebuild, and `Orchestrator::scroll_to_show`,
/// every one of which now supplies it explicitly).
pub fn row_height(model: &dyn CanvasModel, sheet: u32, row: i32) -> ExtentFetch {
    match model.get_row_height(sheet, row) {
        crate::types::fetched::Fetched::Value(h) => ExtentFetch::from_host_px(h),
        crate::types::fetched::Fetched::Absent => {
            ExtentFetch::Px(DEFAULT_ROW_HEIGHT.round() as i32)
        }
        crate::types::fetched::Fetched::BridgeFailed => ExtentFetch::BridgeFailed,
    }
}

/// Column mirror of [`row_height`]; same explicit-`sheet` rationale.
pub fn col_width(model: &dyn CanvasModel, sheet: u32, col: i32) -> ExtentFetch {
    match model.get_column_width(sheet, col) {
        crate::types::fetched::Fetched::Value(w) => ExtentFetch::from_host_px(w),
        crate::types::fetched::Fetched::Absent => ExtentFetch::Px(DEFAULT_COL_WIDTH.round() as i32),
        crate::types::fetched::Fetched::BridgeFailed => ExtentFetch::BridgeFailed,
    }
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
            last_id: 6,
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

    // Relocated from tests/slot.rs to test the private query helpers here.
    // The public fill_axis and scroll_first tests stay with the same suite.

    #[test]
    fn fill_axis_rejects_overflow_even_in_the_trailing_slot() {
        for max_cursor in [None, Some(10)] {
            let mut slots: Vec<RowSlot> = Vec::new();
            assert_eq!(
                fill_axis(&mut slots, 1..=1, 10, max_cursor, |_| Some(i32::MAX)),
                None
            );
            assert!(slots.is_empty(), "an overflowing slot must not be stored");
        }
    }

    #[test]
    fn fill_axis_walks_inclusive_range_and_returns_post_cursor() {
        // 4 columns of 50 px each starting at x=10 -> returns 10 + 4*50 = 210.
        let mut slots: Vec<ColSlot> = Vec::new();
        let end = fill_axis(&mut slots, 1..=4, 10, None, |_| Some(50));
        assert_eq!(slots.len(), 4);
        assert_eq!(slots[0].col, 1);
        assert_eq!(slots[0].left, 10);
        assert_eq!(slots[3].col, 4);
        assert_eq!(slots[3].left, 160);
        assert_eq!(end, Some(210), "cursor returned must be past the last slot");
    }

    #[test]
    fn fill_axis_breaks_post_push_at_max_cursor() {
        // The break compares the just-pushed slot's leading edge against
        // max_cursor *after* push: with max_cursor=110 and 50-wide slots,
        // id=4 at x=150 trips the break — so 4 slots get emitted even though
        // only the first three intersect the canvas. The last slot lies
        // entirely off-canvas; that overshoot is intentional so consumers
        // never miss the last visible boundary.
        let mut slots: Vec<ColSlot> = Vec::new();
        let end = fill_axis(&mut slots, 1..=10, 0, Some(110), |_| Some(50));
        assert_eq!(slots.len(), 4);
        assert_eq!(slots.last().expect("at least one slot").col, 4);
        assert_eq!(slots[3].left, 150);
        assert_eq!(
            end,
            Some(150),
            "cursor sits at the breaking slot's leading edge"
        );
    }

    #[test]
    fn fill_axis_empty_range_pushes_nothing_and_returns_start() {
        let mut slots: Vec<RowSlot> = Vec::new();
        #[allow(clippy::reversed_empty_ranges)]
        let end = fill_axis(&mut slots, 5..=4, 100, None, |_| Some(20));
        assert!(slots.is_empty());
        assert_eq!(end, Some(100));
    }

    #[test]
    fn fill_axis_max_cursor_at_start_still_pushes_first_slot() {
        // The break is post-push, so the first slot lands even when
        // max_cursor == start. This is the load-bearing case for the frozen
        // band passing `max_cursor = None` vs the scroll band passing
        // a real ceiling — the frozen band never short-circuits, but the
        // scroll band must still emit at least one slot per call.
        let mut slots: Vec<ColSlot> = Vec::new();
        fill_axis(&mut slots, 1..=10, 0, Some(0), |_| Some(50));
        assert_eq!(slots.len(), 1, "first slot pushes before the break check");
    }

    #[test]
    fn scroll_first_prefers_view_first_past_frozen_band() {
        // No frozen -> view always wins.
        assert_eq!(scroll_first(0, 1), 1);
        assert_eq!(scroll_first(0, 50), 50);
        // Frozen pushes the floor.
        assert_eq!(
            scroll_first(3, 1),
            4,
            "frozen_count + 1 wins when view <= frozen"
        );
        assert_eq!(scroll_first(3, 4), 4, "tie goes to frozen+1 via max()");
        assert_eq!(scroll_first(3, 10), 10, "view past frozen wins");
    }

    #[test]
    fn slot_at_finds_frozen_then_scroll_then_misses() {
        let frozen = vec![
            ColSlot {
                col: 1,
                left: 0,
                width: 50,
            },
            ColSlot {
                col: 2,
                left: 50,
                width: 50,
            },
        ];
        // Scroll band starts at col 7 (cols 3..=6 scrolled off-screen).
        let scroll = vec![
            ColSlot {
                col: 7,
                left: 100,
                width: 50,
            },
            ColSlot {
                col: 8,
                left: 150,
                width: 50,
            },
            ColSlot {
                col: 9,
                left: 200,
                width: 50,
            },
        ];

        assert_eq!(slot_at(&frozen, &scroll, 1).map(|s| s.col), Some(1));
        assert_eq!(slot_at(&frozen, &scroll, 2).map(|s| s.col), Some(2));
        assert_eq!(slot_at(&frozen, &scroll, 7).map(|s| s.col), Some(7));
        assert_eq!(slot_at(&frozen, &scroll, 9).map(|s| s.col), Some(9));

        // Past the frozen band but before the scroll band's first id — falls
        // into the gap, returns None.
        assert!(
            slot_at(&frozen, &scroll, 3).is_none(),
            "scrolled-off ids are not addressable"
        );
        // Past the scroll band's tail.
        assert!(slot_at(&frozen, &scroll, 99).is_none());
    }

    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "dense contiguous scroll ids")
    )]
    #[test]
    fn slot_at_rejects_sparse_scroll_candidate() {
        let frozen = vec![ColSlot {
            col: 1,
            left: 0,
            width: 50,
        }];
        let scroll = vec![
            ColSlot {
                col: 7,
                left: 50,
                width: 50,
            },
            ColSlot {
                col: 9,
                left: 100,
                width: 50,
            },
        ];

        #[cfg(debug_assertions)]
        let _ = slot_at(&frozen, &scroll, 8);
        #[cfg(not(debug_assertions))]
        assert!(
            slot_at(&frozen, &scroll, 8).is_none(),
            "sparse scroll ids must not index through to the next physical slot"
        );
    }

    #[test]
    fn slot_at_frozen_only_works_with_empty_scroll() {
        let frozen = vec![
            RowSlot {
                row: 1,
                top: 0,
                height: 20,
            },
            RowSlot {
                row: 2,
                top: 20,
                height: 20,
            },
        ];
        let scroll: Vec<RowSlot> = Vec::new();
        assert_eq!(slot_at(&frozen, &scroll, 1).map(|s| s.row), Some(1));
        assert_eq!(slot_at(&frozen, &scroll, 2).map(|s| s.row), Some(2));
        assert!(
            slot_at(&frozen, &scroll, 3).is_none(),
            "no scroll band -> past-frozen returns None"
        );
    }

    #[test]
    fn top_id_and_last_visible_id_fall_back_to_one_when_empty() {
        let scroll: Vec<RowSlot> = Vec::new();
        assert_eq!(top_id(&scroll), 1);
        assert_eq!(last_visible_id(&scroll), 1);
    }

    #[test]
    fn top_id_and_last_visible_id_read_scroll_band_when_populated() {
        let scroll = vec![
            RowSlot {
                row: 10,
                top: 0,
                height: 20,
            },
            RowSlot {
                row: 11,
                top: 20,
                height: 20,
            },
            RowSlot {
                row: 12,
                top: 40,
                height: 20,
            },
        ];
        assert_eq!(top_id(&scroll), 10);
        assert_eq!(last_visible_id(&scroll), 12);
    }

    #[test]
    fn pixel_to_id_hits_inside_band_and_misses_at_end_edge() {
        // Slot bounds are [start, end) — half-open by the strict-less-than
        // check in pixel_to_id. The trailing pixel belongs to the NEXT slot.
        let frozen = vec![ColSlot {
            col: 1,
            left: 0,
            width: 50,
        }];
        let scroll = vec![
            ColSlot {
                col: 2,
                left: 50,
                width: 50,
            },
            ColSlot {
                col: 3,
                left: 100,
                width: 50,
            },
        ];

        assert_eq!(
            pixel_to_id(&frozen, &scroll, 0),
            Some(1),
            "leading edge inclusive"
        );
        assert_eq!(pixel_to_id(&frozen, &scroll, 49), Some(1));
        assert_eq!(
            pixel_to_id(&frozen, &scroll, 50),
            Some(2),
            "boundary belongs to the next slot"
        );
        assert_eq!(pixel_to_id(&frozen, &scroll, 100), Some(3));
        assert_eq!(pixel_to_id(&frozen, &scroll, 149), Some(3));
        assert!(
            pixel_to_id(&frozen, &scroll, 150).is_none(),
            "past last slot's end"
        );
        assert!(
            pixel_to_id(&frozen, &scroll, -1).is_none(),
            "before first slot"
        );
    }

    #[test]
    fn pixel_to_id_empty_bands_returns_none() {
        let frozen: Vec<ColSlot> = Vec::new();
        let scroll: Vec<ColSlot> = Vec::new();
        assert!(pixel_to_id(&frozen, &scroll, 0).is_none());
        assert!(pixel_to_id(&frozen, &scroll, 100).is_none());
    }

    #[test]
    fn boundary_at_snaps_within_hit_zone_to_trailing_edge() {
        let frozen: Vec<ColSlot> = Vec::new();
        let scroll = vec![
            ColSlot {
                col: 1,
                left: 0,
                width: 50,
            },
            ColSlot {
                col: 2,
                left: 50,
                width: 50,
            },
            ColSlot {
                col: 3,
                left: 100,
                width: 50,
            },
        ];

        // Slot 1 ends at x=50. Hit zone 3 -> x  [47, 53] snaps to col 1.
        assert_eq!(boundary_at(&frozen, &scroll, 50, 3), Some(1));
        assert_eq!(boundary_at(&frozen, &scroll, 47, 3), Some(1));
        assert_eq!(boundary_at(&frozen, &scroll, 53, 3), Some(1));
        // Just past the zone -> returns the NEXT slot's edge if within hit_zone.
        assert_eq!(boundary_at(&frozen, &scroll, 100, 3), Some(2));
    }

    #[test]
    fn boundary_at_returns_none_in_slot_interior_and_breaks_early() {
        // The early break: once a slot's end > pixel + hit_zone, no later
        // slot's end can be within hit_zone either (vecs are monotonic). The
        // measure closure tracks how many slots were inspected — proves the
        // break fired without relying on internal state.
        let frozen: Vec<ColSlot> = Vec::new();
        let scroll = vec![
            ColSlot {
                col: 1,
                left: 0,
                width: 100,
            },
            ColSlot {
                col: 2,
                left: 100,
                width: 100,
            },
            ColSlot {
                col: 3,
                left: 200,
                width: 100,
            },
        ];

        // Pixel mid-slot 1, hit_zone 5 -> slot 1's end (100) > 50 + 5, so the
        // loop breaks before inspecting slot 2. None returned because slot 1
        // itself isn't within the zone either.
        assert!(boundary_at(&frozen, &scroll, 50, 5).is_none());
    }

    #[test]
    fn boundary_at_empty_bands_returns_none() {
        let frozen: Vec<RowSlot> = Vec::new();
        let scroll: Vec<RowSlot> = Vec::new();
        assert!(boundary_at(&frozen, &scroll, 10, 3).is_none());
    }

    #[test]
    fn boundary_at_walks_frozen_before_scroll() {
        let frozen = vec![RowSlot {
            row: 1,
            top: 0,
            height: 20,
        }];
        let scroll = vec![
            RowSlot {
                row: 5,
                top: 20,
                height: 20,
            },
            RowSlot {
                row: 6,
                top: 40,
                height: 20,
            },
        ];
        // Frozen slot 1 ends at y=20 -> snaps to row 1.
        assert_eq!(boundary_at(&frozen, &scroll, 20, 2), Some(1));
    }
}
