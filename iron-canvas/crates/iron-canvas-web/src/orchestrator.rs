//! Web-side facade. The frame dispatch + state aggregator now lives in
//! `iron_canvas_core::Orchestrator`; this struct holds the `wasm-bindgen`
//! handle, builds two `WebSurface`s, and delegates every setter / query /
//! paint call to the core orchestrator.

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::layer::RenderOverlays;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::wasm::JsBackedModel;
use crate::web_surface::WebSurface;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::orchestrator::Orchestrator;
use iron_canvas_core::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};
use iron_canvas_core::CanvasModel;

#[wasm_bindgen]
pub struct IronCanvas {
    orch: Orchestrator<WebSurface, Rc<dyn CanvasModel>>,
}

#[wasm_bindgen]
impl IronCanvas {
    /// Construct over two stacked canvases. CSS stacking (`position:
    /// absolute`, correct `z-index`, `pointer-events: none` on the
    /// overlay) is the caller's responsibility.
    pub fn create(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<IronCanvas, JsValue> {
        let grid = WebSurface::grid(grid_canvas)?;
        let overlay = WebSurface::overlay(overlay_canvas)?;
        Ok(IronCanvas {
            orch: Orchestrator::<WebSurface, Rc<dyn CanvasModel>>::new(grid, overlay),
        })
    }

    /// Resize both layers in one call.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.orch
            .resize(CanvasSize { w: css_w, h: css_h }, dpr.round() as i32);
    }

    /// Push a theme by name. Only `"dark"` is recognized; every other
    /// value (including `"light"` and anything misspelled) maps to the
    /// light palette.
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = if name == "dark" {
            CanvasTheme::dark()
        } else {
            CanvasTheme::light()
        };
        self.orch.set_theme(theme);
    }

    /// Conservative repaint blanket — see `Orchestrator::request_repaint`.
    #[allow(non_snake_case)]
    pub fn requestRepaint(&mut self) {
        self.orch.request_repaint();
    }

    /// JS-facing cell-content-changed signal — marks all four pane
    /// quadrants. Pane-granular masks stay Rust-internal.
    #[allow(non_snake_case)]
    pub fn markContentDirty(&mut self) {
        self.orch
            .mark_content_dirty(iron_canvas_core::chrome::PaneRegionMask::ALL);
    }

    /// Paint whichever layers are dirty.
    #[allow(non_snake_case)]
    pub fn paintIfDirty(&mut self) {
        self.orch.paint_if_dirty();
    }

    /// Explicit teardown for React strict-mode / Leptos `Effect` mount
    /// cycles. `Drop` already handles cleanup on scope exit; this just
    /// gives JS a named callsite for the `create -> drop -> create` dance.
    pub fn dispose(self) {}

    /// JS-facing model push. Adopts the IronCalc `Model` JS handle as an
    /// opaque `JsBackedModel` after the structural duck-test in
    /// `JsBackedModel::try_from_js_value`. Returns `JsError` so the JS
    /// catch sees a real `Error` with `.message` and `.stack`.
    #[allow(non_snake_case)]
    pub fn setModel(&mut self, model: JsValue) -> Result<(), JsError> {
        let backed = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.orch.set_model(backed);
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// JS-facing theme push from a host DOM node. Reads the upstream
    /// `--palette-*` custom properties off `el`'s computed style and
    /// builds a `CanvasTheme`.
    #[allow(non_snake_case)]
    pub fn setThemeFromElement(&mut self, el: &web_sys::Element) {
        self.orch
            .set_theme(crate::theme_from_element::from_element(el));
    }
}

// Rust-only API. Counterpart to the `#[wasm_bindgen]` block above —
// these methods take Rust types that don't cross the JS bridge.
impl IronCanvas {
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        self.orch.set_overlays(overlays);
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        self.orch.set_extend_to(target);
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        self.orch.set_clipboard(area);
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        self.orch.set_point_range(range);
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        self.orch.set_formula_refs(refs);
    }

    pub fn set_theme(&mut self, theme: CanvasTheme) {
        self.orch.set_theme(theme);
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.orch.set_theme_variables(vars);
    }

    /// Rust-level model push. Accepts any `CanvasModel` impl behind an
    /// `Rc` — Leptos-side adapters that bridge a host store to the canvas
    /// (e.g. `WorksheetModelAdapter`) route through here.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.orch.set_model(model);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.orch.canvas_size()
    }

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        self.orch.hit_test(x, y)
    }

    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.orch.resize_handle_at(x, y, tolerance)
    }

    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.orch.cell_rect(row, column)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.orch.autofill_handle()
    }

    pub fn request_overlay_repaint(&mut self) {
        self.orch.request_overlay_repaint();
    }
}
