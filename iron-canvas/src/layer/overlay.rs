use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::layer::LayerBase;
use crate::theme::{CanvasTheme, LIGHT};
use crate::types::coord::{AutofillTarget, SheetArea};
use crate::{CanvasModel, CanvasRenderer, FormulaRef, RCRange};

use crate::geometry::frame::FrameContext;

/// Overlay ranges passed to `render()`.
///
/// Selection is not stored here — it is paint-time-derived from
/// `model.get_selected_view()`. The consumer signals selection changes via
/// `IronCanvas::request_overlay_repaint()`.
#[derive(Clone, PartialEq, Default)]
pub struct RenderOverlays {
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<RCRange>,
    /// All formula refs extracted from the current formula (multi-color overlays).
    pub formula_refs: Vec<FormulaRef>,
}

pub(crate) struct OverlayLayer {
    base: LayerBase,
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
            base: LayerBase::new(canvas, renderer),
        })
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.base.mark_dirty();
    }

    pub(crate) fn should_paint(&mut self) -> bool {
        self.base.should_paint()
    }

    pub(crate) fn paint(
        &mut self,
        theme: CanvasTheme,
        overlays: &RenderOverlays,
        model: &dyn CanvasModel,
        frame: &FrameContext,
    ) {
        self.base.renderer.set_theme(theme);
        let size = self.base.canvas_size();
        self.base
            .renderer
            .ctx_ref()
            .clear_rect(0.0, 0.0, size.w, size.h);
        self.base.renderer.render_overlays(model, overlays, frame);
    }

    pub(crate) fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.base.resize(css_w, css_h, dpr);
    }
}
