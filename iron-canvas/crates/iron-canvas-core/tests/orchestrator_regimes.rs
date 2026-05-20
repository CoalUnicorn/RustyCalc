//! `Orchestrator<MemSurface, _>` four-regime integration test.
//!
//! Drives all four `PaintRegime` arms (`Fresh`, `SlotsReuse`, `Viewport`,
//! `Overlay`) through the same dispatch entry point a browser would use,
//! and asserts the captured `DrawOp` log matches each regime's contract:
//!
//! - **Fresh**: full-canvas fill on the grid surface.
//! - **SlotsReuse**: no full-canvas fill (prior pixels are reused).
//! - **Viewport**: `DrawOp::Blit` ops on the grid surface (scroll-blit).
//! - **Overlay**: zero new grid ops; overlay surface clears + repaints.

#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::rc::Rc;

use ironcalc_base::types::{CellType, Style};

use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::orchestrator::Orchestrator;
use iron_canvas_core::types::coord::{AutofillTarget, RCRange};
use iron_canvas_core::{CanvasModel, CanvasTheme, CanvasView};

use iron_canvas_recorder::{DrawOp, MemSurface};

/// In-memory model whose scroll position and active cell can be mutated
/// from the test via `Cell` interior mutability while held as `Rc<Self>`
/// by the Orchestrator. Otherwise returns enough flat data to render a
/// uniform grid (20px rows × 80px cols, no frozen panes, default style).
#[derive(Default)]
struct ScrollableStub {
    top_row: Cell<i32>,
    left_column: Cell<i32>,
    active_row: Cell<i32>,
    active_col: Cell<i32>,
}

impl ScrollableStub {
    fn new() -> Self {
        Self {
            top_row: Cell::new(1),
            left_column: Cell::new(1),
            active_row: Cell::new(1),
            active_col: Cell::new(1),
        }
    }
    fn set_top_row(&self, row: i32) {
        self.top_row.set(row);
    }
}

