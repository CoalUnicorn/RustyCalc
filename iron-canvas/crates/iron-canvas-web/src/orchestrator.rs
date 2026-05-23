//! Web-side facade. The frame dispatch + state aggregator now lives in
//! `iron_canvas_core::Orchestrator`; this struct holds the `wasm-bindgen`
//! handle, builds two `WebSurface`s, and delegates every setter / query /
//! paint call to the core orchestrator.

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::theme::{CanvasTheme, ThemeVariables};
use crate::wasm::JsBackedModel;
use crate::web_surface::WebSurface;
use crate::RenderOverlays;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_core::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};
use iron_canvas_core::CanvasModel;
use iron_canvas_core::Orchestrator;
use iron_canvas_svg::SvgSurface;

#[cfg(feature = "dev-tools")]
use iron_canvas_core::PaintRegimeTag;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::recording::{Frame, IcrHeader, Recording, ThemeSnapshot};
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::DrawOp;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::RecordingSurface;

/// Facade Surface — wraps `WebSurface` in `RecordingSurface` when the
/// `recorder` feature is on (dev builds), bare `WebSurface` otherwise
/// (prod). `Orchestrator<FacadeSurface, _>` flows the choice through
/// the rest of the engine via the `Surface` trait without any other
/// site needing to know which is which.
#[cfg(feature = "dev-tools")]
type FacadeSurface = RecordingSurface<WebSurface>;
#[cfg(not(feature = "dev-tools"))]
type FacadeSurface = WebSurface;

#[cfg(feature = "dev-tools")]
fn wrap_surface(s: WebSurface) -> FacadeSurface {
    RecordingSurface::new(s)
}
#[cfg(not(feature = "dev-tools"))]
fn wrap_surface(s: WebSurface) -> FacadeSurface {
    s
}

#[cfg(feature = "dev-tools")]
const SOFT_WARN_MS: u64 = 30_000;
#[cfg(feature = "dev-tools")]
const HARD_CAP_BYTES: usize = 100 * 1024 * 1024;
#[cfg(feature = "dev-tools")]
const OP_BYTES_HEURISTIC: usize = 150;

// Per-op byte estimate for the hard-cap heuristic. Fixed-shape variants
// settle near `OP_BYTES_HEURISTIC` after JSON encoding; FillText and
// BeginGroup carry owned strings whose length dominates real-world
// recordings (cell text + font_css + color, decoration class names),
// so charge them on top.
#[cfg(feature = "dev-tools")]
fn op_bytes(op: &DrawOp) -> usize {
    match op {
        DrawOp::FillText {
            text,
            font_css,
            color,
            ..
        } => OP_BYTES_HEURISTIC + text.len() + font_css.len() + color.len(),
        DrawOp::BeginGroup { class } => OP_BYTES_HEURISTIC + class.len(),
        _ => OP_BYTES_HEURISTIC,
    }
}

#[cfg(feature = "dev-tools")]
struct RecordingState {
    rec: Recording,
    started_at_ms: f64,
    bytes_estimate: usize,
    soft_warn_fired: bool,
    capped: bool,
}

#[wasm_bindgen]
pub struct IronCanvas {
    orch: Orchestrator<FacadeSurface, Rc<dyn CanvasModel>>,
    // Cached so SVG export can re-push the live model into a throwaway
    // orchestrator. Updated alongside every `set_model` / `setModel`.
    model: Option<Rc<dyn CanvasModel>>,
    #[cfg(feature = "dev-tools")]
    recording: Option<RecordingState>,
}

#[wasm_bindgen]
impl IronCanvas {
    /// Construct over two stacked canvases. CSS stacking (`position:
    /// absolute`, correct `z-index`, `pointer-events: none` on the
    /// overlay) is the caller's responsibility.
    pub fn create(
        grid_canvas: HtmlCanvasElement,
        overlay_canvas: HtmlCanvasElement,
    ) -> Result<IronCanvas, JsValue> {
        let grid = wrap_surface(WebSurface::grid(grid_canvas)?);
        let overlay = wrap_surface(WebSurface::overlay(overlay_canvas)?);
        Ok(IronCanvas {
            orch: Orchestrator::<FacadeSurface, Rc<dyn CanvasModel>>::new(grid, overlay),
            model: None,
            #[cfg(feature = "dev-tools")]
            recording: None,
        })
    }

