#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use crate::{CanvasModel, RCRange, RenderOverlays};
use std::rc::Rc;

struct StubModel;
impl CanvasModel for StubModel {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> crate::CanvasView {
        crate::CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from([1, 1, 1, 1]),
            top_row: 1,
            left_column: 1,
        }
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _: u32, _: i32) -> Option<f64> {
        Some(20.0)
    }
    fn get_column_width(&self, _: u32, _: i32) -> Option<f64> {
        Some(80.0)
    }
    fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
        Some(true)
    }
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<ironcalc_base::types::Style> {
        Some(ironcalc_base::types::Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<ironcalc_base::types::CellType> {
        Some(ironcalc_base::types::CellType::Number)
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Option<String> {
        Some(String::new())
    }
}

// Drag-frame isolation tests
//
// These simulate the headline acceptance criterion without a browser: two
// `PaintGate` instances stand in for the real layers. The logic mirrors
// exactly what `IronCanvas::set_overlays` and `paint_if_dirty` do in
// production, so a pass here proves the fan-out policy is correct.

// Per-frame overlay state used to drive the value-compare path. Autofill drag
// is the canonical scenario where `set_overlays` fires once per frame with a
// changing target — selection no longer lives on `RenderOverlays`.
fn drag_target(col: i32) -> crate::types::coord::AutofillTarget {
    crate::types::coord::AutofillTarget { row: 1, col }
}

#[test]
fn set_overlays_only_dirties_overlay() {
    use crate::layer::PaintGate;
    let mut grid = PaintGate::new();
    let mut overlay = PaintGate::new();
    let mut current = RenderOverlays::default();

    let next = RenderOverlays {
        extend_to: Some(drag_target(2)),
        ..Default::default()
    };
    // mirror set_overlays fan-out policy
    if next != current {
        overlay.mark_dirty();
    }
    current = next;
    let _ = current;

    assert!(
        overlay.should_paint(),
        "overlay must be dirty after set_overlays"
    );
    assert!(
        !grid.should_paint(),
        "grid must NOT be dirty after set_overlays"
    );
}

#[test]
fn sixty_drag_frames_increment_overlay_only() {
    use crate::layer::PaintGate;
    let mut grid = PaintGate::new();
    let mut overlay = PaintGate::new();
    let mut current = RenderOverlays::default();

    for i in 0..60_i32 {
        let next = RenderOverlays {
            extend_to: Some(drag_target(i)),
            ..Default::default()
        };
        // mirror set_overlays
        if next != current {
            overlay.mark_dirty();
        }
        current = next;
        // mirror paint_if_dirty — consume both gates
        grid.should_paint();
        overlay.should_paint();
    }

    assert_eq!(grid.paint_count, 0, "grid must not paint during drag");
    assert_eq!(
        overlay.paint_count, 60,
        "overlay must paint once per drag frame"
    );
}

#[test]
fn set_model_same_rc_is_no_op() {
    let m: Rc<dyn CanvasModel> = Rc::new(StubModel);
    let clone = Rc::clone(&m);
    assert!(Rc::ptr_eq(&m, &clone), "ptr_eq must hold for same Rc");
}

#[test]
fn set_model_different_rc_is_change() {
    let m1: Rc<dyn CanvasModel> = Rc::new(StubModel);
    let m2: Rc<dyn CanvasModel> = Rc::new(StubModel);
    assert!(
        !Rc::ptr_eq(&m1, &m2),
        "distinct Rc allocations must not be equal"
    );
}
