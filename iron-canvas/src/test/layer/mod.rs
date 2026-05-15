use crate::layer::PaintGate;

#[test]
fn fresh_gate_does_not_paint() {
    let gate = PaintGate::new();
    assert!(!gate.should_paint());
}

#[test]
fn mark_dirty_enables_paint() {
    let gate = PaintGate::new();
    gate.mark_dirty();
    assert!(gate.should_paint());
}

#[test]
fn should_paint_clears_flag() {
    let gate = PaintGate::new();
    gate.mark_dirty();
    gate.should_paint();
    assert!(
        !gate.should_paint(),
        "flag must be cleared after first should_paint"
    );
}

#[test]
fn double_mark_dirty_still_paints_once() {
    let gate = PaintGate::new();
    gate.mark_dirty();
    gate.mark_dirty();
    assert!(gate.should_paint());
    assert!(!gate.should_paint());
}
