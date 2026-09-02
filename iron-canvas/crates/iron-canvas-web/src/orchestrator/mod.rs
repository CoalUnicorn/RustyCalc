//! This module contains the web facade for the canvas renderer.
//! `iron_canvas_core::Orchestrator` controls frame dispatch and frame state.
//! `IronCanvas` owns the `wasm-bindgen` handle and the Canvas2D runtime.
//! It delegates setters, queries, recording, and playback.

mod export;
#[cfg(target_arch = "wasm32")]
mod js_api;
#[cfg(feature = "dev-tools")]
mod playback_api;
mod recording;

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::RenderOverlays;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::wasm::JsBackedModel;
use iron_canvas_canvas2d::{Canvas2dRuntime, WebSurface};
use iron_canvas_core::CanvasModel;
use iron_canvas_core::PaintResult;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::RecordingSurface;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::recording::RecordOrigin;

#[cfg(feature = "dev-tools")]
use recording::CanvasMode;

/// The surface type for the web facade.
///
/// A development build uses `RecordingSurface<WebSurface>`.
/// A production build uses `WebSurface`.
/// The `Surface` trait hides this selection from the other components.
#[cfg(feature = "dev-tools")]
type FacadeSurface = RecordingSurface<WebSurface>;
#[cfg(not(feature = "dev-tools"))]
type FacadeSurface = WebSurface;

#[cfg(feature = "dev-tools")]
fn wrap_surface(s: WebSurface) -> FacadeSurface {
    RecordingSurface::new(s)
}
#[cfg(not(feature = "dev-tools"))]
fn wrap_surface(s: WebSurface) -> FacadeSurface {
    s
}

/// The result of one `renderPending()` call.
///
/// The first three variants match `iron_canvas_core::PaintResult`.
/// The `PlaybackActive` variant shows that playback bypassed the core orchestrator.
/// This enum does not allocate a `String` for each frame.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenderResult {
    Idle,
    Rendered,
    RetryRequired,
    /// Playback bypassed the core paint operation for this tick.
    PlaybackActive,
}

#[wasm_bindgen]
pub struct IronCanvas {
    runtime: Canvas2dRuntime<FacadeSurface>,
    // SVG export supplies this model to a temporary orchestrator.
    // Each model setter updates this value.
    model: Option<Rc<dyn CanvasModel>>,
    // This value exists only for a model that comes from `set_model_js`.
    // `themeChanged` must call `JsBackedModel::theme_changed`.
    // The type-erased `Rc<dyn CanvasModel>` cannot call this method.
    // A Rust model sets this value to `None` because its host controls its theme.
    js_model: Option<Rc<JsBackedModel>>,
    #[cfg(feature = "dev-tools")]
    mode: CanvasMode,
}

#[wasm_bindgen]
impl IronCanvas {
    /// Create an `IronCanvas` with two stacked canvases.
    ///
    /// The caller must configure the CSS position and `z-index` values.
    /// The caller must set `pointer-events: none` on the overlay.
    pub fn create(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<IronCanvas, JsValue> {
        let runtime = Canvas2dRuntime::new_with_wrapper(grid_canvas, overlay_canvas, wrap_surface)?;
        Ok(IronCanvas {
            runtime,
            model: None,
            js_model: None,
            #[cfg(feature = "dev-tools")]
            mode: CanvasMode::Live,
        })
    }

    /// Resize both layers in one call.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.runtime.resize(CanvasSize { w: css_w, h: css_h }, dpr);
    }

