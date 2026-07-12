use iron_canvas_core::signal::{CellDamage, GridSignals, RowSpan};

#[test]
fn empty_is_clean() {
    assert!(GridSignals::EMPTY.is_empty());
    assert!(!GridSignals::EMPTY.grid_dirty());
    assert!(!GridSignals::EMPTY.overlay_dirty());
}

#[test]
fn union_accumulates() {
    let s = GridSignals::VIEWPORT | GridSignals::OVERLAY;
    assert!(s.contains(GridSignals::VIEWPORT));
    assert!(s.contains(GridSignals::OVERLAY));
    assert!(!s.contains(GridSignals::STRUCTURAL));
    assert!(s.grid_dirty());
    assert!(s.overlay_dirty());
}

#[test]
fn grid_any_partitions_overlay() {
    assert!(!GridSignals::GRID_ANY.intersects(GridSignals::OVERLAY));
    assert!(GridSignals::ALL.contains(GridSignals::OVERLAY));
    assert!(GridSignals::ALL.contains(GridSignals::GRID_ANY));
}

#[test]
fn bit_or_assign_compiles() {
    let mut s = GridSignals::EMPTY;
    s |= GridSignals::CONTENT;
    s |= GridSignals::OVERLAY;
    assert!(s.contains(GridSignals::CONTENT));
    assert!(s.contains(GridSignals::OVERLAY));
}

#[test]
fn damage_accumulates_and_merges_overlapping_spans() {
    let mut d = CellDamage::default();
    d.add_rows(0, RowSpan { r1: 5, r2: 7 });
    d.add_rows(0, RowSpan { r1: 6, r2: 9 });
    d.add_rows(0, RowSpan { r1: 20, r2: 20 });
    assert_eq!(
        d,
        CellDamage::Rows {
            sheet: 0,
            spans: vec![RowSpan { r1: 5, r2: 9 }, RowSpan { r1: 20, r2: 20 }],
        }
    );
}

#[test]
fn damage_merges_adjacent_spans() {
    let mut d = CellDamage::default();
    d.add_rows(0, RowSpan { r1: 5, r2: 6 });
    d.add_rows(0, RowSpan { r1: 7, r2: 8 });
    assert_eq!(
        d,
        CellDamage::Rows { sheet: 0, spans: vec![RowSpan { r1: 5, r2: 8 }] }
    );
}

#[test]
fn damage_exceeds_on_cross_sheet() {
    let mut d = CellDamage::default();
    d.add_rows(0, RowSpan { r1: 1, r2: 1 });
    d.add_rows(1, RowSpan { r1: 2, r2: 2 });
    assert_eq!(d, CellDamage::Exceeded);
}

#[test]
fn damage_exceeds_past_span_cap() {
    let mut d = CellDamage::default();
    // 9 disjoint spans (rows 0, 10, 20, … 80) blow the cap of 8.
    for i in 0..9 {
        d.add_rows(0, RowSpan { r1: i * 10, r2: i * 10 });
    }
    assert_eq!(d, CellDamage::Exceeded);
}

#[test]
fn damage_normalizes_reversed_spans() {
    // A reversed span that survived to paint time would intersect every
    // pane to nothing — CONTENT drained, pixels stale. Normalize at the
    // accumulator so no caller (incl. the JS facade) can smuggle one in.
    let mut d = CellDamage::default();
    d.add_rows(0, RowSpan { r1: 9, r2: 5 });
    assert_eq!(
        d,
        CellDamage::Rows { sheet: 0, spans: vec![RowSpan { r1: 5, r2: 9 }] }
    );
}

#[test]
fn poison_downgrades_clean_and_rows_and_sticks() {
    let mut d = CellDamage::default();
    d.poison();
    assert_eq!(d, CellDamage::Exceeded);
    // Rows recorded *after* a poison must not resurrect the fast path:
    // the poisoning raise's panes would never repaint.
    d.add_rows(0, RowSpan { r1: 1, r2: 1 });
    assert_eq!(d, CellDamage::Exceeded);

    let mut d = CellDamage::default();
    d.add_rows(0, RowSpan { r1: 1, r2: 1 });
    d.poison();
    assert_eq!(d, CellDamage::Exceeded);
}
