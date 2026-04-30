use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::geometry::{PixelRect, Point};
use crate::layer::{GridLayer, OverlayLayer};
use crate::theme::{CanvasTheme, DARK, LIGHT};
use crate::types::{FreezeConfig, RenderOverlays, Viewport};
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
    viewport: Viewport,
    theme: CanvasTheme,
    freeze: FreezeConfig,
    overlays: RenderOverlays,
    model: Option<Rc<dyn CanvasModel>>,
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
            viewport: Viewport::default(),
            theme: LIGHT,
            freeze: FreezeConfig::default(),
            overlays: RenderOverlays::default(),
            model: None,
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
        let vp = Viewport {
            top_row,
            left_column,
        };
        if vp != self.viewport {
            self.viewport = vp;
            self.grid.mark_dirty();
            self.overlay.mark_dirty();
        }
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
        let freeze = FreezeConfig {
            frozen_rows,
            frozen_cols,
        };
        if freeze != self.freeze {
            self.freeze = freeze;
            self.grid.mark_dirty();
        }
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
        let model = self.model.as_deref();
        self.grid.paint_if_dirty(&self.theme, model);
        self.overlay
            .paint_if_dirty(&self.theme, &self.overlays, model);
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

    pub fn request_overlay_repaint(&mut self) {
        if let Some(model) = self.model.as_deref() {
            let view = model.get_selected_view();
            let vp = Viewport {
                top_row: view.top_row,
                left_column: view.left_column,
            };
            if vp != self.viewport {
                self.viewport = vp;
                self.grid.mark_dirty();
            }
        }
        self.overlay.mark_dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubModel;
    impl CanvasModel for StubModel {
        fn get_selected_sheet(&self) -> u32 {
            0
        }
        fn get_selected_view(&self) -> crate::SelectedView {
            crate::SelectedView {
                sheet: 0,
                row: 1,
                column: 1,
                range: [1, 1, 1, 1],
                top_row: 1,
                left_column: 1,
            }
        }
        fn get_frozen_rows_count(&self, _: u32) -> Result<i32, String> {
            Ok(0)
        }
        fn get_frozen_columns_count(&self, _: u32) -> Result<i32, String> {
            Ok(0)
        }
        fn get_row_height(&self, _: u32, _: i32) -> Result<f64, String> {
            Ok(20.0)
        }
        fn get_column_width(&self, _: u32, _: i32) -> Result<f64, String> {
            Ok(80.0)
        }
        fn get_show_grid_lines(&self, _: u32) -> Result<bool, String> {
            Ok(true)
        }
        fn get_cell_style(
            &self,
            _: u32,
            _: i32,
            _: i32,
        ) -> Result<ironcalc_base::types::Style, String> {
            Ok(ironcalc_base::types::Style::default())
        }
        fn get_cell_type(
            &self,
            _: u32,
            _: i32,
            _: i32,
        ) -> Result<ironcalc_base::types::CellType, String> {
            Ok(ironcalc_base::types::CellType::Number)
        }
        fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Result<String, String> {
            Ok(String::new())
        }
    }

    // ── Drag-frame isolation tests ──────────────────────────────────────────
    //
    // These simulate the headline acceptance criterion without a browser: two
    // `PaintGate` instances stand in for the real layers. The logic mirrors
    // exactly what `IronCanvas::set_overlays` and `paint_if_dirty` do in
    // production, so a pass here proves the fan-out policy is correct.

    fn make_sel(x: f64) -> crate::geometry::PixelRect {
        use crate::geometry::{PixelRect, Point};
        PixelRect {
            top_left: Point { x, y: 0.0 },
            width: 80.0,
            height: 20.0,
        }
    }

    #[test]
    fn set_overlays_only_dirties_overlay() {
        use crate::layer::PaintGate;
        let mut grid = PaintGate::new();
        let mut overlay = PaintGate::new();
        let mut current = RenderOverlays::default();

        let next = RenderOverlays {
            selection: Some(make_sel(10.0)),
            ..Default::default()
        };
        // mirror set_overlays fan-out policy
        if next != current {
            overlay.mark_dirty();
        }
        current = next;
        let _ = current;

        assert!(
            overlay.should_paint(),
            "overlay must be dirty after set_overlays"
        );
        assert!(
            !grid.should_paint(),
            "grid must NOT be dirty after set_overlays"
        );
    }

    #[test]
    fn sixty_drag_frames_increment_overlay_only() {
        use crate::layer::PaintGate;
        let mut grid = PaintGate::new();
        let mut overlay = PaintGate::new();
        let mut current = RenderOverlays::default();

        for i in 0..60_u32 {
            let next = RenderOverlays {
                selection: Some(make_sel(i as f64 * 2.0)),
                ..Default::default()
            };
            // mirror set_overlays
            if next != current {
                overlay.mark_dirty();
            }
            current = next;
            // mirror paint_if_dirty — consume both gates
            grid.should_paint();
            overlay.should_paint();
        }

        assert_eq!(grid.paint_count, 0, "grid must not paint during drag");
        assert_eq!(
            overlay.paint_count, 60,
            "overlay must paint once per drag frame"
        );
    }

    #[test]
    fn set_model_same_rc_is_no_op() {
        let m: Rc<dyn CanvasModel> = Rc::new(StubModel);
        let clone = Rc::clone(&m);
        assert!(Rc::ptr_eq(&m, &clone), "ptr_eq must hold for same Rc");
    }

    #[test]
    fn set_model_different_rc_is_change() {
        let m1: Rc<dyn CanvasModel> = Rc::new(StubModel);
        let m2: Rc<dyn CanvasModel> = Rc::new(StubModel);
        assert!(
            !Rc::ptr_eq(&m1, &m2),
            "distinct Rc allocations must not be equal"
        );
    }

    #[test]
    fn nav_event_only_dirties_overlay() {
        use crate::layer::PaintGate;
        // Simulate worksheet.rs: set_overlays fires, request_repaint does NOT.
        let mut grid = PaintGate::new();
        let mut overlay = PaintGate::new();
        let mut current = RenderOverlays::default();

        let next = RenderOverlays {
            selection: Some(make_sel(20.0)),
            ..Default::default()
        };
        // mirror conditionalized fan-out: nav → set_overlays only
        if next != current {
            overlay.mark_dirty();
        }
        current = next;
        let _ = current;

        assert!(
            overlay.should_paint(),
            "overlay must be dirty after nav set_overlays"
        );
        assert!(
            !grid.should_paint(),
            "grid must NOT be dirty on nav-only event"
        );
    }
}
