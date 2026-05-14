use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::chrome::{Chrome, FrameValidity};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::CanvasSize;
use crate::layer::{GridLayer, OverlayLayer, RenderOverlays};
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::ui::{HitTest, ResizeTarget};
use crate::wasm::JsBackedModel;
use crate::CanvasModel;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[cfg(target_arch = "wasm32")]
fn log_blit_plan(
    plan: &crate::chrome::BlitPlan,
    prev_top: i32,
    prev_pane_range: Option<crate::RCRange>,
    frame: &Chrome,
) {
    use crate::geometry::prim::Axis;
    let axis = match plan.axis {
        Axis::Row => "Row",
        Axis::Column => "Col",
    };
    let new_top = frame.pane_set_top_row_debug();
    let new_last = frame.pane_set_last_row_debug();
    let scroll_rows_len = frame.scroll_rows_len_debug();
    let cached = match prev_pane_range {
        Some(r) => format!("rows {}..={}", r.r1, r.r2),
        None => "<none>".to_string(),
    };
    let msg = format!(
        "[blit] axis={} prev_top={} new_top={} new_last={} cache.range={} scroll_rows.len()={} src=(y={}, h={}) dst=(y={}, h={}) strip=(y={}, h={})",
        axis,
        prev_top,
        new_top,
        new_last,
        cached,
        scroll_rows_len,
        plan.src.top_left.y,
        plan.src.height,
        plan.dst.top_left.y,
        plan.dst.height,
        plan.repaint_strip.top_left.y,
        plan.repaint_strip.height,
    );
    log(&msg);
}

/// Public wasm-bindgen handle owning both canvas layers.
///
/// Consumers mount two stacked `<canvas>` elements and pass them once at
/// startup; subsequent updates go through `set_*` and `requestRepaint`.
/// CSS stacking (`position: absolute`, correct `z-index`, `pointer-events:
/// none` on the overlay) is the caller's responsibility.
#[wasm_bindgen]
pub struct IronCanvas {
    grid: GridLayer,
    overlay: OverlayLayer,
    theme: CanvasTheme,
    overlays: RenderOverlays,
    model: Option<Rc<dyn CanvasModel>>,
    last_frame: Option<Chrome>,
    /// Logical (CSS) canvas size; written by `resize`, read when building
    /// the next `Chrome`.
    size: CanvasSize,
}

#[wasm_bindgen]
impl IronCanvas {
    /// Construct over two stacked canvases.
    pub fn create(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<IronCanvas, JsValue> {
        let grid = GridLayer::create(grid_canvas)?;
        let overlay = OverlayLayer::create(overlay_canvas)?;
        Ok(IronCanvas {
            grid,
            overlay,
            theme: CanvasTheme::light(),
            overlays: RenderOverlays::default(),
            model: None,
            last_frame: None,
            size: CanvasSize { w: 0.0, h: 0.0 },
        })
    }

    /// Resize both layers in one call. No public per-layer resize exists,
    /// so callers cannot leave the pair half-sized.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.size = CanvasSize { w: css_w, h: css_h };
        self.grid.resize(
            css_w.round() as i32,
            css_h.round() as i32,
            dpr.round() as i32,
        );
        self.overlay.resize(
            css_w.round() as i32,
            css_h.round() as i32,
            dpr.round() as i32,
        );
    }

