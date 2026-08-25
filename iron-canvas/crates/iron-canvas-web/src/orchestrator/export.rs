use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_export::SvgSurface;
#[cfg(feature = "pdf")]
use iron_canvas_export::pdf::PdfSurface;
use wasm_bindgen::prelude::*;

use super::IronCanvas;

#[wasm_bindgen]
impl IronCanvas {
    /// Render the current sheet as a self-contained SVG string.
    ///
    /// The method returns an empty string if no model is set.
    /// A temporary orchestrator uses the current theme.
    /// The temporary orchestrator does not change the live surfaces.
    /// `SvgSurface::render` does not include the overlay.
    #[wasm_bindgen(js_name = "exportSvg")]
    pub fn export_svg(&self, css_w: f64, css_h: f64) -> String {
        let Some(model) = self.model.as_ref() else {
            return String::new();
        };
        SvgSurface::render(
            Rc::clone(model),
            self.runtime.orchestrator().theme(),
            CanvasSize { w: css_w, h: css_h },
        )
    }

    /// Render the current sheet as PDF data.
    ///
    /// The method returns an empty vector if no model is set.
    /// `PdfSurface::render` does not include the overlay.
    /// `wasm-bindgen` converts `Vec<u8>` to `Uint8Array`.
    #[cfg(feature = "pdf")]
    #[wasm_bindgen(js_name = "exportPdf")]
    pub fn export_pdf(&self, css_w: f64, css_h: f64) -> Vec<u8> {
        let Some(model) = self.model.as_ref() else {
            return Vec::new();
        };
        PdfSurface::render(
            Rc::clone(model),
            self.runtime.orchestrator().theme(),
            CanvasSize { w: css_w, h: css_h },
        )
    }
}
