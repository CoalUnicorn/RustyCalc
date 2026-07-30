//! Per-camera paint stack: `Orchestrator<WebSurface>` + `DataGridModel`,
//! the same composition as DataGridCanvas in iron-canvas-datagrid-web
//! but driven natively — full CellStyle fidelity, no JS wire.

use std::rc::Rc;

use iron_canvas_canvas2d::{CanvasPainter, WebSurface};
use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_core::{CanvasModel, Orchestrator, PaintResult};
use iron_canvas_datagrid::{DataGrid, DataGridModel};
use leptos::prelude::window;
use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

pub struct CameraCanvas {
    orch: Orchestrator<WebSurface>,
    model: Rc<DataGridModel>,
    // Painter handles for fonts_changed, taken before the surfaces move
    // into the orchestrator — same pattern as IronCanvas/DataGridCanvas.
    grid_painter: Rc<CanvasPainter>,
    overlay_painter: Rc<CanvasPainter>,
}

impl CameraCanvas {
    pub fn create(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<Self, JsValue> {
        let model = Rc::new(DataGridModel::empty());
        let grid_ws = WebSurface::grid(grid_canvas)?;
        let overlay_ws = WebSurface::overlay(overlay_canvas)?;
        let grid_painter = grid_ws.clone_painter();
        let overlay_painter = overlay_ws.clone_painter();
        let mut orch = Orchestrator::<WebSurface>::new(grid_ws, overlay_ws);
        orch.set_model(Rc::clone(&model) as Rc<dyn CanvasModel>);
        let mut cam = Self {
            orch,
            model,
            grid_painter,
            overlay_painter,
        };
        cam.sync_theme_from_document();
        Ok(cam)
    }

    pub fn set_grid(&mut self, grid: DataGrid) {
        self.model.replace(grid);
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
        self.orch.request_repaint();
    }

    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.orch.resize(CanvasSize { w: css_w, h: css_h }, dpr);
    }

    /// Scroll and return the clamped anchors, so callers can persist the
    /// position the grid actually landed on. Scrolls repaint without a
    /// content-dirty mark, matching DataGridCanvas.
    pub fn scroll_by(&mut self, d_rows: i32, d_cols: i32) -> (i32, i32) {
        let anchors = self.model.borrow_mut_with(|g| {
            g.scroll_by(d_rows, d_cols);
            g.scroll_anchors()
        });
        self.orch.view_changed();
        anchors
    }

    /// `top_row`/`left_col` are 1-based (DataGrid's native convention; the
    /// facade's +1 is JS 0-based translation, not needed here).
    pub fn set_scroll(&mut self, top_row: i32, left_col: i32) {
        self.model
            .borrow_mut_with(|g| g.set_scroll(top_row, left_col));
        self.orch.view_changed();
    }

    /// Current 1-based scroll anchors `(top_row, left_col)` — the live viewport,
    /// captured before a re-extract so the user's scroll survives `set_grid`.
    pub fn scroll_anchors(&self) -> (i32, i32) {
        self.model.borrow_with(|g| g.scroll_anchors())
    }

    /// Fit every column to its text, then return the grid's natural pixel
    /// size. The caller owns applying it (widget chrome + clamping live there).
    pub fn autosize(&mut self) -> (f64, f64) {
        let (rows, cols) = self
            .model
            .borrow_with(|g| (g.row_count(), g.column_count()));
        for col in 0..cols {
            // 1-based model coords for the measure; 0-based index for the write.
            if let Some(w) = self.orch.fit_column_width(col as i32 + 1, 1, rows as i32) {
                self.model.borrow_mut_with(|g| g.set_column_width(col, w));
            }
        }
        self.orch.request_repaint();
        self.model.borrow_with(|g| g.content_extent())
    }

    pub fn sync_theme_from_document(&mut self) {
        if let Some(el) = window().document().and_then(|d| d.document_element()) {
            self.orch
                .set_theme(iron_canvas_canvas2d::theme_from_element::from_element(&el));
        }
    }

    pub fn paint_if_dirty(&mut self) -> PaintResult {
        self.orch.paint_if_dirty()
    }

    /// See `IronCanvas::fontsChanged` — clear text-measure memos after
    /// `document.fonts` finishes loading, then mark the content dirty. The
    /// loading listener also pokes the one-shot camera scheduler.
    pub fn fonts_changed(&mut self) {
        self.grid_painter.clear_measure_cache();
        self.overlay_painter.clear_measure_cache();
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
    }
}
