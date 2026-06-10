//! `iron-canvas-datagrid-web` — `#[wasm_bindgen]` bindings that render a
//! JS-supplied data grid through iron-canvas with ZERO IronCalc.
//!
//! ## Coordinate contract
//! The JS-facing handle is **uniformly 0-based** for row/col. This handle is
//! the single translation seam to the mixed-base pure model:
//! - model `set_selection`/`set_active`/`set_scroll` take **1-based** display
//!   coords → add 1 here;
//! - model `set_cell`/`set_column_width` take **0-based** → pass JS through;
//! - engine `HitTest::Cell`/`ResizeTarget::*Edge` are **1-based** → the wire
//!   mirrors subtract 1 before emitting to JS.
//!
//! ## Live / async data
//! wasm is single-threaded: there is no Rust async runtime here. "Live data"
//! means the consumer calls these mutators from any JS callback (`fetch`,
//! WebSocket, `setInterval`) and drives a `requestAnimationFrame` loop that
//! calls `paintIfDirty()`. The bindings never block; `paint_if_dirty` is a
//! no-op when clean.

use std::rc::Rc;

use iron_canvas_canvas2d::WebSurface;
use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasModel, CanvasTheme, Layer, Orchestrator};
use iron_canvas_datagrid::SortDirection;
use iron_canvas_export::SvgSurface;
use wasm_bindgen::prelude::*;

pub mod hover;
pub mod model_cell;
pub mod wire;

use hover::HoverLayer;
use model_cell::DataGridModel;

#[wasm_bindgen]
pub struct DataGridCanvas {
    orch: Orchestrator<WebSurface>,
    model: Rc<DataGridModel>,
    hover: Rc<HoverLayer>,
}

#[wasm_bindgen]
impl DataGridCanvas {
    #[wasm_bindgen(constructor)]
    pub fn new(
        grid_canvas: web_sys::HtmlCanvasElement,
        overlay_canvas: web_sys::HtmlCanvasElement,
    ) -> Result<DataGridCanvas, JsValue> {
        let model = Rc::new(DataGridModel::empty());
        let mut orch = Orchestrator::<WebSurface>::new(
            WebSurface::grid(grid_canvas)?,
            WebSurface::overlay(overlay_canvas)?,
        );
        orch.set_model(Rc::clone(&model) as Rc<dyn CanvasModel>);
        let hover = Rc::new(HoverLayer::default());
        orch.add_decoration(Rc::clone(&hover) as Rc<dyn Layer>);
        Ok(Self { orch, model, hover })
    }

    // --- E.1 Theming ---

    #[wasm_bindgen(js_name = "setThemeFromElement")]
    pub fn set_theme_from_element(&mut self, el: &web_sys::Element) {
        self.orch
            .set_theme(iron_canvas_canvas2d::theme_from_element::from_element(el));
        self.orch.request_repaint();
    }

