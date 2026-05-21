//! Web-side facade. The frame dispatch + state aggregator now lives in
//! `iron_canvas_core::Orchestrator`; this struct holds the `wasm-bindgen`
//! handle, builds two `WebSurface`s, and delegates every setter / query /
//! paint call to the core orchestrator.

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::theme::{CanvasTheme, ThemeVariables};
use crate::RenderOverlays;
use crate::wasm::JsBackedModel;
use crate::web_surface::WebSurface;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_core::Orchestrator;
use iron_canvas_core::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};
use iron_canvas_core::CanvasModel;
use iron_canvas_svg::SvgSurface;

#[wasm_bindgen]
pub struct IronCanvas {
    orch: Orchestrator<WebSurface, Rc<dyn CanvasModel>>,
    // Cached so SVG export can re-push the live model into a throwaway
    // orchestrator. Updated alongside every `set_model` / `setModel`.
    model: Option<Rc<dyn CanvasModel>>,
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
            model: None,
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
        let backed: Rc<dyn CanvasModel> = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.model = Some(Rc::clone(&backed));
        self.orch.set_model(backed);
        Ok(())
    }

    /// Render the current sheet as a self-contained SVG string. Returns
    /// an empty string if no model has been pushed yet. The export
    /// reads the live theme but uses a one-shot orchestrator — never
    /// touches the live grid / overlay surfaces and never fires blit
    /// (always `PaintRegime::Fresh`). Overlays (selection, marching
    /// ants, autofill handle, formula refs) are deliberately omitted
    /// — the overlay surface's SVG output is built but discarded.
    ///
    /// Why the overlay-discard strategy yields a clean grid SVG even
    /// though the throwaway orchestrator's `SelectionLayer` defaults to
    /// an A1 active cell: `LayerBase::paint_overlay_layer` invokes the
    /// `after_paint_renderer_hook` (active-cell repaint) through the
    /// **overlay** renderer's painter, not the grid's. The hook's output
    /// goes to the discarded overlay surface; the grid surface only
    /// receives `render_grid`'s cell / borders / chrome draws.
    #[allow(non_snake_case)]
    pub fn exportSvg(&self, css_w: f64, css_h: f64) -> String {
        let Some(model) = self.model.as_ref() else {
            return String::new();
        };
        let width = css_w.round() as i32;
        let height = css_h.round() as i32;

        let grid = SvgSurface::new(width, height);
        let overlay = SvgSurface::new(width, height);
        let grid_painter = grid.clone_painter();

        let mut export_orch =
            Orchestrator::<SvgSurface, Rc<dyn CanvasModel>>::new(grid, overlay);
        export_orch.set_theme(self.orch.theme().clone());
        export_orch.set_model(Rc::clone(model));
        export_orch.resize(CanvasSize { w: css_w, h: css_h }, 1);
        export_orch.request_repaint();
        export_orch.paint_if_dirty();
        drop(export_orch);

        grid_painter.finish()
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
        self.model = Some(Rc::clone(&model));
        self.orch.set_model(model);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.orch.canvas_size()
    }

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        self.orch.hit_test(x, y)
    }

    /// Layer-bypassing cell resolver. Use during an active drag whose
    /// overlay (e.g. `FormulaRefsLayer`) would otherwise claim the
    /// pointer and starve the host of underlying cell coordinates.
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        self.orch.pixel_to_cell(x, y)
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