    /// Whether this build supports `startRecording` / `stopRecording`.
    /// Always callable from JS so the host can hide its Record button on
    /// prod builds without `try`-sniffing the class shape.
    #[allow(non_snake_case)]
    pub fn recordingSupported() -> bool {
        cfg!(feature = "dev-tools")
    }

    /// Resize both layers in one call.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.orch
            .resize(CanvasSize { w: css_w, h: css_h }, dpr.round() as i32);
    }

    /// Push a theme by name. Only `"dark"` is recognized; every other
    /// value (including `"light"` and anything misspelled) maps to the
    /// light palette.
    pub fn set_theme_name(&mut self, name: &str) {
        let theme = if name == "dark" {
            CanvasTheme::dark()
        } else {
            CanvasTheme::light()
        };
        self.orch.set_theme(theme);
    }

    /// Conservative repaint blanket — see `Orchestrator::request_repaint`.
    #[allow(non_snake_case)]
    pub fn requestRepaint(&mut self) {
        self.orch.request_repaint();
    }

    /// JS-facing cell-content-changed signal — marks all four pane
    /// quadrants. Pane-granular masks stay Rust-internal.
    #[allow(non_snake_case)]
    pub fn markContentDirty(&mut self) {
        self.orch
            .mark_content_dirty(iron_canvas_core::chrome::PaneRegionMask::ALL);
    }

    /// Paint whichever layers are dirty. When a recording is active
    /// (`recorder` feature only), brackets the paint with `begin_frame` /
    /// `end_frame` on both surfaces and pushes a `Frame` whenever at
    /// least one layer emitted ops. Idle rAF ticks are dropped.
    #[allow(non_snake_case)]
    pub fn paintIfDirty(&mut self) {
        #[cfg(feature = "dev-tools")]
        let recording_active = self.recording.is_some();
        #[cfg(feature = "dev-tools")]
        if recording_active {
            self.orch.grid_surface().begin_frame();
            self.orch.overlay_surface().begin_frame();
        }

        self.orch.paint_if_dirty();

        #[cfg(feature = "dev-tools")]
        if recording_active {
            self.capture_frame();
        }
    }

    /// Start a paint-level recording. Errors if a recording is already
    /// active. Both surfaces' painter-level forks are enabled; subsequent
    /// `paintIfDirty` calls capture frames until `stopRecording` (or the
    /// hard-cap watchdog) fires.
    #[cfg(feature = "dev-tools")]
    #[allow(non_snake_case)]
    pub fn startRecording(&mut self) -> Result<(), JsError> {
        if self.recording.is_some() {
            return Err(JsError::new("recording already active"));
        }
        let canvas = self.orch.canvas_size();
        let theme_snap = ThemeSnapshot::from(self.orch.theme());
        let now = js_sys::Date::now();
        let header = IcrHeader::new(canvas.w, canvas.h, theme_snap, now as u64);
        self.recording = Some(RecordingState {
            rec: Recording::new(header),
            started_at_ms: now,
            bytes_estimate: 0,
            soft_warn_fired: false,
            capped: false,
        });
        self.orch.grid_surface().enable_recording();
        self.orch.overlay_surface().enable_recording();
        // Synchronously capture a full Fresh paint as frame 0 so the
        // recording always opens with the whole canvas. Relying on the
        // host's next rAF tick is unsafe — that tick might be a narrow
        // SlotsReuse (active-cell move, single-pane content edit) and
        // frame 0 would be a tiny slice. `request_repaint` drops
        // `last_frame`, which forces the next `paint_if_dirty` to take
        // the Fresh arm; we drive it inline and call `capture_frame`
        // ourselves so the recording's first entry is the snapshot.
        self.orch.request_repaint();
        self.orch.grid_surface().begin_frame();
        self.orch.overlay_surface().begin_frame();
        self.orch.paint_if_dirty();
        self.capture_frame();
        Ok(())
    }

    /// Stop the active recording and return the serialized `.icr` bytes.
    /// If the hard-cap watchdog already fired, `header.partial` is `true`
    /// and the bytes are the truncated tail.
    #[cfg(feature = "dev-tools")]
    #[allow(non_snake_case)]
    pub fn stopRecording(&mut self) -> Result<js_sys::Uint8Array, JsError> {
        self.orch.grid_surface().disable_recording();
        self.orch.overlay_surface().disable_recording();
        let state = self
            .recording
            .take()
            .ok_or_else(|| JsError::new("no active recording"))?;
        let bytes = state
            .rec
            .serialize()
            .map_err(|e| JsError::new(&format!("recording serialize failed: {e}")))?;
        let arr = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        arr.copy_from(&bytes);
        Ok(arr)
    }

    /// Explicit teardown for React strict-mode / Leptos `Effect` mount
    /// cycles. `Drop` already handles cleanup on scope exit; this just
    /// gives JS a named callsite for the `create -> drop -> create` dance.
    pub fn dispose(self) {}

    /// JS-facing model push. Adopts the IronCalc `Model` JS handle as an
    /// opaque `JsBackedModel` after the structural duck-test in
    /// `JsBackedModel::try_from_js_value`. Returns `JsError` so the JS
    /// catch sees a real `Error` with `.message` and `.stack`.
    #[allow(non_snake_case)]
    pub fn setModel(&mut self, model: JsValue) -> Result<(), JsError> {
        let backed: Rc<dyn CanvasModel> = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.model = Some(Rc::clone(&backed));
        self.orch.set_model(backed);
        Ok(())
    }

    /// Render the current sheet as a self-contained SVG string. Returns
    /// an empty string if no model has been pushed yet. The export
    /// reads the live theme but uses a one-shot orchestrator — never
    /// touches the live grid / overlay surfaces and never fires blit
    /// (always `PaintRegime::Fresh`). Overlays (selection, marching
    /// ants, autofill handle, formula refs) are deliberately omitted
    /// — the overlay surface's SVG output is built but discarded.
    ///
    /// Why the overlay-discard strategy yields a clean grid SVG even
    /// though the throwaway orchestrator's `SelectionLayer` defaults to
    /// an A1 active cell: `LayerBase::paint_overlay_layer` invokes the
    /// `after_paint_renderer_hook` (active-cell repaint) through the
    /// **overlay** renderer's painter, not the grid's. The hook's output
    /// goes to the discarded overlay surface; the grid surface only
    /// receives `render_grid`'s cell / borders / chrome draws.
    #[allow(non_snake_case)]
    pub fn exportSvg(&self, css_w: f64, css_h: f64) -> String {
        let Some(model) = self.model.as_ref() else {
            return String::new();
        };
        let width = css_w.round() as i32;
        let height = css_h.round() as i32;

        let grid = SvgSurface::new(width, height);
        let overlay = SvgSurface::new(width, height);
        let grid_painter = grid.clone_painter();

        let mut export_orch = Orchestrator::<SvgSurface, Rc<dyn CanvasModel>>::new(grid, overlay);
        export_orch.set_theme(self.orch.theme().clone());
        export_orch.set_model(Rc::clone(model));
        export_orch.resize(CanvasSize { w: css_w, h: css_h }, 1);
        export_orch.request_repaint();
        export_orch.paint_if_dirty();
        drop(export_orch);

        grid_painter.finish()
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// JS-facing theme push from a host DOM node. Reads the upstream
    /// `--palette-*` custom properties off `el`'s computed style and
    /// builds a `CanvasTheme`.
    #[allow(non_snake_case)]
    pub fn setThemeFromElement(&mut self, el: &web_sys::Element) {
        self.orch
            .set_theme(crate::theme_from_element::from_element(el));
    }
}

