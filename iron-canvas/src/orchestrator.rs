use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::chrome::Chrome;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::CanvasSize;
use crate::layer::{GridLayer, OverlayLayer, RenderOverlays};
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::ui::{HitTest, ResizeTarget};
use crate::wasm::JsBackedModel;
use crate::CanvasModel;

/// Public wasm-bindgen handle owning both canvas layers.
///
/// Consumers mount two stacked `<canvas>` elements and pass them once at
/// startup. All subsequent state pushes use `set_*` / `request_repaint`.
/// CSS stacking (`position: absolute`, correct `z-index`, `pointer-events:
/// none` on the overlay) is the caller's responsibility.
///
/// `GridLayer` and `OverlayLayer` are intentionally private — they are not
/// part of the wasm-bindgen surface and do not appear in the generated `.d.ts`.
#[wasm_bindgen]
pub struct IronCanvas {
    grid: GridLayer,
    overlay: OverlayLayer,
    theme: CanvasTheme,
    overlays: RenderOverlays,
    model: Option<Rc<dyn CanvasModel>>,
    last_frame: Option<Chrome>,
    /// Logical (CSS) canvas size, written by `resize` and read by
    /// `paint_if_dirty` when building the shared `Chrome`.
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

    /// Fan out resize atomically. Both layers resize in one call; partial-resize
    /// state is unreachable because there is no public per-layer resize method.
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

    /// Push a new theme by name ("light" | "dark"). Routes through `set_theme`,
    /// so value-eq + dirty fan-out lives in one place.
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = if name == "dark" {
            CanvasTheme::dark()
        } else {
            CanvasTheme::light()
        };
        self.set_theme(theme);
    }

    /// Mark both layers dirty. JS calls this after a state mutation that
    /// invalidates the painted geometry (scroll, selection change) and then
    /// calls `paint_if_dirty` on the next animation frame.
    pub fn request_repaint(&mut self) {
        self.grid.mark_dirty();
        self.overlay.mark_dirty();
    }

    /// Drive each layer's gate. Layers that are clean skip their paint entirely.
    pub fn paint_if_dirty(&mut self) {
        let mut grid_dirty = self.grid.should_paint();
        let overlay_dirty = self.overlay.should_paint();

        if !grid_dirty && !overlay_dirty {
            return;
        }

        let Some(model) = self.model.as_deref() else {
            return;
        };

        // Overlay-only fast path: reuse the last frame's slot vecs when
        // scroll / freeze / sheet / canvas size are unchanged. Triggered by
        // autofill drag, clipboard state change, formula-ref highlight
        // updates, and active-cell moves.
        //
        // Falls through to a full rebuild when geometry diverged (IronCalc's
        // UserModel is imperative — a sheet swap can mutate `Chrome`
        // identity without going through any setter) OR when no prior frame
        // exists, so the grid layer never sits beneath an overlay it didn't
        // paint with.
        if !grid_dirty {
            match self.last_frame.as_mut() {
                Some(prev) if prev.is_still_valid(model, self.size) => {
                    prev.refresh_overlay_inputs(model);
                    self.overlay.paint(&self.overlays, model, prev);
                    return;
                }
                _ => grid_dirty = true,
            }
        }

        // Full rebuild: scroll/freeze/size/sheet changed, or grid is dirty.
        // Recycle the outgoing frame's slot Vec allocations so steady-state
        // rebuilds don't re-allocate the four pane-set buffers.
        let frame = match self.last_frame.take() {
            Some(prev) => prev.rebuild(model, self.size, &self.theme),
            None => Chrome::current(model, self.size, &self.theme),
        };

        if grid_dirty {
            self.grid.paint(model, &frame);
        }
        if overlay_dirty {
            self.overlay.paint(&self.overlays, model, &frame);
        }

        self.last_frame = Some(frame);
    }

    /// Explicit teardown for React strict-mode / Leptos Effect mount cycles.
    /// Rust's `Drop` handles resource cleanup on scope exit; this provides a
    /// named JS callsite for `create -> drop -> create` patterns.
    pub fn dispose(self) {}

    /// JS-facing model push. Accepts the IronCalc `Model` JS handle as a raw
    /// `JsValue` and adopts it as an opaque `IronCalcModelHandle` after a
    /// structural duck-test (see `JsBackedModel::try_from_js_value`). The
    /// `JsError` return type — not a bare `JsValue` — gives the JS-side
    /// catch a real `Error` with `.message` and `.stack`. Per-call contract
    /// drift on the JS callbacks still surfaces through the `(catch, method)`
    /// extern wrappers in `wasm.rs` (counted via `note_js_throw`).
    ///
    /// Always re-wraps in a fresh `Rc`, so identity-comparison in `set_model`
    /// always sees a change and re-marks the grid dirty.
    pub fn set_model_js(&mut self, model: JsValue) -> Result<(), JsError> {
        let backed: Rc<dyn CanvasModel> = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.set_model(backed);
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// JS-facing theme push from a host DOM node. Reads the upstream
    /// `--palette-*` CSS custom properties off `el`'s computed style and
    /// builds a `CanvasTheme`. Same dirty-bit semantics as `set_theme` —
    /// pushing the same DOM state twice is a no-op.
    ///
    /// This is the canonical RustyCalc bridge: a leptos-use color-mode
    /// effect toggles `data-theme` on `<html>`, then calls this with
    /// `document.documentElement` (or any host element).
    pub fn set_theme_from_element(&mut self, el: &web_sys::Element) {
        self.set_theme(CanvasTheme::from_element(el));
    }
}

