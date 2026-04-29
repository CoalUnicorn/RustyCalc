use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::theme::{CanvasTheme, LIGHT};
use crate::types::RenderOverlays;
use crate::CanvasModel;
use crate::CanvasRenderer;

use super::{grid::target_backing_size, PaintGate};

pub(crate) struct OverlayLayer {
    pub(crate) canvas: HtmlCanvasElement,
    pub(crate) css_width: f64,
    pub(crate) css_height: f64,
    pub(crate) dpr: f64,
    gate: PaintGate,
    /// Long-lived renderer; owns the layer's 2D ctx so paint caches persist
    /// across frames. Theme is hot-swapped per paint via `set_theme`.
    renderer: CanvasRenderer,
}

impl OverlayLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx_opts = js_sys::Object::new();

        js_sys::Reflect::set(&ctx_opts, &"alpha".into(), &JsValue::from(true))
            .map_err(|_| JsValue::from_str("failed to set overlay context alpha option"))?;
        js_sys::Reflect::set(&ctx_opts, &"desynchronized".into(), &JsValue::from(true)).map_err(
            |_| JsValue::from_str("failed to set overlay context desynchronized option"),
        )?;
        let ctx = canvas
            .get_context_with_context_options("2d", &ctx_opts)?
            .ok_or_else(|| JsValue::from_str("overlay canvas 2d context unavailable"))?
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

    /// Clear to transparent, then (if model present) draw the full overlay
    /// phase via the shared renderer. No-op when clean.
    pub(crate) fn paint_if_dirty(
        &mut self,
        theme: &CanvasTheme,
        overlays: &RenderOverlays,
        model: Option<&dyn CanvasModel>,
    ) {
        if !self.gate.should_paint() {
            return;
        }
        self.renderer.set_theme(*theme);
        self.renderer
            .ctx_ref()
            .clear_rect(0.0, 0.0, self.css_width, self.css_height);
        if let Some(m) = model {
            self.renderer.render_overlays(m, overlays);
        }
    }

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