// JS-facing query API. Mirrors the Rust-only query methods below, but
// crosses the wasm-bindgen boundary by serializing via `serde-wasm-bindgen`
// against the wire-shape mirrors in `crate::wire` (the engine enums use
// tuple variants that internal tagging rejects).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// Resolve the cursor against the last painted frame. See
    /// `crate::wire::HitTestWire` for the JS shape (tagged on `kind`).
    #[allow(non_snake_case)]
    pub fn hitTest(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        let wire: crate::wire::HitTestWire = self.orch.hit_test(x, y).into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    /// Pixel rect of a 1-based cell, or `null` if the cell isn't laid out
    /// (e.g. off-screen rows that the slot map hasn't materialized).
    #[allow(non_snake_case)]
    pub fn cellRect(&self, row: i32, column: i32) -> Result<JsValue, JsError> {
        match self.orch.cell_rect(row, column) {
            Some(rect) => Ok(serde_wasm_bindgen::to_value(&rect)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Resize-handle hit-test. `tolerance` is the slop band in CSS pixels.
    /// Returns `null` if no row/column trailing edge is within tolerance.
    #[allow(non_snake_case)]
    pub fn resizeHandleAt(
        &self,
        x: f64,
        y: f64,
        tolerance: f64,
    ) -> Result<JsValue, JsError> {
        match self.orch.resize_handle_at(x, y, tolerance) {
            Some(target) => {
                let wire: crate::wire::ResizeTargetWire = target.into();
                Ok(serde_wasm_bindgen::to_value(&wire)?)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Pixel position of the autofill handle, or `null` if no selection is
    /// drawn (the handle is anchored to the active selection).
    #[allow(non_snake_case)]
    pub fn autofillHandlePos(&self) -> Result<JsValue, JsError> {
        match self.orch.autofill_handle() {
            Some(p) => Ok(serde_wasm_bindgen::to_value(&p)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Layer-bypassing cell resolver. Returns `{row, column}` or `null`.
    /// Bypasses overlay layers (formula-ref dragging, etc.) that would
    /// otherwise claim the pointer.
    #[allow(non_snake_case)]
    pub fn pixelToCell(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        match self.orch.pixel_to_cell(x, y) {
            Some((row, column)) => {
                let wire = crate::wire::CellCoordWire { row, column };
                Ok(serde_wasm_bindgen::to_value(&wire)?)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Current CSS-pixel size of the drawable area: `{ w, h }`.
    #[allow(non_snake_case)]
    pub fn canvasSize(&self) -> Result<JsValue, JsError> {
        let wire: crate::wire::CanvasSizeWire = self.orch.canvas_size().into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    // ============================================================
    // Phase 2 — overlay setters.
    // ============================================================

    /// Mark the overlay layer dirty without changing state. Use after the
    /// host mutates anything the overlay reads from the model (selection,
    /// active cell, formula text) but not held in `RenderOverlays`.
    #[allow(non_snake_case)]
    pub fn requestOverlayRepaint(&mut self) {
        self.orch.request_overlay_repaint();
    }

    /// Autofill drag target. Pass `null` to clear (drag ended / cancelled).
    #[allow(non_snake_case)]
    pub fn setExtendTo(&mut self, target: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::AutofillTargetWire> =
            serde_wasm_bindgen::from_value(target)?;
        self.orch.set_extend_to(wire.map(Into::into));
        Ok(())
    }

    /// Clipboard marching-ants rectangle. Pass `null` to clear.
    #[allow(non_snake_case)]
    pub fn setClipboard(&mut self, area: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::SheetAreaWire> = serde_wasm_bindgen::from_value(area)?;
        self.orch.set_clipboard(wire.map(Into::into));
        Ok(())
    }

    /// Formula-entry point-mode range highlight. Pass `null` to clear.
    #[allow(non_snake_case)]
    pub fn setPointRange(&mut self, range: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::RCRangeWire> = serde_wasm_bindgen::from_value(range)?;
        self.orch.set_point_range(wire.map(Into::into));
        Ok(())
    }

    /// Replace the per-formula draggable references. JS-side `active_ref`
    /// indices stale after this call must be re-pushed via `setOverlays`;
    /// the renderer is defensive (uses `.get()`), so a stale index is
    /// silently ignored rather than panicking.
    #[allow(non_snake_case)]
    pub fn setFormulaRefs(&mut self, refs: JsValue) -> Result<(), JsError> {
        let wire: Vec<crate::wire::FormulaRefWire> = serde_wasm_bindgen::from_value(refs)?;
        let refs: Vec<iron_canvas_core::FormulaRef> = wire.into_iter().map(Into::into).collect();
        self.orch.set_formula_refs(refs);
        Ok(())
    }

    /// Full overlay-state push. Validates `active_ref < formula_refs.len()`
    /// at the boundary — a violating payload throws a `JsError` rather
    /// than silently dropping the highlight on the renderer side.
    #[allow(non_snake_case)]
    pub fn setOverlays(&mut self, overlays: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::RenderOverlaysWire = serde_wasm_bindgen::from_value(overlays)?;
        let engine = wire.into_engine().map_err(|msg| JsError::new(&msg))?;
        self.orch.set_overlays(engine);
        Ok(())
    }

    // ============================================================
    // Phase 3 — theme setters.
    // ============================================================

    /// Full theme push. Every palette field must be present — missing keys
    /// throw a `JsError`. For partial overrides with a LIGHT fallback, use
    /// `setThemeVariables`. For CSS-var driven themes, prefer the existing
    /// `setThemeFromElement`.
    #[allow(non_snake_case)]
    pub fn setTheme(&mut self, theme: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::CanvasThemeWire = serde_wasm_bindgen::from_value(theme)?;
        self.orch.set_theme(wire.into());
        Ok(())
    }

    /// Partial theme override. Each key is optional; missing keys fall back
    /// to `CanvasTheme::light()` via the engine's
    /// `From<ThemeVariables> for CanvasTheme` impl.
    #[allow(non_snake_case)]
    pub fn setThemeVariables(&mut self, vars: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::ThemeVariablesWire = serde_wasm_bindgen::from_value(vars)?;
        self.orch.set_theme_variables(wire.into());
        Ok(())
    }
}

// Rust-only API. Counterpart to the `#[wasm_bindgen]` block above —
// these methods take Rust types that don't cross the JS bridge.
impl IronCanvas {
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        self.orch.set_overlays(overlays);
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        self.orch.set_extend_to(target);
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        self.orch.set_clipboard(area);
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        self.orch.set_point_range(range);
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        self.orch.set_formula_refs(refs);
    }

    pub fn set_theme(&mut self, theme: CanvasTheme) {
        self.orch.set_theme(theme);
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.orch.set_theme_variables(vars);
    }

    /// Rust-level model push. Accepts any `CanvasModel` impl behind an
    /// `Rc` — Leptos-side adapters that bridge a host store to the canvas
    /// (e.g. `WorksheetModelAdapter`) route through here.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.model = Some(Rc::clone(&model));
        self.orch.set_model(model);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.orch.canvas_size()
    }

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        self.orch.hit_test(x, y)
    }

    /// Layer-bypassing cell resolver. Use during an active drag whose
    /// overlay (e.g. `FormulaRefsLayer`) would otherwise claim the
    /// pointer and starve the host of underlying cell coordinates.
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        self.orch.pixel_to_cell(x, y)
    }

    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.orch.resize_handle_at(x, y, tolerance)
    }

    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.orch.cell_rect(row, column)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.orch.autofill_handle()
    }

    pub fn request_overlay_repaint(&mut self) {
        self.orch.request_overlay_repaint();
    }

    /// Drain the per-frame op buffers, push a `Frame` (skipping empty
    /// ones), update the running cap estimate, fire soft-warn / hard-cap
    /// side effects.
    ///
    /// Hard cap: flips `partial`, disables both painter-level forks, but
    /// keeps `self.recording` populated so `stopRecording` can still
    /// drain the partial bytes.
    #[cfg(feature = "dev-tools")]
    fn capture_frame(&mut self) {
        let grid_ops = self.orch.grid_surface().end_frame();
        let overlay_ops = self.orch.overlay_surface().end_frame();
        if grid_ops.is_empty() && overlay_ops.is_empty() {
            return;
        }
        let regime = self.orch.last_regime().unwrap_or(PaintRegimeTag::Fresh);
        let signals = self.orch.last_signals().bits();
        let Some(state) = self.recording.as_mut() else {
            return;
        };
        let now = js_sys::Date::now();
        let t_ms = (now - state.started_at_ms).max(0.0) as u64;
        let frame_idx = state.rec.frames.len() as u32;
        let frame_bytes: usize = grid_ops.iter().map(op_bytes).sum::<usize>()
            + overlay_ops.iter().map(op_bytes).sum::<usize>();
        state.rec.push_frame(Frame {
            frame_idx,
            t_ms,
            regime,
            signals,
            grid_ops,
            overlay_ops,
        });
        state.bytes_estimate = state.bytes_estimate.saturating_add(frame_bytes);

        if !state.soft_warn_fired && t_ms > SOFT_WARN_MS {
            state.soft_warn_fired = true;
            crate::wasm::diag::console_warn(
                "iron-canvas: recording > 30s — call stopRecording() soon",
            );
        }

        if !state.capped && state.bytes_estimate > HARD_CAP_BYTES {
            state.capped = true;
            state.rec.header.partial = true;
            self.orch.grid_surface().disable_recording();
            self.orch.overlay_surface().disable_recording();
            crate::wasm::diag::console_warn(
                "iron-canvas: recording exceeded 100MB cap; auto-stopped (partial)",
            );
        }
    }
}
