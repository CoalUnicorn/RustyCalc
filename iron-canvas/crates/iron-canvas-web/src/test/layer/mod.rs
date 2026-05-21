use iron_canvas_core::layer::PaintGate;
use iron_canvas_core::signal::GridSignals;

const ANY: GridSignals = GridSignals::STRUCTURAL.union(GridSignals::OVERLAY);

#[test]
fn fresh_gate_does_not_paint() {
    let gate = PaintGate::new();
    assert!(!gate.should_paint());
}

#[test]
fn raise_enables_paint() {
    let gate = PaintGate::new();
    gate.raise(ANY);
    assert!(gate.should_paint());
}

#[test]
fn should_paint_clears_flag() {
    let gate = PaintGate::new();
    gate.raise(ANY);
    gate.should_paint();
    assert!(
        !gate.should_paint(),
        "flag must be cleared after first should_paint"
    );
}

#[test]
fn double_raise_still_paints_once() {
    let gate = PaintGate::new();
    gate.raise(ANY);
    gate.raise(ANY);
    assert!(gate.should_paint());
    assert!(!gate.should_paint());
}
