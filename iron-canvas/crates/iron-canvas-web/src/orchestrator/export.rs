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
    /// The method throws if no model is set.
    /// A temporary orchestrator uses the current theme.
    /// The temporary orchestrator does not change the live surfaces.
    /// `SvgSurface::render` does not include the overlay.
    /// The method throws if the export paint attempt does not commit a frame.
    #[wasm_bindgen(js_name = "exportSvg")]
    pub fn export_svg(&self, css_w: f64, css_h: f64) -> Result<String, JsError> {
        let Some(model) = self.model.as_ref() else {
            return Err(JsError::new(
                "no model is set: call setModel before exportSvg",
            ));
        };
        SvgSurface::render(
            Rc::clone(model),
            self.runtime.orchestrator().theme(),
            CanvasSize { w: css_w, h: css_h },
        )
        .map_err(|error| JsError::new(&format!("SVG export failed: {error}")))
    }

    /// Render the current sheet as PDF data.
    ///
    /// The method throws if no model is set.
    /// `PdfSurface::render` does not include the overlay.
    /// The method throws if the export paint attempt does not commit a frame.
    /// `wasm-bindgen` converts `Vec<u8>` to `Uint8Array`.
    #[cfg(feature = "pdf")]
    #[wasm_bindgen(js_name = "exportPdf")]
    pub fn export_pdf(&self, css_w: f64, css_h: f64) -> Result<Vec<u8>, JsError> {
        let Some(model) = self.model.as_ref() else {
            return Err(JsError::new(
                "no model is set: call setModel before exportPdf",
            ));
        };
        PdfSurface::render(
            Rc::clone(model),
            self.runtime.orchestrator().theme(),
            CanvasSize { w: css_w, h: css_h },
        )
        .map_err(|error| JsError::new(&format!("PDF export failed: {error}")))
    }
}
