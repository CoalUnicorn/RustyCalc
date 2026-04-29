use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::theme::{CanvasTheme, LIGHT};
use crate::CanvasModel;
use crate::CanvasRenderer;

use super::PaintGate;

pub(crate) struct GridLayer {
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) css_width: f64,
    pub(crate) css_height: f64,
    pub(crate) dpr: f64,
    gate: PaintGate,
    /// Long-lived renderer; owns the layer's 2D ctx so paint caches persist
    /// across frames. Theme is hot-swapped per paint via `set_theme`.
    renderer: CanvasRenderer,
}

impl GridLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx_opts = js_sys::Object::new();
        js_sys::Reflect::set(&ctx_opts, &"alpha".into(), &JsValue::from(false))
            .map_err(|_| JsValue::from_str("failed to set grid context alpha option"))?;
        let ctx = canvas
            .get_context_with_context_options("2d", &ctx_opts)?
            .ok_or_else(|| JsValue::from_str("grid canvas 2d context unavailable"))?
            .unchecked_into::<CanvasRenderingContext2d>();
        let renderer = CanvasRenderer::for_layer(ctx, 0.0, 0.0, LIGHT);
        Ok(Self {
            canvas,
            css_width: 0.0,
            css_height: 0.0,
            dpr: 1.0,
            gate: PaintGate::new(),
            renderer,
        })
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.gate.mark_dirty();
    }

    /// Paint the grid: theme bg + (if model present) the cell/header phases.
    /// No-op when clean.
    pub(crate) fn paint_if_dirty(&mut self, theme: &CanvasTheme, model: Option<&dyn CanvasModel>) {
        if !self.gate.should_paint() {
            return;
        }
        self.renderer.set_theme(*theme);
        // Bypass the renderer's cached fill helper: this raw write desyncs
        // `last_fill`, so invalidate the cache before any cached path runs.
        let ctx = self.renderer.ctx_ref();
        ctx.set_fill_style_str(theme.cell_bg);
        ctx.fill_rect(0.0, 0.0, self.css_width, self.css_height);
        self.renderer.invalidate_paint_cache();
        if let Some(m) = model {
            self.renderer.render_grid(m);
        }
    }

    /// Resize the backing store. Ports the guard from `CanvasRenderer::new`:
    /// only reallocates the bitmap when dimensions actually change.
    pub(crate) fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.css_width = css_w;
        self.css_height = css_h;
        self.dpr = dpr;
        let (target_w, target_h) = target_backing_size(css_w, css_h, dpr);
        if self.canvas.width() != target_w || self.canvas.height() != target_h {
            self.canvas.set_width(target_w);
            self.canvas.set_height(target_h);
        } else {
            self.renderer
                .ctx_ref()
                .set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
                .expect("set_transform should not fail");
        }
        self.renderer
            .ctx_ref()
            .scale(dpr, dpr)
            .expect("scale should not fail");
        self.renderer.set_size(css_w, css_h);
        self.renderer.invalidate_paint_cache();
    }
}

/// Convert CSS dimensions + DPR to backing-store pixel counts.
/// Extracted so the resize guard logic is testable without a real canvas.
pub(super) fn target_backing_size(css_w: f64, css_h: f64, dpr: f64) -> (u32, u32) {
    ((css_w * dpr) as u32, (css_h * dpr) as u32)
}

#[cfg(test)]
mod tests {
    use super::target_backing_size;

    #[test]
    fn backing_size_scales_by_dpr() {
        assert_eq!(target_backing_size(100.0, 200.0, 2.0), (200, 400));
    }

    #[test]
    fn backing_size_at_1x_dpr_equals_css() {
        assert_eq!(target_backing_size(1920.0, 1080.0, 1.0), (1920, 1080));
    }

    #[test]
    fn backing_size_truncates_fractional_pixels() {
        // (375.5 * 2.0) = 751.0 exactly; (100.3 * 1.5) = 150.45 → truncates to 150
        assert_eq!(target_backing_size(100.3, 50.7, 1.5), (150, 76));
    }
}
