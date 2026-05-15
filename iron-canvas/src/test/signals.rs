use crate::signal::GridSignals;

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
