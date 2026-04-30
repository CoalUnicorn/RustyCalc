use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::geometry::{FrameContext, PixelRect, Point};
use crate::layer::{GridLayer, OverlayLayer};
use crate::theme::{CanvasTheme, DARK, LIGHT};
use crate::types::RenderOverlays;
use crate::wasm::JsBackedModel;
use crate::{CanvasModel, HitTest, ResizeTarget};

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
    last_frame: Option<FrameContext>,
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
            theme: LIGHT,
            overlays: RenderOverlays::default(),
            model: None,
            last_frame: None,
        })
    }

    /// Fan out resize atomically. Both layers resize in one call; partial-resize
    /// state is unreachable because there is no public per-layer resize method.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.grid.resize(css_w, css_h, dpr);
        self.overlay.resize(css_w, css_h, dpr);
    }

    /// Push a new scroll origin. Fans out to grid + overlay; no-op if unchanged.
    pub fn set_viewport(&mut self, top_row: i32, left_column: i32) {
        if self
            .last_frame
            .as_ref()
            .map(|f| f.top_row == top_row && f.left_column == left_column)
            .unwrap_or(false)
        {
            return;
        }
        self.grid.mark_dirty();
        self.overlay.mark_dirty();
    }

    /// Push a new theme by name ("light" | "dark"). Fans out to grid + overlay; no-op if unchanged.
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = if name == "dark" { DARK } else { LIGHT };
        if theme != self.theme {
            self.theme = theme;
            self.grid.mark_dirty();
            self.overlay.mark_dirty();
        }
    }

    /// Push a freeze configuration. Grid-only; no-op if unchanged.
    pub fn set_freeze(&mut self, frozen_rows: u32, frozen_cols: u32) {
        if self
            .last_frame
            .as_ref()
            .map(|f| {
                f.frozen.frozen_rows_count() == frozen_rows as i32
                    && f.frozen.frozen_cols_count() == frozen_cols as i32
            })
            .unwrap_or(false)
        {
            return;
        }
        self.grid.mark_dirty();
    }

    /// Mark both layers dirty. JS calls this after any state mutation
    /// (scroll, selection change, resize) and then calls `paint_if_dirty`
    /// on the next animation frame.
    pub fn request_repaint(&mut self) {
        self.grid.mark_dirty();
        self.overlay.mark_dirty();
    }

    /// Drive each layer's gate. Layers that are clean skip their paint entirely.
    pub fn paint_if_dirty(&mut self) {
        let grid_dirty = self.grid.should_paint();
        let overlay_dirty = self.overlay.should_paint();

        if !grid_dirty && !overlay_dirty {
            return;
        }

        let Some(model) = self.model.as_deref() else {
            return;
        };

        // One viewport snapshot per tick — both layers paint against the same
        // visible region, frozen geometry, and prefix-sum pixel offsets.
        // The same `last_frame` is later read by `hit_test`, `cell_rect`, and
        // `resize_handle_at` so input handlers see exactly what was painted.
        let frame = FrameContext::current(model, self.grid.canvas_size());

        if grid_dirty {
            self.grid.paint(self.theme, model, &frame);
        }
        if overlay_dirty {
            self.overlay
                .paint(self.theme, &self.overlays, model, &frame);
        }

        self.last_frame = Some(frame);
    }

    /// Explicit teardown for React strict-mode / Leptos Effect mount cycles.
    /// Rust's `Drop` handles resource cleanup on scope exit; this provides a
    /// named JS callsite for `create → drop → create` patterns.
    pub fn dispose(self) {}

    /// JS-facing model push. Accepts the IronCalc `Model` JS handle as a raw
    /// `JsValue` and `unchecked_into`-casts it. No runtime validation — a
    /// wrong handle throws on first render-time method call, not here.
    ///
    /// Currently always re-wraps in a fresh `Rc`, so identity-comparison in
    /// `set_model` always sees a change and re-marks the grid dirty. Good
    /// enough for now; tighten by tracking the previous `JsValue` once the
    /// JS-side push frequency is known.
    pub fn set_model_js(&mut self, model: JsValue) {
        let backed: Rc<dyn CanvasModel> = Rc::new(JsBackedModel::from_js_value(model));
        self.set_model(backed);
    }

    /// Push the active selection rectangle (CSS pixels). Overlay-only;
    /// value-compare in `set_overlays` makes redundant pushes free.
    pub fn set_selection_rect(&mut self, x: f64, y: f64, width: f64, height: f64) {
        let next = RenderOverlays {
            selection: Some(PixelRect {
                top_left: Point { x, y },
                width,
                height,
            }),
            ..self.overlays.clone()
        };
        self.set_overlays(next);
    }

    /// Drop the active selection rectangle. Other overlay fields are preserved.
    pub fn clear_selection(&mut self) {
        if self.overlays.selection.is_none() {
            return;
        }
        let next = RenderOverlays {
            selection: None,
            ..self.overlays.clone()
        };
        self.set_overlays(next);
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
        let (Some(frame), Some(model)) = (self.last_frame.as_ref(), self.model.as_deref()) else {
            return HitTest::Outside;
        };
        frame.hit_test(model, x, y)
    }

    /// Probe for a row/column resize handle near `(x, y)`. `tolerance` is the
    /// half-width of the hit-zone in CSS pixels — caller-controlled because
    /// it tracks cursor styling, not paint geometry. Returns `None` before
    /// the first paint.
    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        let frame = self.last_frame.as_ref()?;
        let model = self.model.as_deref()?;
        frame.resize_handle_at(model, x, y, tolerance)
    }

    /// Pixel rect of `(row, column)` in the last painted frame's coordinate
    /// space. Returns `None` before the first paint or for cells outside the
    /// frame's visible region (frozen bands + scrollable area).
    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        let frame = self.last_frame.as_ref()?;
        let model = self.model.as_deref()?;
        frame.cell_rect(model, row, column)
    }

    /// Pixel position of the autofill handle for the active selection in the
    /// last painted frame, or `None` for full-row/column/sheet selections.
    /// Used by callers that need the handle's *position* (e.g. drag-start
    /// state); for "is the cursor *over* it?" use `hit_test` and match on
    /// `HitTest::AutofillHandle` instead.
    pub fn autofill_handle(&self) -> Option<Point> {
        let frame = self.last_frame.as_ref()?;
        let model = self.model.as_deref()?;
        frame.autofill_handle(model)
    }

    pub fn request_overlay_repaint(&mut self) {
        if let Some(model) = self.model.as_deref() {
            let view = model.get_selected_view();
            let scrolled = self
                .last_frame
                .as_ref()
                .map(|f| f.top_row != view.top_row || f.left_column != view.left_column)
                .unwrap_or(true);
            if scrolled {
                self.grid.mark_dirty();
            }
        }
        self.overlay.mark_dirty();
    }
}
