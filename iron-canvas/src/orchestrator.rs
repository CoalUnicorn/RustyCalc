use std::rc::Rc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::chrome::{BlitPlan, Chrome, FramePath, FrameValidity, PaneRegionMask};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::CanvasSize;
use crate::layer::{GridLayer, OverlayLayer, RenderOverlays};
use crate::signal::GridSignals;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::ui::{HitTest, ResizeTarget};
use crate::wasm::JsBackedModel;
use crate::CanvasModel;

/// Named verdict of `paintIfDirty`'s dispatch. One arm per concrete paint
/// path; the exhaustive `match` over `decide()` replaces the old
/// `try_paint_overlay_only → try_paint_blit → paint_grid` fallthrough chain.
#[allow(dead_code)] // Structural's inner StructuralReason isn't read yet; future PRs refine.
pub(crate) enum PaintRegime {
    Overlay,
    Viewport(BlitPlan),
    Content(PaneRegionMask),
    Structural(StructuralReason),
}

/// Why `decide()` chose `Structural` — a full or SlotsReuse rebuild. The
/// variants are inert in PR 1+2 (decide always returns `Unknown`); later
/// PRs refine `is_still_valid` to distinguish causes.
#[allow(dead_code)] // Variants beyond `Unknown` activate when is_still_valid is refined.
pub(crate) enum StructuralReason {
    CanvasResize,
    SheetChange,
    FreezeChange,
    ThemeChange,
    FirstPaint,
    Unknown,
}

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
    let primary = match plan.shifts.first() {
        Some(s) => s,
        None => return,
    };
    let msg = format!(
        "[blit] axis={} prev_top={} new_top={} new_last={} cache.range={} scroll_rows.len()={} shifts={} src=(y={}, h={}) dst=(y={}, h={}) strip=(y={}, h={})",
        axis,
        prev_top,
        new_top,
        new_last,
        cached,
        scroll_rows_len,
        plan.shifts.len(),
        primary.src.top_left.y,
        primary.src.height,
        primary.dst.top_left.y,
        primary.dst.height,
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
    /// Typed cell-content-changed signal accumulated since the last paint.
    /// `Some(mask)` means the named panes' cached buffers are stale and
    /// must refetch from the model; `decide()` routes it to the `Content`
    /// arm when the viewport is otherwise SlotsReuse-valid. Cleared at
    /// the end of every `paintIfDirty`.
    pending_content: Option<PaneRegionMask>,
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
            pending_content: None,
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

    /// Conservative repaint blanket. JS callers that don't know which
    /// signal class fits (scroll, selection, theme bridge) land here;
    /// callers that DO know should use the typed setters
    /// (`markContentDirty`, `request_overlay_repaint`, `set_theme*`).
    ///
    /// Raises `STRUCTURAL | OVERLAY` — explicitly *not* `CONTENT`. The
    /// `Content` regime is reserved for real cell-value changes routed
    /// through `markContentDirty`; raising it speculatively here would
    /// veto the blit arm on every scroll (the blit arm's CONTENT-veto
    /// exists to prevent stale-pixel propagation, and a blanket
    /// `requestRepaint` cannot prove content is unchanged either way,
    /// so we let geometric `try_blit` decide and fall back to a
    /// `Structural` rebuild when it can't).
    #[allow(non_snake_case)]
    pub fn requestRepaint(&mut self) {
        self.grid
            .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
        self.overlay.raise(GridSignals::OVERLAY);
    }

    /// JS-facing cell-content-changed signal. Use after `Model.set_user_input`
    /// (or any other path that mutates a visible cell's value) so the next
    /// paint refetches values for all visible panes instead of fingerprint-
    /// skipping on stale cached buffers. Marks all four pane quadrants —
    /// the conservative default; pane-granular masks stay Rust-internal.
    #[allow(non_snake_case)]
    pub fn markContentDirty(&mut self) {
        self.mark_content_dirty(PaneRegionMask::ALL);
    }

    /// Paint whichever layers are dirty. Clean layers are skipped; see
    /// `ARCHITECTURE.md` for the cache rules and the overlay-only path.
    ///
    /// Dispatches via `decide()` into one of four named regimes:
    /// `Overlay` (cached frame, overlay-only), `Viewport` (scroll-blit,
    /// content-clean), `Content` (cell-change without viewport shift),
    /// `Structural` (full or slots-reuse rebuild). The `match` is
    /// exhaustive — adding a regime breaks the build here, by design.
    #[allow(non_snake_case)]
    pub fn paintIfDirty(&mut self) {
        let grid_signals = self.grid.drain_signals();
        let overlay_signals = self.overlay.drain_signals();
        let signals = grid_signals | overlay_signals;
        if signals.is_empty() {
            return;
        }
        let Some(model_rc) = self.model.clone() else {
            return;
        };
        let model: &dyn CanvasModel = model_rc.as_ref();

        let overlay_dirty = signals.overlay_dirty();
        let content_dirty = signals.contains(GridSignals::CONTENT);

        match self.decide(signals, model) {
            PaintRegime::Overlay => self.paint_overlay(model),
            PaintRegime::Viewport(plan) => self.paint_viewport(model, plan),
            PaintRegime::Content(mask) => self.paint_content(model, mask, overlay_dirty),
            PaintRegime::Structural(_) => self.paint_rebuild(model, overlay_dirty, content_dirty),
        }
        self.pending_content = None;
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
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    /// Rust-level theme push. The wasm surface keeps `set_theme_name` to
    /// preserve the JS handle. Value-compares against `self.theme` and,
    /// on change, marks both layers dirty.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != self.theme {
            self.theme = theme;
            self.grid.raise(GridSignals::STRUCTURAL);
            self.overlay.raise(GridSignals::STRUCTURAL);
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
            self.grid.raise(GridSignals::STRUCTURAL);
            self.overlay.raise(GridSignals::STRUCTURAL);
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
        self.overlay.raise(GridSignals::OVERLAY);
    }

    /// Typed cell-content-changed signal. Marks the named panes' cached
    /// buffers stale so the next `paintIfDirty` refetches their values
    /// from the model via the `Content` regime arm — fixes the recalc
    /// bug where a formula dependent on an edited cell silently kept
    /// painting the stale cached value (commit `25d91d2`).
    ///
    /// Repeated calls accumulate (bitwise OR) until paint consumes them.
    /// Also marks the grid dirty so the next `paintIfDirty` runs even
    /// without a separate `requestRepaint`.
    pub(crate) fn mark_content_dirty(&mut self, mask: PaneRegionMask) {
        self.pending_content = Some(match self.pending_content {
            Some(prev) => prev.union(mask),
            None => mask,
        });
        self.grid.raise(GridSignals::CONTENT);
    }

    /// Classify which paint regime to run for the current state. Pure over
    /// `&self`; arm methods own the mutation. `grid_dirty` is consumed by
    /// the caller before we get here, so `decide` takes it as a parameter
    /// rather than re-reading the gate (which is take-and-clear).
    ///
    /// PR 1+2 contract: `Content` is unreachable until `mark_content_dirty`
    /// is wired in PR 4; `Structural` always reports `Unknown` until
    /// `is_still_valid` is refined to distinguish causes.
    fn decide(&self, sig: GridSignals, model: &dyn CanvasModel) -> PaintRegime {
        let validity = self
            .last_frame
            .as_ref()
            .map_or(FrameValidity::Rebuild, |f| {
                f.is_still_valid(model, self.size)
            });
        let pending_content = self.pending_content;

        if !sig.grid_dirty()
            && sig.overlay_dirty()
            && matches!(validity, FrameValidity::SlotsReuse)
            && self.last_frame.is_some()
        {
            return PaintRegime::Overlay;
        }

        // Blit detection is geometric: `try_blit` diffs `last_frame`'s
        // scroll/freeze/sheet/size against the model and returns a plan
        // only on a real viewport shift. We gate on CONTENT (a blit on
        // stale content propagates wrong pixels — the recalc bug) but
        // not on a typed VIEWPORT signal: no JS-facing setter raises
        // VIEWPORT today, so requiring it would dead-code this arm.
        if !sig.contains(GridSignals::CONTENT) && self.grid.painter_supports_blit() {
            if let Some(plan) = self
                .last_frame
                .as_ref()
                .and_then(|f| f.try_blit(model, self.size, &self.theme))
            {
                return PaintRegime::Viewport(plan);
            }
        }

        if sig.contains(GridSignals::CONTENT)
            && matches!(validity, FrameValidity::SlotsReuse)
            && self.last_frame.is_some()
        {
            let mask = pending_content.unwrap_or(PaneRegionMask::ALL);
            return PaintRegime::Content(mask);
        }

        PaintRegime::Structural(StructuralReason::Unknown)
    }

    /// Overlay-only fast path. Triggered by autofill drag, clipboard state
    /// change, formula-ref highlight updates, and active-cell moves —
    /// anything that leaves grid pixels untouched. `decide()` proves the
    /// preconditions (slot vecs still match, `last_frame` is `Some`).
    fn paint_overlay(&mut self, model: &dyn CanvasModel) {
        let Some(prev) = self.last_frame.as_mut() else {
            return;
        };
        prev.refresh_overlay_inputs(model);
        self.overlay.paint(&self.overlays, model, prev);
    }

    /// Scroll-blit fast path. `decide()` already filtered no-op scrolls,
    /// backends without `supports_blit`, and viewport shifts where the
    /// kept band can't be reused; we trust the verdict and the supplied
    /// plan. Always repaints the overlay too — a viewport shift moves
    /// every overlay primitive's pixel position.
    fn paint_viewport(&mut self, model: &dyn CanvasModel, plan: BlitPlan) {
        let Some(prev) = self.last_frame.take() else {
            return;
        };
        #[cfg(target_arch = "wasm32")]
        let prev_top = prev.pane_set_top_row_debug();
        #[cfg(target_arch = "wasm32")]
        let prev_pane_range = self.grid.bottom_right_cache_range_debug();
        let frame = Chrome::next(
            Some(prev),
            model,
            self.size,
            &self.theme,
            FramePath::Blit(plan.clone()),
        );
        #[cfg(target_arch = "wasm32")]
        log_blit_plan(&plan, prev_top, prev_pane_range, &frame);
        self.grid.paint_blit(model, &frame, &plan);
        self.overlay.paint(&self.overlays, model, &frame);
        self.last_frame = Some(frame);
    }

    /// Content-changed-but-viewport-unchanged regime. Slot vecs still
    /// match the live model (so we reuse them via `from_slots_reuse`)
    /// but cell values may have shifted, so we drop the `PaneCache`
    /// entries for the masked panes and let `render_pane` refetch.
    /// Unmasked panes fingerprint-skip cleanly.
    fn paint_content(
        &mut self,
        model: &dyn CanvasModel,
        mask: PaneRegionMask,
        overlay_dirty: bool,
    ) {
        let Some(prev) = self.last_frame.take() else {
            // No prior frame: the Content contract (viewport unchanged) is
            // meaningless. Fall back to a structural rebuild; content is
            // dirty by construction here.
            self.paint_rebuild(model, overlay_dirty, true);
            return;
        };
        let mut prev = Chrome::next(
            Some(prev),
            model,
            self.size,
            &self.theme,
            FramePath::SlotsReuse,
        );
        prev.refresh_overlay_inputs(model);

        self.grid.invalidate_pane_cache(mask);
        self.grid.invalidate_paint_cache();

        self.grid.paint(model, &prev, None);
        if overlay_dirty {
            self.overlay.paint(&self.overlays, model, &prev);
        }
        self.last_frame = Some(prev);
    }

    /// Full grid repaint. Two `Chrome` sources:
    ///   • `SlotsReuse` — keep prev's slot vecs, promote its painted
    ///     `pane_fingerprints` into `prev_pane_fingerprints`, refresh
    ///     theme + overlay inputs. Cheap; `render_pane` still
    ///     fingerprint-skips per pane.
    ///   • Rebuild — full `next_frame` walk. The new slot vecs make any
    ///     fingerprint compare across the boundary meaningless, so every
    ///     pane repaints.
    fn paint_rebuild(&mut self, model: &dyn CanvasModel, overlay_dirty: bool, content_dirty: bool) {
        let validity = self
            .last_frame
            .as_ref()
            .map_or(FrameValidity::Rebuild, |f| {
                f.is_still_valid(model, self.size)
            });

        let frame = match (validity, self.last_frame.take()) {
            (FrameValidity::SlotsReuse, Some(prev)) => {
                let mut frame = Chrome::next(
                    Some(prev),
                    model,
                    self.size,
                    &self.theme,
                    FramePath::SlotsReuse,
                );
                frame.refresh_overlay_inputs(model);
                frame
            }
            (_, prev) => Chrome::next(prev, model, self.size, &self.theme, FramePath::Fresh),
        };

        // SlotsReuse: theme-only changes leave fingerprints intact; drop
        // the buffers so the next render_pane refetches. CONTENT here
        // means a content-edit escalated to Rebuild (e.g. via scroll); we
        // still drop buffers so render_pane refetches against the new
        // slot vecs instead of trusting a range-matched-but-stale cache.
        if content_dirty || matches!(validity, FrameValidity::SlotsReuse) {
            self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
        }

        self.grid.invalidate_paint_cache();
        self.grid.paint(model, &frame, None);
        if overlay_dirty {
            self.overlay.paint(&self.overlays, model, &frame);
        }

        self.last_frame = Some(frame);
    }
}