    /// Set the theme from its name.
    ///
    /// The value `"dark"` selects the dark palette.
    /// All other values select the light palette.
    #[wasm_bindgen(js_name = "setThemeName")]
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = if name == "dark" {
            CanvasTheme::dark()
        } else {
            CanvasTheme::light()
        };
        self.runtime.orchestrator_mut().set_theme(theme);
        self.restamp_recording_theme();
    }

    /// Request a full repaint.
    #[wasm_bindgen(js_name = "requestRepaint")]
    pub fn request_repaint(&mut self) {
        self.runtime.orchestrator_mut().request_repaint();
    }

    /// Report a content change that can affect the full grid.
    #[wasm_bindgen(js_name = "markContentDirty")]
    pub fn mark_content_dirty(&mut self) {
        self.runtime.orchestrator_mut().mark_content_dirty();
    }

    /// Report content changes in an inclusive row range.
    ///
    /// The renderer limits the repaint to the applicable row bands.
    /// `RowSpan::normalized` accepts the row values in either order.
    /// The renderer uses the row coordinates from the model bridge.
    /// Rows outside the viewport do not cause paint work.
    /// Incomplete damage information causes a full content repaint.
    #[wasm_bindgen(js_name = "markRowsDamaged")]
    pub fn mark_rows_damaged(&mut self, sheet: u32, row_start: i32, row_end: i32) {
        self.runtime.orchestrator_mut().mark_rows_damaged(
            sheet,
            iron_canvas_core::RowSpan {
                r1: row_start,
                r2: row_end,
            },
        );
    }

    /// Paint each layer that has pending work.
    ///
    /// During recording, this method captures each non-idle paint attempt.
    /// The capture includes a held attempt that has no paint operations.
    /// The method does not capture an idle rAF tick.
    /// The host uses `RetryRequired` to schedule another rAF tick.
    #[wasm_bindgen(js_name = "renderPending")]
    pub fn render_pending(&mut self) -> RenderResult {
        #[cfg(feature = "dev-tools")]
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return RenderResult::PlaybackActive;
        }
        #[cfg(feature = "dev-tools")]
        let recording_active = matches!(self.mode, CanvasMode::Recording(_));
        #[cfg(feature = "dev-tools")]
        if recording_active {
            self.runtime.orchestrator().grid_surface().begin_frame();
            self.runtime.orchestrator().overlay_surface().begin_frame();
        }

        let core_result = self.runtime.orchestrator_mut().render_pending();
        let result = match core_result {
            PaintResult::Idle => RenderResult::Idle,
            PaintResult::Rendered => RenderResult::Rendered,
            PaintResult::RetryRequired => RenderResult::RetryRequired,
        };

        #[cfg(feature = "dev-tools")]
        if recording_active {
            self.capture_frame(core_result, RecordOrigin::Live);
        }

        result
    }

    /// Return a one-line trace for the last paint attempt.
    ///
    /// The trace contains the strategy, the grid verdict, and the fetched slots.
    /// Read the trace after each rAF tick to identify expensive paint paths.
    #[wasm_bindgen(js_name = "frameTrace")]
    pub fn frame_trace(&self) -> String {
        #[cfg(feature = "dev-tools")]
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return String::new();
        }
        self.runtime.orchestrator().last_trace().to_string()
    }

    /// Consume this handle and run its normal drop operation.
    ///
    /// JavaScript hosts can use this method during repeated mount operations.
    pub fn dispose(self) {}

    /// Set the model from an IronCalc JavaScript handle.
    ///
    /// `JsBackedModel::try_from_js_value` checks the structure of the value.
    /// The method returns `JsError` if the value does not have the required API.
    #[wasm_bindgen(js_name = "setModel")]
    pub fn set_model_js(&mut self, model: JsValue) -> Result<(), JsError> {
        let backed = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.js_model = Some(Rc::clone(&backed));
        let erased: Rc<dyn CanvasModel> = backed;
        self.model = Some(Rc::clone(&erased));
        self.runtime.orchestrator_mut().set_model(erased);
        Ok(())
    }

    /// Report a workbook theme change.
    ///
    /// Call this method after `model.setTheme(...)`.
    /// The method clears the cached bridge theme and marks content as dirty.
    /// The next `renderPending` call fetches the styles again.
    /// For a Rust model, the method only marks content as dirty.
    #[wasm_bindgen(js_name = "themeChanged")]
    pub fn theme_changed(&mut self) {
        if let Some(m) = &self.js_model {
            m.theme_changed();
        }
        self.mark_content_dirty();
    }

    /// Report that the browser loaded new fonts.
    ///
    /// The method clears cached text measurements and requests a repaint.
    /// Use `addEventListener` because multiple canvases share `document.fonts`:
    /// `document.fonts.addEventListener("loadingdone", () => canvas.fontsChanged());`
    #[wasm_bindgen(js_name = "fontsChanged")]
    pub fn fonts_changed(&mut self) {
        self.runtime.fonts_changed();
    }
}

// This API accepts Rust types and does not cross the JavaScript boundary.
impl IronCanvas {
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        self.runtime.orchestrator_mut().set_overlays(overlays);
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        self.runtime.orchestrator_mut().set_extend_to(target);
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        self.runtime.orchestrator_mut().set_clipboard(area);
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        self.runtime.orchestrator_mut().set_point_range(range);
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        self.runtime.orchestrator_mut().set_formula_refs(refs);
    }

    pub fn set_theme(&mut self, theme: CanvasTheme) {
        self.runtime.orchestrator_mut().set_theme(theme);
        self.restamp_recording_theme();
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.runtime.orchestrator_mut().set_theme_variables(vars);
        self.restamp_recording_theme();
    }

    /// Set a Rust model.
    ///
    /// The method accepts any `CanvasModel` implementation in an `Rc`.
    /// Host adapters such as `WorksheetModelAdapter` use this method.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.model = Some(Rc::clone(&model));
        // A Rust model replaces the current JavaScript model.
        // The Rust host resolves the theme.
        self.js_model = None;
        self.runtime.orchestrator_mut().set_model(model);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.runtime.orchestrator().canvas_size()
    }

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        self.runtime.orchestrator().hit_test(x, y)
    }

    /// Find the cell at a pixel position without an overlay hit test.
    ///
    /// Use this method when an overlay owns the pointer during a drag.
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        self.runtime.orchestrator().pixel_to_cell(x, y)
    }

    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.runtime
            .orchestrator()
            .resize_handle_at(x, y, tolerance)
    }

    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.runtime.orchestrator().cell_rect(row, column)
    }

    pub fn scroll_pane_rect(&self) -> Option<PixelRect> {
        self.runtime.orchestrator().scroll_pane_rect()
    }

    pub fn legal_scroll_origin(&self) -> Option<(i32, i32)> {
        self.runtime.orchestrator().legal_scroll_origin()
    }

    pub fn scroll_to_show(&self, row: i32, column: i32) -> Option<(i32, i32)> {
        self.runtime.orchestrator().scroll_to_show(row, column)
    }

    pub fn fit_column_width(&self, col: i32, first_row: i32, last_row: i32) -> Option<f64> {
        self.runtime
            .orchestrator()
            .fit_column_width(col, first_row, last_row)
    }

    pub fn fit_row_height(&self, row: i32, first_col: i32, last_col: i32) -> Option<f64> {
        self.runtime
            .orchestrator()
            .fit_row_height(row, first_col, last_col)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.runtime.orchestrator().autofill_handle()
    }

    pub fn request_overlay_repaint(&mut self) {
        self.runtime.orchestrator_mut().request_overlay_repaint();
    }

    /// Report a change to the scroll position, selection, active cell, or sheet.
    ///
    /// This method marks the view and overlay in one operation.
    /// The next `render_pending` call selects the applicable paint strategy.
    pub fn view_changed(&mut self) {
        self.runtime.orchestrator_mut().view_changed();
    }
}