    /// Push a theme by name (`"light"` | `"dark"`). Routes through
    /// `set_theme` so value-eq and dirty fan-out stay in one place.
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = if name == "dark" {
            CanvasTheme::dark()
        } else {
            CanvasTheme::light()
        };
        self.set_theme(theme);
    }

    /// Mark both layers dirty. JS calls this after any state mutation that
    /// affects painted geometry (scroll, selection, etc.) and then calls
    /// `paintIfDirty` on the next animation frame.
    #[allow(non_snake_case)]
    pub fn requestRepaint(&mut self) {
        self.grid.mark_dirty();
        self.overlay.mark_dirty();
    }

    /// Paint whichever layers are dirty. Clean layers are skipped; see
    /// `ARCHITECTURE.md` for the cache rules and the overlay-only path.
    ///
    /// Dispatches into one of three named paths, in priority order:
    /// 1. `try_paint_overlay_only` — grid clean + slot vecs match.
    /// 2. `try_paint_blit` — viewport scrolled and the backend can shift pixels.
    /// 3. `paint_grid` — full grid repaint (slot-reuse or rebuild).
    #[allow(non_snake_case)]
    pub fn paintIfDirty(&mut self) {
        let grid_dirty = self.grid.should_paint();
        let overlay_dirty = self.overlay.should_paint();

        if !grid_dirty && !overlay_dirty {
            return;
        }

        let Some(model) = self.model.clone() else {
            return;
        };
        let model: &dyn CanvasModel = model.as_ref();

        let validity = self
            .last_frame
            .as_ref()
            .map_or(FrameValidity::Rebuild, |f| {
                f.is_still_valid(model, self.size)
            });

        if !grid_dirty && self.try_paint_overlay_only(validity, model) {
            return;
        }
        if self.try_paint_blit(model) {
            return;
        }
        self.paint_grid(validity, model, overlay_dirty);
    }

    /// Explicit teardown for React strict-mode / Leptos `Effect` mount
    /// cycles. `Drop` already handles cleanup on scope exit; this just
    /// gives JS a named callsite for the `create -> drop -> create` dance.
    pub fn dispose(self) {}

    /// JS-facing model push. Adopts the IronCalc `Model` JS handle as an
    /// opaque `JsBackedModel` after the structural duck-test in
    /// `JsBackedModel::try_from_js_value`. Returns `JsError` (not bare
    /// `JsValue`) so the JS catch sees a real `Error` with `.message` and
    /// `.stack`; per-call contract drift still surfaces through the
    /// `(catch, method)` wrappers in `wasm.rs`.
    ///
    /// Every call re-wraps in a fresh `Rc`, so `set_model`'s identity
    /// check always sees a change and re-marks the grid dirty.
    #[allow(non_snake_case)]
    pub fn setModel(&mut self, model: JsValue) -> Result<(), JsError> {
        let backed: Rc<dyn CanvasModel> = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.set_model(backed);
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// JS-facing theme push from a host DOM node. Reads the upstream
    /// `--palette-*` custom properties off `el`'s computed style and
    /// builds a `CanvasTheme`. Same idempotence as `set_theme`: pushing
    /// the same DOM state twice is a no-op.
    ///
    /// The canonical RustyCalc bridge: a leptos-use color-mode effect
    /// toggles `data-theme` on `<html>`, then calls this with
    /// `document.documentElement` (or any host element).
    #[allow(non_snake_case)]
    pub fn setThemeFromElement(&mut self, el: &web_sys::Element) {
        self.set_theme(CanvasTheme::from_element(el));
    }
}

