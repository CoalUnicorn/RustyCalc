//! Per-camera paint stack: `Orchestrator<WebSurface>` + `DataGridModel`,
//! the same composition as DataGridCanvas in iron-canvas-datagrid-web
//! but driven natively — full CellStyle fidelity, no JS wire.

use std::rc::Rc;

use iron_canvas_canvas2d::{Canvas2dRuntime, WebSurface};
use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasModel, PaintResult};
use iron_canvas_datagrid::{DataGrid, DataGridModel};
use leptos::prelude::window;
use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

pub struct CameraCanvas {
    runtime: Canvas2dRuntime<WebSurface>,
    model: Rc<DataGridModel>,
}

impl CameraCanvas {
    pub fn create(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<Self, JsValue> {
        let model = Rc::new(DataGridModel::empty());
        let mut runtime = Canvas2dRuntime::new(grid_canvas, overlay_canvas)?;
        runtime
            .orchestrator_mut()
            .set_model(Rc::clone(&model) as Rc<dyn CanvasModel>);
        let mut cam = Self { runtime, model };
        cam.sync_theme_from_document();
        Ok(cam)
    }

    pub fn set_grid(&mut self, grid: DataGrid) {
        self.model.replace(grid);
        self.runtime
            .orchestrator_mut()
            .mark_content_dirty(PaneRegionMask::ALL);
        self.runtime.orchestrator_mut().request_repaint();
    }

    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.runtime.resize(CanvasSize { w: css_w, h: css_h }, dpr);
    }

    /// Scroll and return the clamped anchors, so callers can persist the
    /// position the grid actually landed on. Scrolls repaint without a
    /// content-dirty mark, matching DataGridCanvas.
    pub fn scroll_by(&mut self, d_rows: i32, d_cols: i32) -> (i32, i32) {
        let anchors = self.model.borrow_mut_with(|g| {
            g.scroll_by(d_rows, d_cols);
            g.scroll_anchors()
        });
        self.runtime.orchestrator_mut().view_changed();
        anchors
    }

    /// `top_row`/`left_col` are 1-based (DataGrid's native convention; the
    /// facade's +1 is JS 0-based translation, not needed here).
    pub fn set_scroll(&mut self, top_row: i32, left_col: i32) {
        self.model
            .borrow_mut_with(|g| g.set_scroll(top_row, left_col));
        self.runtime.orchestrator_mut().view_changed();
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
            if let Some(w) =
                self.runtime
                    .orchestrator()
                    .fit_column_width(col as i32 + 1, 1, rows as i32)
            {
                self.model.borrow_mut_with(|g| g.set_column_width(col, w));
            }
        }
        self.runtime.orchestrator_mut().request_repaint();
        self.model.borrow_with(|g| g.content_extent())
    }

    pub fn sync_theme_from_document(&mut self) {
        if let Some(el) = window().document().and_then(|d| d.document_element()) {
            self.runtime
                .orchestrator_mut()
                .set_theme(iron_canvas_canvas2d::theme_from_element::from_element(&el));
        }
    }

    pub fn paint_if_dirty(&mut self) -> PaintResult {
        self.runtime.orchestrator_mut().paint_if_dirty()
    }

    /// See `IronCanvas::fontsChanged` — clear text-measure memos after
    /// `document.fonts` finishes loading, then mark the content dirty. The
    /// loading listener also pokes the one-shot camera scheduler.
    pub fn fonts_changed(&mut self) {
        self.runtime.fonts_changed();
    }
}
