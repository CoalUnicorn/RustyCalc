//! Web-side facade. The frame dispatch + state aggregator now lives in
//! `iron_canvas_core::Orchestrator`; this struct holds the `wasm-bindgen`
//! handle, builds two `WebSurface`s, and delegates every setter / query /
//! paint call to the core orchestrator.

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::RenderOverlays;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::wasm::JsBackedModel;
use iron_canvas_canvas2d::WebSurface;
// `Surface` is only needed by the dev-tools recording path
// (`grid_surface().painter()`); the export helpers no longer use it.
use iron_canvas_core::CanvasModel;
use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
#[cfg(feature = "dev-tools")]
use iron_canvas_core::layer::Surface;
use iron_canvas_core::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};
use iron_canvas_export::SvgSurface;
#[cfg(feature = "pdf")]
use iron_canvas_export::pdf::PdfSurface;

#[cfg(feature = "dev-tools")]
use crate::playback::{PlayClock, PlaybackSession, replay_through};
#[cfg(feature = "dev-tools")]
use iron_canvas_core::PaintRegimeTag;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::DrawOp;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::recording::{Frame, IcrHeader, Recording, ThemeSnapshot};
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::{RecordingFilter, RecordingSurface};

/// Facade Surface — wraps `WebSurface` in `RecordingSurface` when the
/// `dev-tools` feature is on (dev builds), bare `WebSurface` otherwise
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
        DrawOp::BeginGroup { class } => OP_BYTES_HEURISTIC + class.as_str().len(),
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

/// The three mutually-exclusive lifecycle states of dev-tools capture /
/// replay. `Live` is the only state in which `paintIfDirty` actually
/// paints; `Recording(_)` adds frame capture; `Playback(_)` short-circuits
/// the host's rAF loop and replays recorded ops through the live painters.
#[cfg(feature = "dev-tools")]
enum CanvasMode {
    Live,
    Recording(RecordingState),
    Playback(PlaybackSession),
}

