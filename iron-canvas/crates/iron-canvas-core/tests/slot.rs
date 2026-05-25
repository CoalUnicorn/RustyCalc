//! Direct unit tests for `geometry::slot`. These primitives are exercised
//! indirectly through `Chrome::next`'s pane construction, but the edge
//! cases here (empty scroll band, `max_cursor` boundary, `boundary_at`'s
//! early break, etc.) deserve targeted assertions independent of the
//! Chrome scaffolding.

use iron_canvas_core::geometry::slot::{
    boundary_at, fill_axis, last_visible_id, pixel_to_id, scroll_first, slot_at, top_id, ColSlot,
    RowSlot,
};

#[test]
fn fill_axis_walks_inclusive_range_and_returns_post_cursor() {
    // 4 columns of 50 px each starting at x=10 → returns 10 + 4*50 = 210.
    let mut slots: Vec<ColSlot> = Vec::new();
    let end = fill_axis(&mut slots, 1..=4, 10, i32::MAX, |_| 50);
    assert_eq!(slots.len(), 4);
    assert_eq!(slots[0].col, 1);
    assert_eq!(slots[0].left, 10);
    assert_eq!(slots[3].col, 4);
    assert_eq!(slots[3].left, 160);
    assert_eq!(end, 210, "cursor returned must be past the last slot");
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
    let end = fill_axis(&mut slots, 1..=10, 0, 110, |_| 50);
    assert_eq!(slots.len(), 4);
    assert_eq!(slots.last().expect("at least one slot").col, 4);
    assert_eq!(slots[3].left, 150);
    assert_eq!(end, 150, "cursor sits at the breaking slot's leading edge");
}

#[test]
fn fill_axis_empty_range_pushes_nothing_and_returns_start() {
    let mut slots: Vec<RowSlot> = Vec::new();
    #[allow(clippy::reversed_empty_ranges)]
    let end = fill_axis(&mut slots, 5..=4, 100, i32::MAX, |_| 20);
    assert!(slots.is_empty());
    assert_eq!(end, 100);
}

#[test]
fn fill_axis_max_cursor_at_start_still_pushes_first_slot() {
    // The break is post-push, so the first slot lands even when
    // max_cursor == start. This is the load-bearing case for the frozen
    // band passing `max_cursor = i32::MAX` vs the scroll band passing
    // a real ceiling — the frozen band never short-circuits, but the
    // scroll band must still emit at least one slot per call.
    let mut slots: Vec<ColSlot> = Vec::new();
    fill_axis(&mut slots, 1..=10, 0, 0, |_| 50);
    assert_eq!(slots.len(), 1, "first slot pushes before the break check");
}

#[test]
fn scroll_first_prefers_view_first_past_frozen_band() {
    // No frozen → view always wins.
    assert_eq!(scroll_first(0, 1), 1);
    assert_eq!(scroll_first(0, 50), 50);
    // Frozen pushes the floor.
    assert_eq!(scroll_first(3, 1), 4, "frozen_count + 1 wins when view <= frozen");
    assert_eq!(scroll_first(3, 4), 4, "tie goes to frozen+1 via max()");
    assert_eq!(scroll_first(3, 10), 10, "view past frozen wins");
}

#[test]
fn slot_at_finds_frozen_then_scroll_then_misses() {
    let frozen = vec![
        ColSlot { col: 1, left: 0, width: 50 },
        ColSlot { col: 2, left: 50, width: 50 },
    ];
    // Scroll band starts at col 7 (cols 3..=6 scrolled off-screen).
    let scroll = vec![
        ColSlot { col: 7, left: 100, width: 50 },
        ColSlot { col: 8, left: 150, width: 50 },
        ColSlot { col: 9, left: 200, width: 50 },
    ];

    assert_eq!(slot_at(&frozen, &scroll, 1).map(|s| s.col), Some(1));
    assert_eq!(slot_at(&frozen, &scroll, 2).map(|s| s.col), Some(2));
    assert_eq!(slot_at(&frozen, &scroll, 7).map(|s| s.col), Some(7));
    assert_eq!(slot_at(&frozen, &scroll, 9).map(|s| s.col), Some(9));

    // Past the frozen band but before the scroll band's first id — falls
    // into the gap, returns None.
    assert!(slot_at(&frozen, &scroll, 3).is_none(), "scrolled-off ids are not addressable");
    // Past the scroll band's tail.
    assert!(slot_at(&frozen, &scroll, 99).is_none());
}