    #[wasm_bindgen(js_name = "setThemeName")]
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = match name {
            "dark" => CanvasTheme::dark(),
            _ => CanvasTheme::light(),
        };
        self.orch.set_theme(theme);
        self.orch.request_repaint();
    }

    // --- E.2 Optional frozen header row (default OFF) ---

    #[wasm_bindgen(js_name = "setFrozenHeader")]
    pub fn set_frozen_header(&mut self, on: bool) {
        self.model.borrow_mut_with(|g| g.set_frozen_header(on));
        self.orch.request_repaint(); // freeze count is structural geometry
    }

    #[wasm_bindgen(js_name = "setData")]
    pub fn set_data(&mut self, data: JsValue) -> Result<(), JsValue> {
        let wire: wire::GridDataWire = serde_wasm_bindgen::from_value(data)?;
        self.model.replace(wire.into_model());
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
        self.orch.request_repaint();
        Ok(())
    }

    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.orch
            .resize(CanvasSize { w: css_w, h: css_h }, dpr as i32);
    }

    #[wasm_bindgen(js_name = "paintIfDirty")]
    pub fn paint_if_dirty(&mut self) {
        self.orch.paint_if_dirty();
    }

    // --- D.1 Scrolling ---

    #[wasm_bindgen(js_name = "setScroll")]
    pub fn set_scroll(&mut self, top_row: i32, left_col: i32) {
        self.model
            .borrow_mut_with(|g| g.set_scroll(top_row + 1, left_col + 1)); // 0→1 based
        self.orch.request_repaint();
    }

    #[wasm_bindgen(js_name = "scrollBy")]
    pub fn scroll_by(&mut self, d_rows: i32, d_cols: i32) {
        self.model.borrow_mut_with(|g| g.scroll_by(d_rows, d_cols)); // delta, no offset
        self.orch.request_repaint();
    }

    // --- D.2 Selection + hit-test ---

    #[wasm_bindgen(js_name = "hitTest")]
    pub fn hit_test(&self, x: f64, y: f64) -> Result<JsValue, JsValue> {
        let wire = wire::HitTestWire::from(self.orch.hit_test(x, y));
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    #[wasm_bindgen(js_name = "selectCell")]
    pub fn select_cell(&mut self, row: i32, col: i32) {
        self.model.borrow_mut_with(|g| {
            g.set_active(row + 1, col + 1);
            g.set_selection(row + 1, col + 1, row + 1, col + 1);
        });
        self.orch.request_overlay_repaint();
    }

    #[wasm_bindgen(js_name = "setSelection")]
    pub fn set_selection(&mut self, r1: i32, c1: i32, r2: i32, c2: i32) {
        self.model
            .borrow_mut_with(|g| g.set_selection(r1 + 1, c1 + 1, r2 + 1, c2 + 1));
        self.orch.request_overlay_repaint();
    }

    /// Hover-highlight a cell (0-based); any negative coordinate clears.
    /// Drives the custom `HoverLayer` decoration — compare-then-raise, so
    /// pointer-move spam on one cell costs no repaint.
    #[wasm_bindgen(js_name = "setHover")]
    pub fn set_hover(&mut self, row: i32, col: i32) {
        if self.hover.set_cell(HoverLayer::cell_from_js(row, col)) {
            self.orch.request_overlay_repaint();
        }
    }

    // --- D.3 Column resize ---

    #[wasm_bindgen(js_name = "resizeHandleAt")]
    pub fn resize_handle_at(&self, x: f64, y: f64, tol: f64) -> Result<JsValue, JsValue> {
        match self.orch.resize_handle_at(x, y, tol) {
            Some(t) => Ok(serde_wasm_bindgen::to_value(
                &wire::ResizeTargetWire::from(t),
            )?),
            None => Ok(JsValue::NULL),
        }
    }

    #[wasm_bindgen(js_name = "setColumnWidth")]
    pub fn set_column_width(&mut self, col: i32, width: f64) {
        if col < 0 {
            return;
        }
        self.model
            .borrow_mut_with(|g| g.set_column_width(col as usize, width));
        self.orch.request_repaint(); // geometry changed → Fresh rebuild
    }

    // --- D.4 Sort ---

    #[wasm_bindgen(js_name = "sortByColumn")]
    pub fn sort_by_column(&mut self, col: i32, ascending: bool) {
        if col < 0 {
            return;
        }
        let dir = if ascending {
            SortDirection::Ascending
        } else {
            SortDirection::Descending
        };
        self.model.borrow_mut_with(|g| g.sort_by(col as usize, dir));
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
        self.orch.request_repaint();
    }

    #[wasm_bindgen(js_name = "clearSort")]
    pub fn clear_sort(&mut self) {
        self.model.borrow_mut_with(|g| g.clear_sort());
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
        self.orch.request_repaint();
    }

    #[wasm_bindgen(js_name = "currentSort")]
    pub fn current_sort(&self) -> Result<JsValue, JsValue> {
        match self.model.borrow_current_sort() {
            Some((column, ascending)) => Ok(serde_wasm_bindgen::to_value(&wire::SortWire {
                column,
                ascending,
            })?),
            None => Ok(JsValue::NULL),
        }
    }

    // --- D.5 Live-update mutators ---

    #[wasm_bindgen(js_name = "setCell")]
    pub fn set_cell(&mut self, row: i32, col: i32, value: String) {
        if row < 0 || col < 0 {
            return;
        }
        self.model
            .borrow_mut_with(|g| g.set_cell(row as usize, col as usize, value)); // model set_cell is 0-based
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
        self.orch.request_repaint();
    }

    #[wasm_bindgen(js_name = "appendRows")]
    pub fn append_rows(&mut self, rows: JsValue) -> Result<(), JsValue> {
        let rows: Vec<Vec<String>> = serde_wasm_bindgen::from_value(rows)?;
        self.model.borrow_mut_with(|g| {
            for r in rows {
                g.append_row(r);
            }
        });
        self.orch.mark_content_dirty(PaneRegionMask::ALL);
        self.orch.request_repaint();
        Ok(())
    }

    #[wasm_bindgen(js_name = "exportSvg")]
    pub fn export_svg(&self, css_w: f64, css_h: f64) -> String {
        // `&self` is fine: `SvgSurface::render` clones the model `Rc` and
        // drives its own throwaway orchestrator — `self` is never mutated.
        SvgSurface::render(
            Rc::clone(&self.model) as Rc<dyn CanvasModel>,
            self.orch.theme(),
            CanvasSize { w: css_w, h: css_h },
        )
    }
}
