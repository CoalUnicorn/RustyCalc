use super::axis::AxisSlot;

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
pub(super) fn slot_at<'a, S: AxisSlot>(frozen: &'a [S], scroll: &'a [S], id: i32) -> Option<&'a S> {
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
pub(super) fn top_id<S: AxisSlot>(scroll: &[S]) -> i32 {
    scroll.first().map(|s| s.id()).unwrap_or(1)
}

/// Last scroll-band id visible, falling back to [`top_id`] when the band is
/// empty so callers always get a valid id.
pub(super) fn last_visible_id<S: AxisSlot>(scroll: &[S]) -> i32 {
    scroll
        .last()
        .map(|s| s.id())
        .unwrap_or_else(|| top_id(scroll))
}

/// Linear-scan frozen-then-scroll for the slot covering `pixel`. Used to map
/// a canvas Y/X back to a row/column.
pub(super) fn pixel_to_id<S: AxisSlot>(frozen: &[S], scroll: &[S], pixel: i32) -> Option<i32> {
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
pub(super) fn boundary_at<S: AxisSlot>(
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