impl IronCanvas {
    /// Push overlay state. Overlay-only; value-compares so a redundant push is a no-op.
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        if overlays != self.overlays {
            self.overlays = overlays;
            self.overlay.mark_dirty();
        }
    }

    /// Rust-level theme push. Mirrors `set_theme_name` minus the string lookup;
    /// the wasm-bindgen surface keeps `set_theme_name` so the JS handle is
    /// unchanged. Value-compares against `self.theme`; on change marks both
    /// layers dirty.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != self.theme {
            self.theme = theme;
            self.grid.mark_dirty();
            self.overlay.mark_dirty();
        }
    }

    /// Push a theme described by upstream CSS-variable inputs. Convenience
    /// over `set_theme(vars.build())` — same dirty-bit semantics, idempotent
    /// when the resolved palette equals the current one.
    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.set_theme(vars.build());
    }

    /// Push a new data model. Grid-only; identity-compares via `Rc::ptr_eq`
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

    /// Logical (CSS) canvas size as last set by `resize`. Frame-independent —
    /// callers that need to compare a cursor position against the canvas
    /// bounds (autoscroll edge zones, drag clamping) read this instead of
    /// re-deriving it from a DOM element on every event.
    pub fn canvas_size(&self) -> CanvasSize {
        self.size
    }

    // Query API
    //
    // The mirror of the command surface above. Where `set_*` push state INTO
    // the canvas + model, these read what's currently *painted*: every query
    // resolves against `last_frame`, the snapshot built by the most recent
    // `paint_if_dirty`. That makes hit-tests provably consistent with what
    // the user sees on screen — even between a scroll mutation and the next
    // animation frame, when the model and the painted state disagree.
    //
    // Before the first paint, `last_frame` is `None` and queries fall back
    // to absent variants (`Outside`, `None`). Defensive — surfaces missing-
    // paint bugs rather than masking them with a hidden frame rebuild.

    /// What the user sees at canvas-space `(x, y)`, against the last painted
    /// frame. Returns `Outside` before the first paint or for off-canvas
    /// points (negative coordinates).
    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        let Some(frame) = self.last_frame.as_ref() else {
            return HitTest::Outside;
        };
        frame.hit_test(x.round() as i32, y.round() as i32)
    }

    /// Probe for a row/column resize handle near `(x, y)`. `tolerance` is the
    /// half-width of the hit-zone in CSS pixels — caller-controlled because
    /// it tracks cursor styling, not paint geometry. Returns `None` before
    /// the first paint.
    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.last_frame.as_ref()?.resize_handle_at(
            x.round() as i32,
            y.round() as i32,
            tolerance.round() as i32,
        )
    }

    /// Pixel rect of `(row, column)` in the last painted frame's coordinate
    /// space. Returns `None` before the first paint or for cells outside the
    /// frame's visible region (frozen bands + scrollable area).
    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.last_frame.as_ref()?.cell_rect(row, column)
    }

    /// Pixel position of the autofill handle for the active selection in the
    /// last painted frame, or `None` for full-row/column/sheet selections
    /// or selections whose bottom-right is off-frame. Used by callers that
    /// need the handle's *position* (e.g. drag-start state); for "is the
    /// cursor *over* it?" use `hit_test` and match on
    /// `HitTest::AutofillHandle` instead.
    pub fn autofill_handle(&self) -> Option<Point> {
        self.last_frame.as_ref()?.autofill_handle()
    }

    /// Mark the overlay layer dirty. Selection / autofill / formula-ref /
    /// clipboard signals funnel through here. Grid escalation when scroll /
    /// freeze / sheet / size diverged is handled by `paint_if_dirty`'s
    /// `is_still_valid` check, not duplicated here.
    pub fn request_overlay_repaint(&mut self) {
        self.overlay.mark_dirty();
    }
}