#[wasm_bindgen]
pub struct IronCanvas {
    orch: Orchestrator<FacadeSurface>,
    // Cached so SVG export can re-push the live model into a throwaway
    // orchestrator. Updated alongside every `set_model` / `setModel`.
    model: Option<Rc<dyn CanvasModel>>,
    // Typed twin of `model`, kept only when the model came through `setModel`
    // (the JS path) — `themeChanged` needs `JsBackedModel::theme_changed`,
    // which the type-erased `Rc<dyn CanvasModel>` can't reach. `None` for
    // Rust-level models, whose theme handling lives host-side.
    js_model: Option<Rc<JsBackedModel>>,
    // Live DPR — the engine doesn't retain it across `resize` calls, and
    // both the recording header and playback restore need it. Initialized
    // to `1` because some entry paths (the test surface) call `startRecording`
    // before any `resize`, and a DPR of `0` in the recording header would
    // round-trip through playback nonsensically. Only read under `dev-tools`.
    #[cfg_attr(not(feature = "dev-tools"), allow(dead_code))]
    last_dpr: f64,
    #[cfg(feature = "dev-tools")]
    mode: CanvasMode,
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
            orch: Orchestrator::<FacadeSurface>::new(grid, overlay),
            model: None,
            js_model: None,
            last_dpr: 1.0,
            #[cfg(feature = "dev-tools")]
            mode: CanvasMode::Live,
        })
    }

    /// Whether this build supports `startRecording` / `stopRecording`.
    /// Always callable from JS so the host can hide its Record button on
    /// prod builds without `try`-sniffing the class shape.
    #[wasm_bindgen(js_name = "recordingSupported")]
    pub fn recording_supported() -> bool {
        cfg!(feature = "dev-tools")
    }

    /// Resize both layers in one call.
    pub fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.last_dpr = dpr;
        self.orch.resize(CanvasSize { w: css_w, h: css_h }, dpr);
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
        self.restamp_recording_theme();
    }

    /// Conservative repaint blanket — see `Orchestrator::request_repaint`.
    #[wasm_bindgen(js_name = "requestRepaint")]
    pub fn request_repaint(&mut self) {
        self.orch.request_repaint();
    }

    /// JS-facing cell-content-changed signal — marks all four pane
    /// quadrants. Pane-granular masks stay Rust-internal.
    #[wasm_bindgen(js_name = "markContentDirty")]
    pub fn mark_content_dirty(&mut self) {
        self.orch
            .mark_content_dirty(iron_canvas_core::chrome::PaneRegionMask::ALL);
    }

    /// Paint whichever layers are dirty. When a recording is active
    /// (`dev-tools` feature only), brackets the paint with `begin_frame` /
    /// `end_frame` on both surfaces and pushes a `Frame` whenever at
    /// least one layer emitted ops. Idle rAF ticks are dropped.
    #[wasm_bindgen(js_name = "paintIfDirty")]
    pub fn paint_if_dirty(&mut self) {
        #[cfg(feature = "dev-tools")]
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return;
        }
        #[cfg(feature = "dev-tools")]
        let recording_active = matches!(self.mode, CanvasMode::Recording(_));
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
    /// active. `opts` is an optional `RecordingFilter` JS object —
    /// `{ layers?: "both"|"gridOnly"|"overlayOnly", skipGroups?: string[] }`.
    /// Undefined/null means "record everything" (default filter).
    /// Layer scope decides which surfaces fork ops; `skipGroups` drops
    /// named `begin_group` brackets (and their contents) within recorded
    /// surfaces. Subsequent `paintIfDirty` calls capture frames until
    /// `stopRecording` (or the hard-cap watchdog) fires.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "startRecording")]
    pub fn start_recording(&mut self, opts: JsValue) -> Result<(), JsError> {
        match &self.mode {
            CanvasMode::Live => {}
            CanvasMode::Recording(_) => {
                return Err(JsError::new("recording already active"));
            }
            CanvasMode::Playback(_) => {
                return Err(JsError::new("cannot start a recording during playback"));
            }
        }
        // `undefined` / `null` -> default filter (record everything).
        // Anything else is parsed as a `RecordingFilter` shape.
        let filter: RecordingFilter = if opts.is_undefined() || opts.is_null() {
            RecordingFilter::default()
        } else {
            serde_wasm_bindgen::from_value(opts)?
        };
        let canvas = self.orch.canvas_size();
        let theme_snap = ThemeSnapshot::from(self.orch.theme());
        let now = js_sys::Date::now();
        let header = IcrHeader::new(canvas.w, canvas.h, self.last_dpr, theme_snap, now as u64);
        self.mode = CanvasMode::Recording(RecordingState {
            rec: Recording::new(header),
            started_at_ms: now,
            bytes_estimate: 0,
            soft_warn_fired: false,
            capped: false,
        });
        if filter.layers.includes_grid() {
            self.orch
                .grid_surface()
                .set_skip_groups(filter.skip_groups.clone());
            self.orch.grid_surface().enable_recording();
        }
        if filter.layers.includes_overlay() {
            self.orch
                .overlay_surface()
                .set_skip_groups(filter.skip_groups);
            self.orch.overlay_surface().enable_recording();
        }
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
    #[wasm_bindgen(js_name = "stopRecording")]
    pub fn stop_recording(&mut self) -> Result<js_sys::Uint8Array, JsError> {
        if !matches!(self.mode, CanvasMode::Recording(_)) {
            return Err(JsError::new("no active recording"));
        }
        self.orch.grid_surface().disable_recording();
        self.orch.overlay_surface().disable_recording();
        let CanvasMode::Recording(state) = std::mem::replace(&mut self.mode, CanvasMode::Live)
        else {
            unreachable!("guarded above by matches!(Recording(_))")
        };
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
        let backed = Rc::new(JsBackedModel::try_from_js_value(model)?);
        self.js_model = Some(Rc::clone(&backed));
        let erased: Rc<dyn CanvasModel> = backed;
        self.model = Some(Rc::clone(&erased));
        self.orch.set_model(erased);
        Ok(())
    }

    /// Host contract for workbook-theme changes: call after
    /// `model.setTheme(...)`. Drops the bridge's cached theme and marks
    /// content dirty — a theme swap changes every style fingerprint, so the
    /// next `paintIfDirty` repaints from fresh fetches. Without this call the
    /// stale cache silently misrenders theme colors (no error, host bug).
    /// No-op beyond the dirty mark for Rust-level models.
    #[wasm_bindgen(js_name = "themeChanged")]
    pub fn theme_changed(&mut self) {
        if let Some(m) = &self.js_model {
            m.theme_changed();
        }
        self.mark_content_dirty();
    }

    /// Render the current sheet as a self-contained SVG string. Returns
    /// an empty string if no model has been pushed yet. Reads the live
    /// theme but delegates to `SvgSurface::render`, which drives a
    /// one-shot orchestrator — never touching the live surfaces and
    /// never firing blit. See `SvgSurface::render` for the
    /// overlay-discard policy.
    #[wasm_bindgen(js_name = "exportSvg")]
    pub fn export_svg(&self, css_w: f64, css_h: f64) -> String {
        let Some(model) = self.model.as_ref() else {
            return String::new();
        };
        SvgSurface::render(
            Rc::clone(model),
            self.orch.theme(),
            CanvasSize { w: css_w, h: css_h },
        )
    }

    /// JS-facing PDF export. Mirrors `exportSvg`'s overlay-discard
    /// policy via `PdfSurface::render`. `Vec<u8>` auto-converts to
    /// `Uint8Array` across the wasm-bindgen boundary.
    #[cfg(feature = "pdf")]
    #[wasm_bindgen(js_name = "exportPdf")]
    pub fn export_pdf(&self, css_w: f64, css_h: f64) -> Vec<u8> {
        let Some(model) = self.model.as_ref() else {
            return Vec::new();
        };
        PdfSurface::render(
            Rc::clone(model),
            self.orch.theme(),
            CanvasSize { w: css_w, h: css_h },
        )
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl IronCanvas {
    /// JS-facing theme push from a host DOM node. Reads the upstream
    /// `--palette-*` custom properties off `el`'s computed style and
    /// builds a `CanvasTheme`.
    #[wasm_bindgen(js_name = "setThemeFromElement")]
    pub fn set_theme_from_element(&mut self, el: &web_sys::Element) {
        self.orch
            .set_theme(crate::theme_from_element::from_element(el));
        self.restamp_recording_theme();
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
    #[wasm_bindgen(js_name = "hitTest")]
    pub fn hit_test_js(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        let wire: crate::wire::HitTestWire = self.orch.hit_test(x, y).into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    /// Pixel rect of a 1-based cell, or `null` if the cell isn't laid out
    /// (e.g. off-screen rows that the slot map hasn't materialized).
    #[wasm_bindgen(js_name = "cellRect")]
    pub fn cell_rect_js(&self, row: i32, column: i32) -> Result<JsValue, JsError> {
        match self.orch.cell_rect(row, column) {
            Some(rect) => Ok(serde_wasm_bindgen::to_value(&rect)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Resize-handle hit-test. `tolerance` is the slop band in CSS pixels.
    /// Returns `null` if no row/column trailing edge is within tolerance.
    #[wasm_bindgen(js_name = "resizeHandleAt")]
    pub fn resize_handle_at_js(&self, x: f64, y: f64, tolerance: f64) -> Result<JsValue, JsError> {
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
    #[wasm_bindgen(js_name = "autofillHandlePos")]
    pub fn autofill_handle_pos(&self) -> Result<JsValue, JsError> {
        match self.orch.autofill_handle() {
            Some(p) => Ok(serde_wasm_bindgen::to_value(&p)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Layer-bypassing cell resolver. Returns `{row, column}` or `null`.
    /// Bypasses overlay layers (formula-ref dragging, etc.) that would
    /// otherwise claim the pointer.
    #[wasm_bindgen(js_name = "pixelToCell")]
    pub fn pixel_to_cell_js(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        match self.orch.pixel_to_cell(x, y) {
            Some((row, column)) => {
                let wire = crate::wire::CellCoordWire { row, column };
                Ok(serde_wasm_bindgen::to_value(&wire)?)
            }
            None => Ok(JsValue::NULL),
        }
    }

    /// Current CSS-pixel size of the drawable area: `{ w, h }`.
    #[wasm_bindgen(js_name = "canvasSize")]
    pub fn canvas_size_js(&self) -> Result<JsValue, JsError> {
        let wire: crate::wire::CanvasSizeWire = self.orch.canvas_size().into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    // ============================================================
    // Phase 2 — overlay setters.
    // ============================================================

    /// Mark the overlay layer dirty without changing state. Use after the
    /// host mutates anything the overlay reads from the model (selection,
    /// active cell, formula text) but not held in `RenderOverlays`.
    #[wasm_bindgen(js_name = "requestOverlayRepaint")]
    pub fn request_overlay_repaint_js(&mut self) {
        self.orch.request_overlay_repaint();
    }

    /// Autofill drag target. Pass `null` to clear (drag ended / cancelled).
    #[wasm_bindgen(js_name = "setExtendTo")]
    pub fn set_extend_to_js(&mut self, target: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::AutofillTargetWire> = serde_wasm_bindgen::from_value(target)?;
        self.orch.set_extend_to(wire.map(Into::into));
        Ok(())
    }

    /// Clipboard marching-ants rectangle. Pass `null` to clear.
    #[wasm_bindgen(js_name = "setClipboard")]
    pub fn set_clipboard_js(&mut self, area: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::SheetAreaWire> = serde_wasm_bindgen::from_value(area)?;
        self.orch.set_clipboard(wire.map(Into::into));
        Ok(())
    }

    /// Formula-entry point-mode range highlight. Pass `null` to clear.
    #[wasm_bindgen(js_name = "setPointRange")]
    pub fn set_point_range_js(&mut self, range: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::RCRangeWire> = serde_wasm_bindgen::from_value(range)?;
        self.orch.set_point_range(wire.map(Into::into));
        Ok(())
    }

    /// Replace the per-formula draggable references.
    #[wasm_bindgen(js_name = "setFormulaRefs")]
    pub fn set_formula_refs_js(&mut self, refs: JsValue) -> Result<(), JsError> {
        let wire: Vec<crate::wire::FormulaRefWire> = serde_wasm_bindgen::from_value(refs)?;
        let refs: Vec<iron_canvas_core::FormulaRef> = wire.into_iter().map(Into::into).collect();
        self.orch.set_formula_refs(refs);
        Ok(())
    }

    /// Full overlay-state push. Currently infallible at the boundary; the
    /// `Result` is preserved so future invariant checks can surface as a
    /// `JsError` without touching the call site.
    #[wasm_bindgen(js_name = "setOverlays")]
    pub fn set_overlays_js(&mut self, overlays: JsValue) -> Result<(), JsError> {
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
    #[wasm_bindgen(js_name = "setTheme")]
    pub fn set_theme_js(&mut self, theme: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::CanvasThemeWire = serde_wasm_bindgen::from_value(theme)?;
        self.orch.set_theme(wire.into());
        self.restamp_recording_theme();
        Ok(())
    }

    /// Partial theme override. Each key is optional; missing keys fall back
    /// to `CanvasTheme::light()` via the engine's
    /// `From<ThemeVariables> for CanvasTheme` impl.
    #[wasm_bindgen(js_name = "setThemeVariables")]
    pub fn set_theme_variables_js(&mut self, vars: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::ThemeVariablesWire = serde_wasm_bindgen::from_value(vars)?;
        self.orch.set_theme_variables(wire.into());
        self.restamp_recording_theme();
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
        self.restamp_recording_theme();
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.orch.set_theme_variables(vars);
        self.restamp_recording_theme();
    }

    /// Rust-level model push. Accepts any `CanvasModel` impl behind an
    /// `Rc` — Leptos-side adapters that bridge a host store to the canvas
    /// (e.g. `WorksheetModelAdapter`) route through here.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.model = Some(Rc::clone(&model));
        // A Rust-level model replaces any JS one; its theme is host-resolved.
        self.js_model = None;
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

    pub fn fit_column_width(&self, col: i32, first_row: i32, last_row: i32) -> Option<f64> {
        self.orch.fit_column_width(col, first_row, last_row)
    }

    pub fn fit_row_height(&self, row: i32, first_col: i32, last_col: i32) -> Option<f64> {
        self.orch.fit_row_height(row, first_col, last_col)
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
    /// keeps the `Recording` mode populated so `stopRecording` can still
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
        let CanvasMode::Recording(state) = &mut self.mode else {
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

    /// Re-stamp the active recording's header with the current theme so
    /// playback metadata stays consistent with the resolved hex colors
    /// embedded in subsequent ops. Called from every theme setter on
    /// `IronCanvas` — without it, a `setTheme` mid-recording would leave
    /// the header pinned to the initial palette while op colors carry
    /// the new one. No-op when recording is inactive or the feature is
    /// disabled at build time.
    #[cfg(feature = "dev-tools")]
    fn restamp_recording_theme(&mut self) {
        if let CanvasMode::Recording(state) = &mut self.mode {
            state.rec.header.theme = ThemeSnapshot::from(self.orch.theme());
        }
    }

    #[cfg(not(feature = "dev-tools"))]
    #[inline]
    fn restamp_recording_theme(&mut self) {
        let _ = self;
    }
}

// ============================================================================
// Recording playback (dev-tools)
//
// Live-canvas playback: rents the worksheet's grid + overlay painters and
// replays recorded `DrawOp`s through them. `paintIfDirty` short-circuits
// while a session is loaded, so the orchestrator never observes playback.
// `exitPlayback` calls `request_repaint` to force a `Fresh` next tick, which
// restores the live worksheet from scratch.
// ============================================================================

#[cfg(feature = "dev-tools")]
#[wasm_bindgen]
impl IronCanvas {
    /// Parse `.icr` bytes, enter playback, and paint frame 0.
    /// While a session is live, `paintIfDirty` is a no-op.
    ///
    /// Refuses while a recording is being captured — the live grid /
    /// overlay surfaces are wrapped in `RecordingPainter`, so the seek's
    /// replay ops would otherwise fork into the active recording buffer
    /// and corrupt the `.icr`.
    ///
    /// Resizes the orchestrator and the canvas backing stores to the
    /// recording's dimensions and overrides their inline CSS width /
    /// height so the displayed canvas matches. `exitPlayback` restores
    /// both. The pre-playback CSS dimensions and DPR are stashed on the
    /// session.
    #[wasm_bindgen(js_name = "loadRecording")]
    pub fn load_recording(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        if matches!(self.mode, CanvasMode::Recording(_)) {
            return Err(JsError::new(
                "cannot load a recording while a capture is active",
            ));
        }
        let rec = Recording::deserialize(bytes)
            .map_err(|e| JsError::new(&format!("recording deserialize failed: {e}")))?;
        if rec.frames.is_empty() {
            return Err(JsError::new("recording has no frames"));
        }
        let live_size = self.orch.canvas_size();
        let live_dpr = self.last_dpr;
        let rec_size = CanvasSize {
            w: rec.header.canvas_w,
            h: rec.header.canvas_h,
        };
        let rec_dpr = rec.header.dpr;

        // Resize the orchestrator + canvas backing stores to recording
        // dims. Inline CSS on the canvases is overridden separately —
        // the `.ws-canvas` class pins display size to `100%`, which would
        // otherwise scale the backing store back down to the container.
        self.last_dpr = rec_dpr;
        self.orch.resize(rec_size, rec_dpr);
        set_canvas_css_size(self.orch.grid_surface().inner().canvas(), rec_size)?;
        set_canvas_css_size(self.orch.overlay_surface().inner().canvas(), rec_size)?;

        self.mode = CanvasMode::Playback(PlaybackSession::new(rec, live_size, live_dpr));
        self.seek_recording_inner(0)
    }

    /// Seek to `frame_idx` (clamped to the recording length) and repaint.
    /// Pauses an active play loop so the host's next `tickPlayback` doesn't
    /// jump back to whatever the stale play-anchor projected.
    ///
    /// Refuses during an active capture — see `loadRecording` for the
    /// same reason.
    #[wasm_bindgen(js_name = "seekRecording")]
    pub fn seek_recording(&mut self, frame_idx: u32) -> Result<(), JsError> {
        match &mut self.mode {
            CanvasMode::Recording(_) => {
                return Err(JsError::new(
                    "cannot seek a recording while a capture is active",
                ));
            }
            // Fall through; `seek_recording_inner` produces "no recording loaded".
            CanvasMode::Live => {}
            CanvasMode::Playback(s) => {
                s.clock = PlayClock::Paused;
            }
        }
        self.seek_recording_inner(frame_idx)
    }

    /// Begin time-accurate playback. `now_ms` is the host's
    /// `performance.now()` reading captured at the same instant as the
    /// call — used as the wall-clock anchor for subsequent ticks. Errs
    /// when no recording is loaded.
    ///
    /// At end-of-recording, this is a no-op (the host decides whether to
    /// rewind via `seekRecording(0)`). Two arms of `tickPlayback`'s
    /// auto-pause contract would otherwise diverge: `Play` from the last
    /// frame would restart the timeline, but `Play` arriving one frame
    /// later (after the auto-pause) does not.
    #[wasm_bindgen(js_name = "playRecording")]
    pub fn play_recording(&mut self, now_ms: f64) -> Result<(), JsError> {
        let CanvasMode::Playback(s) = &mut self.mode else {
            return Err(JsError::new("no recording loaded"));
        };
        if s.frame_idx + 1 >= s.frame_count() {
            return Ok(());
        }
        s.anchor(now_ms);
        Ok(())
    }

    /// Halt the play loop. Idempotent.
    #[wasm_bindgen(js_name = "pauseRecording")]
    pub fn pause_recording(&mut self) {
        if let CanvasMode::Playback(s) = &mut self.mode {
            s.clock = PlayClock::Paused;
        }
    }

    /// `true` while play is active. `false` when paused, at end-of-recording,
    /// or when no recording is loaded.
    #[wasm_bindgen(js_name = "isPlaying")]
    pub fn is_playing(&self) -> bool {
        matches!(
            &self.mode,
            CanvasMode::Playback(s) if matches!(s.clock, PlayClock::Playing { .. })
        )
    }

    /// Drive playback forward to whichever frame matches `now_ms` against
    /// the play-anchor. Returns `true` if the displayed frame changed.
    /// Auto-pauses on reaching the last frame. No-op when no session is
    /// loaded or `playing == false`.
    ///
    /// Host pattern: call from the same rAF loop that drives `paintIfDirty`.
    /// `paintIfDirty` short-circuits while a session is loaded, so the two
    /// never paint in the same tick.
    #[wasm_bindgen(js_name = "tickPlayback")]
    pub fn tick_playback(&mut self, now_ms: f64) -> bool {
        let CanvasMode::Playback(s) = &mut self.mode else {
            return false;
        };
        let PlayClock::Playing {
            anchor_ms,
            anchor_frame_idx,
        } = s.clock
        else {
            return false;
        };
        let target = s.target_frame_for(anchor_ms, anchor_frame_idx, now_ms);
        let last = s.frame_count().saturating_sub(1);
        let changed = target != s.frame_idx;
        if target >= last {
            s.clock = PlayClock::Paused;
        }
        if !changed {
            return false;
        }
        // Ignore inner errors: target is computed from the session's own
        // frame_count, so the only failure mode (no session) can't fire here.
        let _ = self.seek_recording_inner(target);
        true
    }

    /// Drop the active session and request a Fresh repaint so the live
    /// worksheet returns on the next rAF tick. Restores the canvas CSS
    /// dimensions and resizes the orchestrator back to the pre-playback
    /// live size + DPR. No-op when no session is loaded.
    #[wasm_bindgen(js_name = "exitPlayback")]
    pub fn exit_playback(&mut self) {
        let CanvasMode::Playback(session) = std::mem::replace(&mut self.mode, CanvasMode::Live)
        else {
            return;
        };
        // Clear the inline overrides so the `.ws-canvas { width: 100%;
        // height: 100% }` rule controls display size again.
        let _ = clear_canvas_css_size(self.orch.grid_surface().inner().canvas());
        let _ = clear_canvas_css_size(self.orch.overlay_surface().inner().canvas());

        self.last_dpr = session.live_dpr;
        self.orch.resize(session.live_size, session.live_dpr);
        self.orch.request_repaint();
    }

    /// `true` while a playback session is loaded.
    #[wasm_bindgen(js_name = "playbackActive")]
    pub fn playback_active(&self) -> bool {
        matches!(self.mode, CanvasMode::Playback(_))
    }

    /// Total frames in the loaded recording, or 0 if none loaded.
    #[wasm_bindgen(js_name = "recordingFrameCount")]
    pub fn recording_frame_count(&self) -> u32 {
        if let CanvasMode::Playback(s) = &self.mode {
            s.frame_count()
        } else {
            0
        }
    }

    /// Current playback frame index, or 0 if none loaded.
    #[wasm_bindgen(js_name = "recordingCurrentFrame")]
    pub fn recording_current_frame(&self) -> u32 {
        if let CanvasMode::Playback(s) = &self.mode {
            s.frame_idx
        } else {
            0
        }
    }
}

/// Override `<canvas>` inline width / height (CSS pixels) so the
/// displayed canvas matches `size`. The `.ws-canvas` CSS class pins
/// display size to `100%`; without an inline override the recording's
/// backing-store resize would be scaled back down to the container.
#[cfg(feature = "dev-tools")]
fn set_canvas_css_size(canvas: &HtmlCanvasElement, size: CanvasSize) -> Result<(), JsError> {
    let style = canvas.style();
    style
        .set_property("width", &format!("{}px", size.w))
        .map_err(|_| JsError::new("failed to set canvas inline width"))?;
    style
        .set_property("height", &format!("{}px", size.h))
        .map_err(|_| JsError::new("failed to set canvas inline height"))?;
    Ok(())
}

/// Clear the inline width / height overrides set by
/// `set_canvas_css_size` so the `.ws-canvas` CSS rule controls display
/// size again.
#[cfg(feature = "dev-tools")]
fn clear_canvas_css_size(canvas: &HtmlCanvasElement) -> Result<(), JsError> {
    let style = canvas.style();
    style
        .remove_property("width")
        .map_err(|_| JsError::new("failed to clear canvas inline width"))?;
    style
        .remove_property("height")
        .map_err(|_| JsError::new("failed to clear canvas inline height"))?;
    Ok(())
}

#[cfg(feature = "dev-tools")]
impl IronCanvas {
    fn seek_recording_inner(&mut self, frame_idx: u32) -> Result<(), JsError> {
        let CanvasMode::Playback(session) = &mut self.mode else {
            return Err(JsError::new("no recording loaded"));
        };
        let count = session.frame_count();
        if count == 0 {
            return Err(JsError::new("recording has no frames"));
        }
        let clamped = frame_idx.min(count - 1);
        session.frame_idx = clamped;

        let grid = self.orch.grid_surface().painter();
        let overlay = self.orch.overlay_surface().painter();
        replay_through(grid, overlay, &session.recording, clamped);
        Ok(())
    }
}