impl CanvasModel for ScrollableStub {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        let r = self.active_row.get();
        let c = self.active_col.get();
        Some(CanvasView {
            sheet: 0,
            row: r,
            column: c,
            selection: RCRange {
                r1: r,
                c1: c,
                r2: r,
                c2: c,
            },
            top_row: self.top_row.get(),
            left_column: self.left_column.get(),
        })
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
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<Style> {
        Some(Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<CellType> {
        Some(CellType::Number)
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Option<String> {
        Some(String::new())
    }
}

fn build(model: Rc<ScrollableStub>) -> Orchestrator<MemSurface, Rc<ScrollableStub>> {
    let mut orch =
        Orchestrator::<MemSurface, Rc<ScrollableStub>>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1);
    orch.set_model(model);
    orch
}

fn grid_ops_len<M: CanvasModel>(orch: &Orchestrator<MemSurface, M>) -> usize {
    orch.grid.surface.recorder().ops().len()
}
fn overlay_ops_len<M: CanvasModel>(orch: &Orchestrator<MemSurface, M>) -> usize {
    orch.overlay.surface.recorder().ops().len()
}
fn grid_ops_since<M: CanvasModel>(
    orch: &Orchestrator<MemSurface, M>,
    cursor: usize,
) -> Vec<DrawOp> {
    orch.grid.surface.recorder().ops()[cursor..].to_vec()
}
fn overlay_ops_since<M: CanvasModel>(
    orch: &Orchestrator<MemSurface, M>,
    cursor: usize,
) -> Vec<DrawOp> {
    orch.overlay.surface.recorder().ops()[cursor..].to_vec()
}

#[test]
fn fresh_regime_emits_canvas_fill_and_overlay_clear() {
    let stub = Rc::new(ScrollableStub::new());
    let mut orch = build(Rc::clone(&stub));

    orch.paint_if_dirty();

    let grid_ops = orch.grid.surface.recorder().ops();
    let overlay_ops = orch.overlay.surface.recorder().ops();
    assert!(!grid_ops.is_empty(), "Fresh must paint the grid");
    assert!(!overlay_ops.is_empty(), "Fresh must paint the overlay");

    // The grid layer's full-canvas bg fill runs only on Fresh; SlotsReuse
    // / Blitted paths preserve prior pixels.
    assert!(
        grid_ops.iter().any(|op| matches!(
            op,
            DrawOp::RectFill { rect, .. } if rect.width >= 800 && rect.height >= 600
        )),
        "Fresh must emit a full-canvas RectFill (the grid bg)"
    );
    // Overlay clears its canvas at frame start (every regime that paints it).
    assert!(
        overlay_ops
            .iter()
            .any(|op| matches!(op, DrawOp::ClearRect { .. })),
        "Fresh must clear the overlay canvas"
    );
}

#[test]
fn slots_reuse_regime_skips_full_canvas_fill() {
    let stub = Rc::new(ScrollableStub::new());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame.

    let grid_before = grid_ops_len(&orch);

    // A theme change keeps the viewport stable → validity = SlotsReuse.
    // The decide cascade routes here because grid_dirty (STRUCTURAL is
    // raised) blocks the Overlay arm and screen_for_blit returns None
    // (no scroll).
    orch.set_theme(CanvasTheme::dark());
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert!(!new_grid_ops.is_empty(), "SlotsReuse must repaint the grid");
    // No full-canvas bg fill: SlotsReuse path skips it by design.
    assert!(
        !new_grid_ops.iter().any(|op| matches!(
            op,
            DrawOp::RectFill { rect, .. } if rect.width >= 800 && rect.height >= 600
        )),
        "SlotsReuse must NOT emit a full-canvas RectFill"
    );
}

#[test]
fn viewport_regime_emits_blit_op() {
    let stub = Rc::new(ScrollableStub::new());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);

    // Scroll one row. No content change. Raise OVERLAY (the only typed
    // signal we have for "something happened") so paint_if_dirty doesn't
    // bail empty — last_frame stays populated, decide() catches the
    // viewport shift via screen_for_blit and routes to Viewport.
    stub.set_top_row(2);
    orch.request_overlay_repaint();
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert!(
        new_grid_ops
            .iter()
            .any(|op| matches!(op, DrawOp::Blit { .. })),
        "Viewport regime must emit at least one DrawOp::Blit; got {:?}",
        new_grid_ops
    );
}

#[test]
fn overlay_regime_leaves_grid_untouched() {
    let stub = Rc::new(ScrollableStub::new());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);

    // Autofill drag: raises OVERLAY only, no grid signal. Viewport
    // unchanged → validity = SlotsReuse. decide() picks Overlay.
    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    let new_overlay_ops = overlay_ops_since(&orch, overlay_before);
    assert!(
        new_grid_ops.is_empty(),
        "Overlay regime must NOT touch the grid surface; got {:?}",
        new_grid_ops
    );
    assert!(
        !new_overlay_ops.is_empty(),
        "Overlay regime must repaint the overlay"
    );
    assert!(
        new_overlay_ops
            .iter()
            .any(|op| matches!(op, DrawOp::ClearRect { .. })),
        "Overlay regime must clear the overlay canvas"
    );
}

#[test]
fn empty_signals_short_circuit_paint_if_dirty() {
    let stub = Rc::new(ScrollableStub::new());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);

    // No signals raised since the prior paint — paint_if_dirty must bail.
    orch.paint_if_dirty();

    assert_eq!(grid_ops_len(&orch), grid_before);
    assert_eq!(overlay_ops_len(&orch), overlay_before);
}

#[test]
fn content_dirty_invalidates_pane_cache_through_slots_reuse() {
    let stub = Rc::new(ScrollableStub::new());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);

    // mark_content_dirty(ALL) raises CONTENT — viewport stays valid so
    // decide() picks SlotsReuse with mask = ALL.
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert!(
        !new_grid_ops.is_empty(),
        "Content-dirty SlotsReuse must repaint the grid"
    );
    assert!(
        !new_grid_ops.iter().any(|op| matches!(
            op,
            DrawOp::RectFill { rect, .. } if rect.width >= 800 && rect.height >= 600
        )),
        "Content-dirty SlotsReuse must NOT full-canvas-fill"
    );
}