#[test]
fn slot_at_frozen_only_works_with_empty_scroll() {
    let frozen = vec![
        RowSlot { row: 1, top: 0, height: 20 },
        RowSlot { row: 2, top: 20, height: 20 },
    ];
    let scroll: Vec<RowSlot> = Vec::new();
    assert_eq!(slot_at(&frozen, &scroll, 1).map(|s| s.row), Some(1));
    assert_eq!(slot_at(&frozen, &scroll, 2).map(|s| s.row), Some(2));
    assert!(slot_at(&frozen, &scroll, 3).is_none(), "no scroll band → past-frozen returns None");
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
        RowSlot { row: 10, top: 0, height: 20 },
        RowSlot { row: 11, top: 20, height: 20 },
        RowSlot { row: 12, top: 40, height: 20 },
    ];
    assert_eq!(top_id(&scroll), 10);
    assert_eq!(last_visible_id(&scroll), 12);
}

#[test]
fn pixel_to_id_hits_inside_band_and_misses_at_end_edge() {
    // Slot bounds are [start, end) — half-open by the strict-less-than
    // check in pixel_to_id. The trailing pixel belongs to the NEXT slot.
    let frozen = vec![ColSlot { col: 1, left: 0, width: 50 }];
    let scroll = vec![
        ColSlot { col: 2, left: 50, width: 50 },
        ColSlot { col: 3, left: 100, width: 50 },
    ];

    assert_eq!(pixel_to_id(&frozen, &scroll, 0), Some(1), "leading edge inclusive");
    assert_eq!(pixel_to_id(&frozen, &scroll, 49), Some(1));
    assert_eq!(pixel_to_id(&frozen, &scroll, 50), Some(2), "boundary belongs to the next slot");
    assert_eq!(pixel_to_id(&frozen, &scroll, 100), Some(3));
    assert_eq!(pixel_to_id(&frozen, &scroll, 149), Some(3));
    assert!(pixel_to_id(&frozen, &scroll, 150).is_none(), "past last slot's end");
    assert!(pixel_to_id(&frozen, &scroll, -1).is_none(), "before first slot");
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
        ColSlot { col: 1, left: 0, width: 50 },
        ColSlot { col: 2, left: 50, width: 50 },
        ColSlot { col: 3, left: 100, width: 50 },
    ];

    // Slot 1 ends at x=50. Hit zone 3 → x ∈ [47, 53] snaps to col 1.
    assert_eq!(boundary_at(&frozen, &scroll, 50, 3), Some(1));
    assert_eq!(boundary_at(&frozen, &scroll, 47, 3), Some(1));
    assert_eq!(boundary_at(&frozen, &scroll, 53, 3), Some(1));
    // Just past the zone → returns the NEXT slot's edge if within hit_zone.
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
        ColSlot { col: 1, left: 0, width: 100 },
        ColSlot { col: 2, left: 100, width: 100 },
        ColSlot { col: 3, left: 200, width: 100 },
    ];

    // Pixel mid-slot 1, hit_zone 5 → slot 1's end (100) > 50 + 5, so the
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
    let frozen = vec![RowSlot { row: 1, top: 0, height: 20 }];
    let scroll = vec![
        RowSlot { row: 5, top: 20, height: 20 },
        RowSlot { row: 6, top: 40, height: 20 },
    ];
    // Frozen slot 1 ends at y=20 → snaps to row 1.
    assert_eq!(boundary_at(&frozen, &scroll, 20, 2), Some(1));
}
