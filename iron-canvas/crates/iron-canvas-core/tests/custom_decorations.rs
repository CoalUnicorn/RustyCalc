//! Stage-1 spec for the open decoration registry — the consumer band on
//! `Decorations`, reached through `Orchestrator::{add_decoration,
//! remove_decoration}`:
//!
//! - the `custom` group bracket paints above `FormulaRefs` (topmost),
//! - a custom hit beats the formula-ref decoration (and the frame walk),
//! - `add_decoration` raises OVERLAY; `remove_decoration` raises it only
//!   when the id was found.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::Chrome;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::painter::{GroupClass, PaintColor, Painter};
use iron_canvas_core::{
    FormulaRef, FormulaRefKind, HitTest, Layer, Orchestrator, PixelRect, Point, RCRange,
    RenderStrategy, SheetArea,
};
use iron_canvas_recorder::{DrawOp, MemSurface};

use common::TestModel;

const PROBE_COLOR: &str = "#ff00ff";

/// Minimal consumer layer: paints one magenta rect and, when `hits` is
/// set, claims every probe with an off-grid sentinel cell.
struct ProbeLayer {
    hits: bool,
}

impl Layer for ProbeLayer {
    fn group(&self) -> GroupClass {
        GroupClass::Custom
    }

    fn paint(&self, _frame: &Chrome, painter: &dyn Painter) {
        painter.rect_fill(
            PixelRect {
                top_left: Point { x: 1, y: 1 },
                width: 2,
                height: 2,
            },
            PaintColor::Static(PROBE_COLOR),
        );
    }

    fn hit_test(
        &self,
        _frame: &Chrome,
        _selection_range: RCRange,
        _x: i32,
        _y: i32,
    ) -> Option<HitTest> {
        self.hits.then_some(HitTest::Cell {
            row: -7,
            column: -7,
        })
    }
}

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_model(model);
    orch
}

fn overlay_group_pos(orch: &Orchestrator<MemSurface>, class: GroupClass) -> usize {
    let ops = orch.overlay_surface().recorder().ops();
    let Some(pos) = ops
        .iter()
        .position(|op| matches!(op, DrawOp::BeginGroup { class: c } if *c == class))
    else {
        panic!("no begin_group({class:?}) bracket on the overlay surface");
    };
    pos
}

#[test]
fn custom_band_paints_above_formula_refs() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(stub);
    orch.add_decoration(Rc::new(ProbeLayer { hits: false }));
    orch.render_pending();

    let formula_refs = overlay_group_pos(&orch, GroupClass::FormulaRefs);
    let custom = overlay_group_pos(&orch, GroupClass::Custom);
    assert!(
        custom > formula_refs,
        "custom band must bracket after formula-refs (topmost): custom at {custom}, formula-refs at {formula_refs}",
    );

    let ops = orch.overlay_surface().recorder().ops();
    assert!(
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { color, .. } if color == PROBE_COLOR)),
        "custom layer's paint must run inside its bracket",
    );
}

#[test]
fn custom_hit_beats_formula_ref_until_removed() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(stub);
    orch.set_formula_refs(vec![FormulaRef {
        sheet_area: SheetArea {
            sheet: 0,
            range: RCRange::from([2, 2, 4, 4]),
        },
        color_idx: 0,
        kind: FormulaRefKind::Direct,
    }]);
    let id = orch.add_decoration(Rc::new(ProbeLayer { hits: true }));
    orch.render_pending();

    let Some(rect) = orch.cell_rect(3, 3) else {
        panic!("cell (3,3) must be in frame after the first paint");
    };
    let x = f64::from(rect.top_left.x) + f64::from(rect.width) / 2.0;
    let y = f64::from(rect.top_left.y) + f64::from(rect.height) / 2.0;

    assert!(
        matches!(
            orch.hit_test(x, y),
            HitTest::Cell {
                row: -7,
                column: -7
            }
        ),
        "custom layer must hit-test before the formula-ref decoration",
    );

    assert!(orch.remove_decoration(id));
    assert!(
        matches!(orch.hit_test(x, y), HitTest::FormulaRef { .. }),
        "with the custom layer removed, the formula-ref hit resurfaces",
    );
}

#[test]
fn add_and_found_removal_raise_overlay_noop_removal_does_not() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(stub);
    orch.render_pending();
    assert_eq!(orch.last_strategy(), Some(RenderStrategy::FullRebuild));

    let id = orch.add_decoration(Rc::new(ProbeLayer { hits: false }));
    orch.render_pending();
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::OverlayOnly),
        "add_decoration raises OVERLAY",
    );

    assert!(orch.remove_decoration(id), "first removal finds the id");
    orch.render_pending();
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::OverlayOnly),
        "found removal raises OVERLAY",
    );

    let grid_ops = orch.grid_surface().recorder().ops().len();
    let overlay_ops = orch.overlay_surface().recorder().ops().len();
    assert!(!orch.remove_decoration(id), "second removal is a no-op");
    orch.render_pending();
    assert_eq!(
        orch.grid_surface().recorder().ops().len(),
        grid_ops,
        "no-op removal must not repaint the grid",
    );
    assert_eq!(
        orch.overlay_surface().recorder().ops().len(),
        overlay_ops,
        "no-op removal must not repaint the overlay",
    );
}
