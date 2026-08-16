//! Web-side facade. The frame dispatch + state aggregator now lives in
//! `iron_canvas_core::Orchestrator`; this struct holds the `wasm-bindgen`
//! handle and delegates facade setters, queries, recording, and playback to
//! the shared Canvas2D runtime/core orchestrator.

use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::RenderOverlays;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::wasm::JsBackedModel;
use iron_canvas_canvas2d::{Canvas2dRuntime, WebSurface};
use iron_canvas_core::CanvasModel;
use iron_canvas_core::PaintResult;
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
use iron_canvas_recorder::DrawOp;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::recording::{
    Frame, IcrHeader, RecordOrigin, RecordedPaintResult, Recording, ThemeSnapshot, TraceRecord,
};
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
#[cfg(feature = "dev-tools")]
const ATTEMPT_METADATA_BYTES_HEURISTIC: usize = 256;

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

/// Wire result of one `paintIfDirty()` call. Mirrors
/// `iron_canvas_core::PaintResult` plus the dev-tools playback
/// short-circuit, which never reaches the core orchestrator at all. C-style
/// enum — no per-frame `String`/data allocation, and the match in
/// `paint_if_dirty` stays exhaustive against both sources.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JsPaintResult {
    Idle,
    Painted,
    Retry,
    /// Dev-tools playback short-circuit: no core paint ran this tick.
    Playback,
}

#[wasm_bindgen]
pub struct IronCanvas {
    runtime: Canvas2dRuntime<FacadeSurface>,
    // Cached so SVG export can re-push the live model into a throwaway
    // orchestrator. Updated alongside every `set_model` / `setModel`.
    model: Option<Rc<dyn CanvasModel>>,
    // Typed twin of `model`, kept only when the model came through `setModel`
    // (the JS path) — `themeChanged` needs `JsBackedModel::theme_changed`,
    // which the type-erased `Rc<dyn CanvasModel>` can't reach. `None` for
    // Rust-level models, whose theme handling lives host-side.
    js_model: Option<Rc<JsBackedModel>>,
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
        let runtime = Canvas2dRuntime::new_with_wrapper(grid_canvas, overlay_canvas, wrap_surface)?;
        Ok(IronCanvas {
            runtime,
            model: None,
            js_model: None,
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
        self.runtime.resize(CanvasSize { w: css_w, h: css_h }, dpr);
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
        self.runtime.orchestrator_mut().set_theme(theme);
        self.restamp_recording_theme();
    }

    /// Conservative repaint blanket — see `Orchestrator::request_repaint`.
    #[wasm_bindgen(js_name = "requestRepaint")]
    pub fn request_repaint(&mut self) {
        self.runtime.orchestrator_mut().request_repaint();
    }

    /// JS-facing cell-content-changed signal for conservative grid-wide work.
    #[wasm_bindgen(js_name = "markContentDirty")]
    pub fn mark_content_dirty(&mut self) {
        self.runtime.orchestrator_mut().mark_content_dirty();
    }

    /// Row-scoped `markContentDirty`: names the damaged rows so the engine
    /// can clip the repaint to those bands (typing, single-cell edit,
    /// recalc diff). Degrades to the full content path automatically when
    /// damage info is incomplete. Rows are inclusive, order-insensitive
    /// (`PendingWork::mark_rows` normalizes reversed spans via
    /// `RowSpan::normalized`), in the same row coordinates the model bridge
    /// uses; rows outside the viewport intersect to nothing at paint time
    /// and cost nothing.
    #[wasm_bindgen(js_name = "markRowsDamaged")]
    pub fn mark_rows_damaged(&mut self, sheet: u32, row_start: i32, row_end: i32) {
        self.runtime.orchestrator_mut().mark_rows_damaged(
            sheet,
            iron_canvas_core::RowSpan {
                r1: row_start,
                r2: row_end,
            },
        );
    }

    /// Paint whichever layers are dirty. When a recording is active
    /// (`dev-tools` feature only), brackets the paint with `begin_frame` /
    /// `end_frame` on both surfaces and pushes an attempt for every non-idle
    /// result, including zero-op holds. Idle rAF ticks are dropped.
    ///
    /// Returns the outcome so the host's rAF loop can decide whether to keep
    /// itself armed (`Retry`) and whether this tick is worth attributing
    /// diagnostics to — see `JsPaintResult`.
    #[wasm_bindgen(js_name = "paintIfDirty")]
    pub fn paint_if_dirty(&mut self) -> JsPaintResult {
        #[cfg(feature = "dev-tools")]
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return JsPaintResult::Playback;
        }
        #[cfg(feature = "dev-tools")]
        let recording_active = matches!(self.mode, CanvasMode::Recording(_));
        #[cfg(feature = "dev-tools")]
        if recording_active {
            self.runtime.orchestrator().grid_surface().begin_frame();
            self.runtime.orchestrator().overlay_surface().begin_frame();
        }

        let core_result = self.runtime.orchestrator_mut().paint_if_dirty();
        let result = match core_result {
            PaintResult::Idle => JsPaintResult::Idle,
            PaintResult::Painted => JsPaintResult::Painted,
            PaintResult::Retry => JsPaintResult::Retry,
        };

        #[cfg(feature = "dev-tools")]
        if recording_active {
            self.capture_frame(core_result, RecordOrigin::Live);
        }

        result
    }

