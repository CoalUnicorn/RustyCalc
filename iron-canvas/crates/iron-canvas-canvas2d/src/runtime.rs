//! Shared ownership for a Canvas2D grid/overlay pair.
//!
//! The renderer remains responsible for transaction policy. This type only
//! keeps the concrete browser resources that every Canvas2D host needs in the
//! same lifetime: two surfaces, their painter handles, canvas elements, and
//! the live DPR used by resize and playback.

use std::rc::Rc;

use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;

use crate::{CanvasPainter, WebSurface};

/// Canvas2D-backed runtime shared by the spreadsheet, data-grid, and camera
/// hosts. `S` may be `WebSurface` or a host-owned wrapper such as the recorder
/// surface; this crate never needs to know which one was selected.
pub struct Canvas2dRuntime<S>
where
    S: Surface,
{
    orchestrator: Orchestrator<S>,
    grid_canvas: HtmlCanvasElement,
    overlay_canvas: HtmlCanvasElement,
    grid_painter: Rc<CanvasPainter>,
    overlay_painter: Rc<CanvasPainter>,
    dpr: f64,
}

impl Canvas2dRuntime<WebSurface> {
    /// Construct a runtime with bare Canvas2D surfaces.
    pub fn new(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<Self, JsValue> {
        Self::new_with_wrapper(grid_canvas, overlay_canvas, |surface| surface)
    }
}

impl<S> Canvas2dRuntime<S>
where
    S: Surface,
{
    /// Construct a runtime and let the host wrap each surface after the raw
    /// Canvas2D handles have been retained. The wrapper is called once for the
    /// grid and once for the overlay, in that order.
    pub fn new_with_wrapper<F>(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
        mut wrap: F,
    ) -> Result<Self, JsValue>
    where
        F: FnMut(WebSurface) -> S,
    {
        let grid_canvas_handle = grid_canvas.clone();
        let overlay_canvas_handle = overlay_canvas.clone();
        let grid_surface = WebSurface::grid(grid_canvas)?;
        let overlay_surface = WebSurface::overlay(overlay_canvas)?;
        let grid_painter = grid_surface.clone_painter();
        let overlay_painter = overlay_surface.clone_painter();
        let grid = wrap(grid_surface);
        let overlay = wrap(overlay_surface);

        Ok(Self {
            orchestrator: Orchestrator::new(grid, overlay),
            grid_canvas: grid_canvas_handle,
            overlay_canvas: overlay_canvas_handle,
            grid_painter,
            overlay_painter,
            dpr: 1.0,
        })
    }

    pub fn orchestrator(&self) -> &Orchestrator<S> {
        &self.orchestrator
    }

    pub fn orchestrator_mut(&mut self) -> &mut Orchestrator<S> {
        &mut self.orchestrator
    }

    /// Resize both surfaces and keep the playback/recording DPR beside the
    /// same operation that updates the backing stores.
    pub fn resize(&mut self, size: CanvasSize, dpr: f64) {
        self.dpr = dpr;
        self.orchestrator.resize(size, dpr);
    }

    pub fn dpr(&self) -> f64 {
        self.dpr
    }

    /// Clear both Canvas2D measure caches and queue one content repaint. Font
    /// loading changes text metrics, so the invalidation belongs with the
    /// painter resources rather than with a generic core trace flag.
    pub fn fonts_changed(&mut self) {
        self.grid_painter.clear_measure_cache();
        self.overlay_painter.clear_measure_cache();
        self.orchestrator.mark_content_dirty();
    }

    pub fn grid_canvas(&self) -> &HtmlCanvasElement {
        &self.grid_canvas
    }

    pub fn overlay_canvas(&self) -> &HtmlCanvasElement {
        &self.overlay_canvas
    }
}