impl IronCanvas {
    /// Push overlay state. Overlay-only; value-compared, so a redundant
    /// push is a no-op.
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        if overlays != self.overlays {
            self.overlays = overlays;
            self.overlay.mark_dirty();
        }
    }

    /// Rust-level theme push. The wasm surface keeps `set_theme_name` to
    /// preserve the JS handle. Value-compares against `self.theme` and,
    /// on change, marks both layers dirty.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != self.theme {
            self.theme = theme;
            self.grid.mark_dirty();
            self.overlay.mark_dirty();
        }
    }

    /// Push a theme described by upstream CSS-variable inputs. Convenience
    /// over `set_theme(vars.build())`; same idempotence rules.
    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.set_theme(vars.build());
    }

    /// Push a new data model. Grid-only; identity-compared via `Rc::ptr_eq`,
    /// so pushing the same `Rc` twice is a no-op.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        let changed = match &self.model {
            Some(prev) => !Rc::ptr_eq(prev, &model),
            None => true,
        };
        if changed {
            self.model = Some(model);
            self.grid.mark_dirty();
        }
    }

    /// Logical (CSS) canvas size as last set by `resize`. Independent of
    /// the painted frame, so callers comparing cursor position to canvas
    /// bounds (autoscroll edge zones, drag clamping) avoid a DOM round-trip
    /// per pointer event.
    pub fn canvas_size(&self) -> CanvasSize {
        self.size
    }

    // Query API.
    //
    // All queries resolve against `last_frame`, the snapshot emitted by the
    // most recent `paintIfDirty`. Before the first paint `last_frame` is
    // `None` and every query returns its absent variant (`Outside`, `None`)
    // rather than triggering a hidden rebuild.

    /// Resolve canvas-space `(x, y)` against the last painted frame.
    /// Returns `Outside` before the first paint or for negative coordinates.
    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        let Some(frame) = self.last_frame.as_ref() else {
            return HitTest::Outside;
        };
        frame.hit_test(x.round() as i32, y.round() as i32)
    }

    /// Probe for a row/column resize handle near `(x, y)`. `tolerance` is
    /// the hit-zone half-width in CSS pixels; the caller controls it
    /// because it tracks cursor styling, not paint geometry. Returns
    /// `None` before the first paint.
    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.last_frame.as_ref()?.resize_handle_at(
            x.round() as i32,
            y.round() as i32,
            tolerance.round() as i32,
        )
    }

    /// Pixel rect of `(row, column)` in the last painted frame. Returns
    /// `None` before the first paint or for cells outside the visible
    /// region (frozen bands plus the scrollable area).
    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.last_frame.as_ref()?.cell_rect(row, column)
    }

    /// Pixel position of the autofill handle for the active selection.
    /// `None` for full-row/column/sheet selections, and for selections
    /// whose bottom-right is off-frame.
    ///
    /// This is a *position* query (use it for drag-start state). For
    /// "is the cursor over the handle?", use `hit_test` and match
    /// `HitTest::AutofillHandle`; the two are not interchangeable because
    /// `hit_test` applies `AUTOFILL_HIT_PAD_PX` and this does not.
    pub fn autofill_handle(&self) -> Option<Point> {
        self.last_frame.as_ref()?.autofill_handle()
    }

    /// Mark the overlay dirty. Selection, autofill, formula-ref, and
    /// clipboard signals funnel through here; grid escalation on scroll /
    /// freeze / sheet / size change is owned by `paintIfDirty` via
    /// `is_still_valid`, not duplicated at the callsite.
    pub fn request_overlay_repaint(&mut self) {
        self.overlay.mark_dirty();
    }

    /// Overlay-only fast path. Returns true when the overlay was painted
    /// against the cached `last_frame` and the caller should stop.
    /// Triggered by autofill drag, clipboard state change, formula-ref
    /// highlight updates, and active-cell moves — anything that leaves
    /// grid pixels untouched.
    fn try_paint_overlay_only(&mut self, validity: FrameValidity, model: &dyn CanvasModel) -> bool {
        if !matches!(validity, FrameValidity::SlotsReuse) {
            return false;
        }
        let Some(prev) = self.last_frame.as_mut() else {
            return false;
        };
        prev.refresh_overlay_inputs(model);
        self.overlay.paint(&self.overlays, model, prev);
        true
    }

    /// Scroll-blit fast path. Runs whenever the viewport scrolled along a
    /// single axis and the painter can shift kept-band pixels — even when
    /// only the overlay was marked dirty (arrow-nav fires
    /// `request_overlay_repaint`, but a viewport scroll still requires a
    /// grid update). `try_blit` filters no-op scrolls and cases where the
    /// kept band can't be reused (theme / sheet / frozen / canvas
    /// changed, or scroll exceeded the viewport extent).
    ///
    /// Returns true on success. Backends without `supports_blit` (e.g.
    /// SVG) opt out and fall through to a full repaint.
    fn try_paint_blit(&mut self, model: &dyn CanvasModel) -> bool {
        if !self.grid.painter_supports_blit() {
            return false;
        }
        let Some(plan) = self
            .last_frame
            .as_ref()
            .and_then(|f| f.try_blit(model, self.size, &self.theme))
        else {
            return false;
        };
        let Some(prev) = self.last_frame.take() else {
            return false;
        };
        #[cfg(target_arch = "wasm32")]
        let prev_top = prev.pane_set_top_row_debug();
        #[cfg(target_arch = "wasm32")]
        let prev_pane_range = self.grid.bottom_right_cache_range_debug();
        let frame = Chrome::next_frame_with_blit(prev, model, self.size, &self.theme, &plan);
        #[cfg(target_arch = "wasm32")]
        log_blit_plan(&plan, prev_top, prev_pane_range, &frame);
        self.grid.paint_blit(model, &frame, &plan);
        self.overlay.paint(&self.overlays, model, &frame);
        self.last_frame = Some(frame);
        true
    }

    /// Full grid repaint. Two `Chrome` sources:
    ///   • `SlotsReuse` — keep prev's slot vecs, promote its painted
    ///     `pane_fingerprints` into `prev_pane_fingerprints`, refresh
    ///     theme + overlay inputs. Cheap; `render_pane` still
    ///     fingerprint-skips per pane.
    ///   • Rebuild — full `next_frame` walk. The new slot vecs make any
    ///     fingerprint compare across the boundary meaningless, so every
    ///     pane repaints.
    fn paint_grid(
        &mut self,
        validity: FrameValidity,
        model: &dyn CanvasModel,
        overlay_dirty: bool,
    ) {
        let frame = match (validity, self.last_frame.take()) {
            (FrameValidity::SlotsReuse, Some(mut prev)) => {
                prev.prev_pane_fingerprints = prev.pane_fingerprints.replace([0; 4]);
                prev.theme = self.theme.clone();
                prev.slots_reused = true;
                prev.refresh_overlay_inputs(model);
                prev
            }
            (_, prev) => Chrome::next_frame(prev, model, self.size, &self.theme),
        };

        self.grid.paint(model, &frame);
        if overlay_dirty {
            self.overlay.paint(&self.overlays, model, &frame);
        }

        self.last_frame = Some(frame);
    }
}