    /// One-line attribution for the last painted frame: regime, grid verdict,
    /// and the cell slots handed to the model.
    /// Poll it per rAF tick to see which path a spiking frame took —
    /// `Viewport grid:strip … ` then `SlotsReuse grid:FULL …` is the post-blit
    /// full repaint that
    /// `docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md` targets.
    #[wasm_bindgen(js_name = "frameTrace")]
    pub fn frame_trace(&self) -> String {
        #[cfg(feature = "dev-tools")]
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return String::new();
        }
        self.runtime.orchestrator().last_trace().to_string()
    }

    /// Enable structured capture for the next `frameDiagnostics()` reads.
    /// Disabled by default; disabling clears the retained snapshot.
    /// Dev-tools builds only.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "setFrameDiagnosticsEnabled")]
    pub fn set_frame_diagnostics_enabled(&mut self, enabled: bool) {
        self.runtime
            .orchestrator_mut()
            .set_frame_diagnostics_enabled(enabled);
    }

    /// Diagnostic probe address for the next non-idle paint attempt:
    /// the snapshot reports which planned segments contain it. Attempt-
    /// scoped, range-only, never read by the planner. Dev-tools only.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "setFrameDiagnosticsProbe")]
    pub fn set_frame_diagnostics_probe(&mut self, r1: i32, c1: i32, r2: i32, c2: i32) {
        self.runtime
            .orchestrator_mut()
            .set_frame_diagnostics_probe(iron_canvas_core::RCRange { r1, c1, r2, c2 });
    }

    /// Structured snapshot of the last completed live attempt.
    /// Returns `undefined` when capture is disabled or during playback;
    /// live callers use `frameTrace()` for the allocation-free one-line
    /// summary. Dev-tools builds only.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "frameDiagnostics")]
    pub fn frame_diagnostics(&self) -> JsValue {
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return JsValue::UNDEFINED;
        }
        match self.runtime.orchestrator().frame_diagnostics() {
            None => JsValue::UNDEFINED,
            Some(diag) => {
                let wire = crate::wire::FrameDiagnosticsWire::from(&diag);
                serde_wasm_bindgen::to_value(&wire).unwrap_or(JsValue::UNDEFINED)
            }
        }
    }

    /// Structured diagnostics for the currently displayed recorded attempt.
    /// Returns `undefined` outside playback; live callers should use
    /// `frameTrace()` for the allocation-free one-line core summary.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "recordingCurrentAttempt")]
    pub fn recording_current_attempt(&self) -> Result<JsValue, JsError> {
        let CanvasMode::Playback(session) = &self.mode else {
            return Ok(JsValue::UNDEFINED);
        };
        let Some(frame) = session.recording.frames.get(session.frame_idx as usize) else {
            return Ok(JsValue::UNDEFINED);
        };
        serde_wasm_bindgen::to_value(frame)
            .map_err(|e| JsError::new(&format!("recording attempt serialization failed: {e}")))
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
        let canvas = self.runtime.orchestrator().canvas_size();
        let theme_snap = ThemeSnapshot::from(self.runtime.orchestrator().theme());
        let now = js_sys::Date::now();
        let header = IcrHeader::new(
            canvas.w,
            canvas.h,
            self.runtime.dpr(),
            theme_snap,
            now as u64,
        );
        self.mode = CanvasMode::Recording(RecordingState {
            rec: Recording::new(header),
            started_at_ms: now,
            bytes_estimate: 0,
            soft_warn_fired: false,
            capped: false,
        });
        if filter.layers.includes_grid() {
            self.runtime
                .orchestrator()
                .grid_surface()
                .set_skip_groups(filter.skip_groups.clone());
            self.runtime
                .orchestrator()
                .grid_surface()
                .enable_recording();
        }
        if filter.layers.includes_overlay() {
            self.runtime
                .orchestrator()
                .overlay_surface()
                .set_skip_groups(filter.skip_groups);
            self.runtime
                .orchestrator()
                .overlay_surface()
                .enable_recording();
        }
        // Synchronously request the recording baseline. Relying on the
        // host's next rAF tick is unsafe — that tick might be a narrow
        // SlotsReuse (active-cell move, stable-layout content edit). The
        // attempt is marked as `forced_baseline`; if capture holds, it stays
        // in the diagnostic timeline and a later committed Fresh establishes
        // the replay anchor.
        self.runtime.orchestrator_mut().request_repaint();
        self.runtime.orchestrator().grid_surface().begin_frame();
        self.runtime.orchestrator().overlay_surface().begin_frame();
        let result = self.runtime.orchestrator_mut().paint_if_dirty();
        self.capture_frame(result, RecordOrigin::ForcedBaseline);
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
        self.runtime
            .orchestrator()
            .grid_surface()
            .disable_recording();
        self.runtime
            .orchestrator()
            .overlay_surface()
            .disable_recording();
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
        self.runtime.orchestrator_mut().set_model(erased);
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

    /// Fonts finished loading after paints already ran — memoized text
    /// widths may reflect a fallback font. Clear the Canvas2D measure
    /// memos and repaint. Host wiring (addEventListener, never the
    /// single-slot `onloadingdone =` property — multiple canvases share
    /// `document.fonts`):
    /// `document.fonts.addEventListener("loadingdone", () => canvas.fontsChanged());`
    #[wasm_bindgen(js_name = "fontsChanged")]
    pub fn fonts_changed(&mut self) {
        self.runtime.fonts_changed();
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
            self.runtime.orchestrator().theme(),
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
            self.runtime.orchestrator().theme(),
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
        self.runtime
            .orchestrator_mut()
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
        let wire: crate::wire::HitTestWire = self.runtime.orchestrator().hit_test(x, y).into();
        Ok(serde_wasm_bindgen::to_value(&wire)?)
    }

    /// Pixel rect of a 1-based cell, or `null` if the cell isn't laid out
    /// (e.g. off-screen rows that the slot map hasn't materialized).
    #[wasm_bindgen(js_name = "cellRect")]
    pub fn cell_rect_js(&self, row: i32, column: i32) -> Result<JsValue, JsError> {
        match self.runtime.orchestrator().cell_rect(row, column) {
            Some(rect) => Ok(serde_wasm_bindgen::to_value(&rect)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Resize-handle hit-test. `tolerance` is the slop band in CSS pixels.
    /// Returns `null` if no row/column trailing edge is within tolerance.
    #[wasm_bindgen(js_name = "resizeHandleAt")]
    pub fn resize_handle_at_js(&self, x: f64, y: f64, tolerance: f64) -> Result<JsValue, JsError> {
        match self
            .runtime
            .orchestrator()
            .resize_handle_at(x, y, tolerance)
        {
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
        match self.runtime.orchestrator().autofill_handle() {
            Some(p) => Ok(serde_wasm_bindgen::to_value(&p)?),
            None => Ok(JsValue::NULL),
        }
    }

    /// Layer-bypassing cell resolver. Returns `{row, column}` or `null`.
    /// Bypasses overlay layers (formula-ref dragging, etc.) that would
    /// otherwise claim the pointer.
    #[wasm_bindgen(js_name = "pixelToCell")]
    pub fn pixel_to_cell_js(&self, x: f64, y: f64) -> Result<JsValue, JsError> {
        match self.runtime.orchestrator().pixel_to_cell(x, y) {
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
        let wire: crate::wire::CanvasSizeWire = self.runtime.orchestrator().canvas_size().into();
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
        self.runtime.orchestrator_mut().request_overlay_repaint();
    }

    /// The view moved: scroll, selection, active cell, or sheet. Marks view
    /// plus overlay atomically — see `Orchestrator::view_changed`. Intent
    /// only: whether the movement shifts pixels, stays inside the painted
    /// frame, or needs a rebuild is the next `paintIfDirty`'s geometric
    /// verdict, not the caller's.
    #[wasm_bindgen(js_name = "viewChanged")]
    pub fn view_changed_js(&mut self) {
        self.runtime.orchestrator_mut().view_changed();
    }

    /// Autofill drag target. Pass `null` to clear (drag ended / cancelled).
    #[wasm_bindgen(js_name = "setExtendTo")]
    pub fn set_extend_to_js(&mut self, target: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::AutofillTargetWire> = serde_wasm_bindgen::from_value(target)?;
        self.runtime
            .orchestrator_mut()
            .set_extend_to(wire.map(Into::into));
        Ok(())
    }

    /// Clipboard marching-ants rectangle. Pass `null` to clear.
    #[wasm_bindgen(js_name = "setClipboard")]
    pub fn set_clipboard_js(&mut self, area: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::SheetAreaWire> = serde_wasm_bindgen::from_value(area)?;
        self.runtime
            .orchestrator_mut()
            .set_clipboard(wire.map(Into::into));
        Ok(())
    }

    /// Formula-entry point-mode range highlight. Pass `null` to clear.
    #[wasm_bindgen(js_name = "setPointRange")]
    pub fn set_point_range_js(&mut self, range: JsValue) -> Result<(), JsError> {
        let wire: Option<crate::wire::RCRangeWire> = serde_wasm_bindgen::from_value(range)?;
        self.runtime
            .orchestrator_mut()
            .set_point_range(wire.map(Into::into));
        Ok(())
    }

    /// Replace the per-formula draggable references.
    #[wasm_bindgen(js_name = "setFormulaRefs")]
    pub fn set_formula_refs_js(&mut self, refs: JsValue) -> Result<(), JsError> {
        let wire: Vec<crate::wire::FormulaRefWire> = serde_wasm_bindgen::from_value(refs)?;
        let refs: Vec<iron_canvas_core::FormulaRef> = wire.into_iter().map(Into::into).collect();
        self.runtime.orchestrator_mut().set_formula_refs(refs);
        Ok(())
    }

    /// Full overlay-state push. Currently infallible at the boundary; the
    /// `Result` is preserved so future invariant checks can surface as a
    /// `JsError` without touching the call site.
    #[wasm_bindgen(js_name = "setOverlays")]
    pub fn set_overlays_js(&mut self, overlays: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::RenderOverlaysWire = serde_wasm_bindgen::from_value(overlays)?;
        let engine = wire.into_engine().map_err(|msg| JsError::new(&msg))?;
        self.runtime.orchestrator_mut().set_overlays(engine);
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
        self.runtime.orchestrator_mut().set_theme(wire.into());
        self.restamp_recording_theme();
        Ok(())
    }

    /// Partial theme override. Each key is optional; missing keys fall back
    /// to `CanvasTheme::light()` via the engine's
    /// `From<ThemeVariables> for CanvasTheme` impl.
    #[wasm_bindgen(js_name = "setThemeVariables")]
    pub fn set_theme_variables_js(&mut self, vars: JsValue) -> Result<(), JsError> {
        let wire: crate::wire::ThemeVariablesWire = serde_wasm_bindgen::from_value(vars)?;
        self.runtime
            .orchestrator_mut()
            .set_theme_variables(wire.into());
        self.restamp_recording_theme();
        Ok(())
    }
}

// Rust-only API. Counterpart to the `#[wasm_bindgen]` block above —
// these methods take Rust types that don't cross the JS bridge.
impl IronCanvas {
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        self.runtime.orchestrator_mut().set_overlays(overlays);
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        self.runtime.orchestrator_mut().set_extend_to(target);
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        self.runtime.orchestrator_mut().set_clipboard(area);
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        self.runtime.orchestrator_mut().set_point_range(range);
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        self.runtime.orchestrator_mut().set_formula_refs(refs);
    }

    pub fn set_theme(&mut self, theme: CanvasTheme) {
        self.runtime.orchestrator_mut().set_theme(theme);
        self.restamp_recording_theme();
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.runtime.orchestrator_mut().set_theme_variables(vars);
        self.restamp_recording_theme();
    }

    /// Rust-level model push. Accepts any `CanvasModel` impl behind an
    /// `Rc` — Leptos-side adapters that bridge a host store to the canvas
    /// (e.g. `WorksheetModelAdapter`) route through here.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.model = Some(Rc::clone(&model));
        // A Rust-level model replaces any JS one; its theme is host-resolved.
        self.js_model = None;
        self.runtime.orchestrator_mut().set_model(model);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.runtime.orchestrator().canvas_size()
    }

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        self.runtime.orchestrator().hit_test(x, y)
    }

    /// Layer-bypassing cell resolver. Use during an active drag whose
    /// overlay (e.g. `FormulaRefsLayer`) would otherwise claim the
    /// pointer and starve the host of underlying cell coordinates.
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        self.runtime.orchestrator().pixel_to_cell(x, y)
    }

    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.runtime
            .orchestrator()
            .resize_handle_at(x, y, tolerance)
    }

    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.runtime.orchestrator().cell_rect(row, column)
    }

    pub fn scroll_pane_rect(&self) -> Option<PixelRect> {
        self.runtime.orchestrator().scroll_pane_rect()
    }

    pub fn legal_scroll_origin(&self) -> Option<(i32, i32)> {
        self.runtime.orchestrator().legal_scroll_origin()
    }

    pub fn scroll_to_show(&self, row: i32, column: i32) -> Option<(i32, i32)> {
        self.runtime.orchestrator().scroll_to_show(row, column)
    }

    pub fn fit_column_width(&self, col: i32, first_row: i32, last_row: i32) -> Option<f64> {
        self.runtime
            .orchestrator()
            .fit_column_width(col, first_row, last_row)
    }

    pub fn fit_row_height(&self, row: i32, first_col: i32, last_col: i32) -> Option<f64> {
        self.runtime
            .orchestrator()
            .fit_row_height(row, first_col, last_col)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.runtime.orchestrator().autofill_handle()
    }

    pub fn request_overlay_repaint(&mut self) {
        self.runtime.orchestrator_mut().request_overlay_repaint();
    }

    /// The view moved: scroll, selection, active cell, or sheet. Marks view
    /// plus overlay atomically — see `Orchestrator::view_changed`. Intent
    /// only: whether the movement shifts pixels, stays inside the painted
    /// frame, or needs a rebuild is the next `paint_if_dirty`'s geometric
    /// verdict, not the caller's.
    pub fn view_changed(&mut self) {
        self.runtime.orchestrator_mut().view_changed();
    }

    /// Drain the per-attempt op buffers, push a `Frame` for every non-idle
    /// result (including zero-op holds), update the running cap estimate, fire
    /// soft-warn / hard-cap
    /// side effects.
    ///
    /// Hard cap: flips `partial`, disables both painter-level forks, but
    /// keeps the `Recording` mode populated so `stopRecording` can still
    /// drain the partial bytes.
    #[cfg(feature = "dev-tools")]
    fn capture_frame(&mut self, result: PaintResult, origin: RecordOrigin) {
        let grid_ops = self.runtime.orchestrator().grid_surface().end_frame();
        let overlay_ops = self.runtime.orchestrator().overlay_surface().end_frame();
        if result == PaintResult::Idle {
            return;
        }
        let CanvasMode::Recording(state) = &mut self.mode else {
            return;
        };
        let now = js_sys::Date::now();
        let t_ms = (now - state.started_at_ms).max(0.0) as u64;
        let frame_idx = state.rec.frames.len() as u32;
        let frame_bytes: usize = grid_ops.iter().map(op_bytes).sum::<usize>()
            + overlay_ops.iter().map(op_bytes).sum::<usize>()
            + ATTEMPT_METADATA_BYTES_HEURISTIC;
        state.rec.push_frame(Frame {
            frame_idx,
            t_ms,
            origin,
            result: match result {
                PaintResult::Painted => RecordedPaintResult::Painted,
                PaintResult::Retry => RecordedPaintResult::Retry,
                PaintResult::Idle => unreachable!("idle attempts are omitted above"),
            },
            trace: TraceRecord::from(self.runtime.orchestrator().last_trace()),
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
            self.runtime
                .orchestrator()
                .grid_surface()
                .disable_recording();
            self.runtime
                .orchestrator()
                .overlay_surface()
                .disable_recording();
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
            state.rec.header.theme = ThemeSnapshot::from(self.runtime.orchestrator().theme());
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
        let live_size = self.runtime.orchestrator().canvas_size();
        let live_dpr = self.runtime.dpr();
        let rec_size = CanvasSize {
            w: rec.header.canvas_w,
            h: rec.header.canvas_h,
        };
        let rec_dpr = rec.header.dpr;

        // Resize the orchestrator + canvas backing stores to recording
        // dims. Inline CSS on the canvases is overridden separately —
        // the `.ws-canvas` class pins display size to `100%`, which would
        // otherwise scale the backing store back down to the container.
        self.runtime.resize(rec_size, rec_dpr);
        set_canvas_css_size(self.runtime.grid_canvas(), rec_size)?;
        set_canvas_css_size(self.runtime.overlay_canvas(), rec_size)?;

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
        let _ = clear_canvas_css_size(self.runtime.grid_canvas());
        let _ = clear_canvas_css_size(self.runtime.overlay_canvas());

        self.runtime.resize(session.live_size, session.live_dpr);
        // Kept: playback bypasses last_frame, so resize's self-invalidation alone can't cover this.
        self.runtime.orchestrator_mut().request_repaint();
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

        let grid = self.runtime.orchestrator().grid_surface().painter();
        let overlay = self.runtime.orchestrator().overlay_surface().painter();
        let present_grid = || self.runtime.orchestrator().grid_surface().present();
        replay_through(grid, overlay, &session.recording, clamped, &present_grid);
        Ok(())
    }
}
