//! Stage 1 fingerprint paint-skip — `render_pane` must emit zero `DrawOp`s
//! when the bulk-fetched buffers content-match the prior frame (under
//! `frame.kind == FrameKindTag::SlotsReused`), and must repaint exactly
//! the pane whose fingerprint changed.
//!
//! These tests target `render_pane` directly rather than `render_grid` so
//! the assertion surface stays the 4-pass per-pane walk. Header strips,
//! corner box, and frozen separators run above in `render_grid` and are
//! not fingerprint-gated.

use std::cell::RefCell;
use std::collections::HashMap;

use ironcalc_base::types::{CellType, Style};

use iron_canvas_core::chrome::{Chrome, FrameKindTag, PaneRegion};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_recorder::RecorderPainter;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, CanvasSize, CanvasView, RCRange};

/// Stateful model: cell values live in a `RefCell<HashMap>` so a test can
/// mutate one cell between paints without rebuilding the frame.
#[derive(Default)]
struct MutableModel {
    cells: RefCell<HashMap<(i32, i32), String>>,
    frozen_cols: i32,
}

impl MutableModel {
    fn set_cell(&self, row: i32, col: i32, value: &str) {
        self.cells
            .borrow_mut()
            .insert((row, col), value.to_string());
    }
}

impl CanvasModel for MutableModel {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from([1, 1, 1, 1]),
            top_row: 1,
            left_column: 1,
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(self.frozen_cols)
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
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<Style> {
        Some(Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<CellType> {
        Some(CellType::Text)
    }
    fn get_formatted_cell_value(&self, _: u32, row: i32, col: i32) -> Option<String> {
        Some(
            self.cells
                .borrow()
                .get(&(row, col))
                .cloned()
                .unwrap_or_default(),
        )
    }
}

fn canvas() -> CanvasSize {
    CanvasSize { w: 600.0, h: 400.0 }
}

/// Paint one pane through a fresh `RecorderPainter` and return its op
/// count. A new core per call keeps the recorder log isolated; the
/// fingerprint state lives on `Chrome` and survives across cores.
fn paint_pane(model: &MutableModel, frame: &Chrome, pane: PaneRegion) -> usize {
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_pane(model, pane, frame);
    let count = core.painter().ops().len();
    count
}

/// Mirrors the orchestrator's `SlotsReuse` branch: rotate the painted
/// fingerprints into `prev_pane_fingerprints` and flip the kind tag, so
/// the next `render_pane` call hits the skip-comparison branch.
fn promote_to_slots_reuse(frame: &mut Chrome) {
    frame.prev_pane_fingerprints = frame.pane_fingerprints.replace([0; 4]);
    frame.kind = FrameKindTag::SlotsReused;
}

#[test]
fn render_pane_skips_on_idempotent_repaint() {
    let m = MutableModel::default();
    let theme = CanvasTheme::light();
    let mut frame = Chrome::next(None, &m, canvas(), &theme, iron_canvas_core::chrome::FramePath::Fresh);

    // First paint runs through the full 4-pass walk; the kind is Fresh,
    // so the skip branch is gated off but `pane_fingerprints` is still
    // populated for the next frame.
    let first = paint_pane(&m, &frame, PaneRegion::BottomRight);
    assert!(first > 0, "first paint of a non-empty pane must emit ops");

    promote_to_slots_reuse(&mut frame);

    // Model unchanged ⇒ identical bulk-fetch buffers ⇒ identical
    // fingerprint ⇒ the entire 4-pass walk is skipped. Recorder log
    // must be byte-empty.
    let second = paint_pane(&m, &frame, PaneRegion::BottomRight);
    assert_eq!(
        second, 0,
        "idempotent repaint under SlotsReused must skip render_pane entirely",
    );
}

#[test]
fn render_pane_skip_is_scoped_to_changed_pane() {
    // `frozen_cols = 2` splits the data-bearing region: BottomLeft owns
    // cols 1..=2, BottomRight owns cols 3..=. A mutation in one pane
    // must leave the other pane's fingerprint untouched.
    let m = MutableModel {
        frozen_cols: 2,
        ..Default::default()
    };
    let theme = CanvasTheme::light();
    let mut frame = Chrome::next(None, &m, canvas(), &theme, iron_canvas_core::chrome::FramePath::Fresh);

    // Prime the per-pane fingerprints for both data-bearing panes.
    let _ = paint_pane(&m, &frame, PaneRegion::BottomLeft);
    let _ = paint_pane(&m, &frame, PaneRegion::BottomRight);

    promote_to_slots_reuse(&mut frame);

    // Col 5 lives past the frozen seam → BottomRight only.
    m.set_cell(1, 5, "changed");

    let bl_after = paint_pane(&m, &frame, PaneRegion::BottomLeft);
    let br_after = paint_pane(&m, &frame, PaneRegion::BottomRight);

    assert_eq!(
        bl_after, 0,
        "unaffected pane must skip — per-pane fingerprint is the load-bearing claim",
    );
    assert!(br_after > 0, "mutated pane must repaint");
}
