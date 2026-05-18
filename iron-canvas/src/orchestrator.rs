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

/// Named verdict of `paintIfDirty`'s dispatch. Each variant aligns 1:1
/// with a `paint_*` method; the regime carries everything that method
/// needs (mask, dirty bits) so `paintIfDirty` is pure pattern-destructure.
/// Variants align with `FramePath`: `SlotsReuse` and `Fresh` here map to
/// `FramePath::SlotsReuse` and `FramePath::Fresh` inside `Chrome::next`.
pub(crate) enum PaintRegime {
    Overlay,
    Viewport(BlitPlan),
    SlotsReuse {
        mask: PaneRegionMask,
        signals: GridSignals,
    },
    Fresh(GridSignals),
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
    /// Bits name the panes whose cached buffers are stale and must refetch.
    /// `decide()` routes a non-empty mask through the SlotsReuse arm when
    /// the viewport is otherwise reusable. Reset to `EMPTY` at the end of
    /// every `paintIfDirty`.
    pending_content: PaneRegionMask,
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
            pending_content: PaneRegionMask::EMPTY,
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
    /// (`markContentDirty`, `request_overlay_repaint`, `set_theme*`,
    /// `setModel`).
    ///
    /// Drops `last_frame` so the next `paintIfDirty` falls to `Fresh` —
    /// the cheaper `SlotsReuse` / `Viewport` arms gate on
    /// `last_frame.is_some()`. This covers workbook swaps that mutate
    /// the model in place (row heights / column widths read at slot-vec
    /// build time, not cached anywhere `STRUCTURAL` alone would
    /// invalidate). JS callers that want the cheap structural rebuild
    /// have typed setters; `requestRepaint` is the worst-case fallback.
    ///
    /// Raises `STRUCTURAL | OVERLAY` — explicitly *not* `CONTENT`. The
    /// `CONTENT` bit is reserved for real cell-value changes routed
    /// through `markContentDirty`; raising it here would force
    /// `PaneCache::ALL` invalidation in `paint_fresh` and a blanket
    /// `requestRepaint` cannot prove the cache is stale either way.
    #[allow(non_snake_case)]
    pub fn requestRepaint(&mut self) {
        self.last_frame = None;
        self.pending_content = PaneRegionMask::EMPTY;
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
    /// content-clean), `SlotsReuse` (viewport stable; mask-scoped pane
    /// refetch or full refresh on theme change), `Fresh` (full rebuild).
    /// The `match` is exhaustive — adding a regime breaks the build
    /// here, by design.
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

        match self.decide(signals, model) {
            PaintRegime::Overlay => self.paint_overlay(model),
            PaintRegime::Viewport(plan) => self.paint_viewport(model, plan),
            PaintRegime::SlotsReuse { mask, signals } => {
                self.paint_slots_reuse(model, mask, signals)
            }
            PaintRegime::Fresh(signals) => self.paint_fresh(model, signals),
        }
        self.pending_content = PaneRegionMask::EMPTY;
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
            // `is_still_valid` doesn't see model identity, so a workbook
            // swap with the same scroll/sheet/freeze/size would otherwise
            // reuse the prev pane_set (stale row heights / column widths).
            self.last_frame = None;
            self.pending_content = PaneRegionMask::EMPTY;
            self.grid
                .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
            self.overlay.raise(GridSignals::OVERLAY);
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
    /// from the model via the `SlotsReuse` arm (mask = these panes) —
    /// fixes the recalc bug where a formula dependent on an edited
    /// cell silently kept painting the stale cached value (commit
    /// `25d91d2`).
    ///
    /// Repeated calls accumulate (bitwise OR) until paint consumes them.
    /// Also marks the grid dirty so the next `paintIfDirty` runs even
    /// without a separate `requestRepaint`.
    pub(crate) fn mark_content_dirty(&mut self, mask: PaneRegionMask) {
        self.pending_content |= mask;
        self.grid.raise(GridSignals::CONTENT);
    }

    /// Classify which paint regime to run for the current state. Pure over
    /// `&self`; arm methods own the mutation. The signal bits are consumed
    /// by the caller via `drain_signals`, so we take them as a parameter
    /// rather than re-reading the take-and-clear gate.
    fn decide(&self, sig: GridSignals, model: &dyn CanvasModel) -> PaintRegime {
        let content_dirty = sig.contains(GridSignals::CONTENT);
        let validity = self
            .last_frame
            .as_ref()
            .map_or(FrameValidity::Rebuild, |f| {
                f.is_still_valid(model, self.size)
            });

        if !sig.grid_dirty()
            && sig.overlay_dirty()
            && matches!(validity, FrameValidity::SlotsReuse)
            && self.last_frame.is_some()
        {
            return PaintRegime::Overlay;
        }

        // Blit detection is geometric: `screen_for_blit` diffs `last_frame`'s
        // scroll/freeze/sheet/size against the model and returns a plan
        // only on a real viewport shift. Gated on CONTENT (a blit on
        // stale content propagates wrong pixels — the recalc bug) but
        // not on a typed VIEWPORT signal: no JS-facing setter raises
        // VIEWPORT today, so requiring it would dead-code this arm.
        // `GridLayer` is concretely `CanvasPainter` (a `BlitPainter`), so
        // the capability is a compile-time fact here, not a runtime check.
        if !content_dirty {
            if let Some(plan) = self
                .last_frame
                .as_ref()
                .and_then(|f| f.screen_for_blit(model, self.size, &self.theme))
            {
                return PaintRegime::Viewport(plan);
            }
        }

        // When content is dirty we honour the consumer-supplied pending
        // mask; otherwise refresh all panes (theme touches every pane
        // uniformly).
        if matches!(validity, FrameValidity::SlotsReuse) && self.last_frame.is_some() {
            let mask = if content_dirty && !self.pending_content.is_empty() {
                self.pending_content
            } else {
                PaneRegionMask::ALL
            };
            return PaintRegime::SlotsReuse { mask, signals: sig };
        }

        PaintRegime::Fresh(sig)
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

    /// Scroll-blit fast path. `decide()` already filtered no-op scrolls
    /// and viewport shifts where the kept band can't be reused; we trust
    /// the verdict and the supplied plan. Always repaints the overlay
    /// too — a viewport shift moves every overlay primitive's pixel
    /// position.
    fn paint_viewport(&mut self, model: &dyn CanvasModel, plan: BlitPlan) {
        let Some(prev) = self.last_frame.take() else {
            return;
        };
        let frame = Chrome::next(
            Some(prev),
            model,
            self.size,
            &self.theme,
            FramePath::Blit(&plan),
        );
        self.grid.paint_blit(model, &frame, &plan);
        self.overlay.paint(&self.overlays, model, &frame);
        self.last_frame = Some(frame);
    }

    /// SlotsReuse regime: prev's slot vecs survive (viewport unchanged),
    /// only `pane_cache` entries inside `mask` are invalidated so
    /// `render_pane` refetches there. Unmasked panes fingerprint-skip.
    /// `decide()` guarantees `last_frame` is `Some` before selecting
    /// this regime.
    fn paint_slots_reuse(
        &mut self,
        model: &dyn CanvasModel,
        mask: PaneRegionMask,
        signals: GridSignals,
    ) {
        let Some(prev) = self.last_frame.take() else {
            return;
        };
        let mut frame = Chrome::next(
            Some(prev),
            model,
            self.size,
            &self.theme,
            FramePath::SlotsReuse,
        );
        frame.refresh_overlay_inputs(model);

        self.grid.invalidate_pane_cache(mask);
        self.grid.invalidate_paint_cache();

        self.grid.paint(model, &frame);
        if signals.overlay_dirty() {
            self.overlay.paint(&self.overlays, model, &frame);
        }
        self.last_frame = Some(frame);
    }

    /// Full grid repaint. Slot vecs are walked fresh from the model; the
    /// new vecs make any cross-frame fingerprint compare meaningless, so
    /// every pane repaints. Selected when `decide()` finds slot vecs have
    /// diverged (scroll/freeze/sheet/canvas size change) or no prior
    /// frame exists. The reusable-slot pathway lives in `paint_slots_reuse`.
    ///
    /// The `CONTENT` bit gates `PaneCache` invalidation: a content edit
    /// escalated to Fresh (e.g. via concurrent scroll) means the cache's
    /// range-matched buffers may now be stale against the new slot vecs.
    fn paint_fresh(&mut self, model: &dyn CanvasModel, signals: GridSignals) {
        let prev = self.last_frame.take();
        let frame = Chrome::next(prev, model, self.size, &self.theme, FramePath::Fresh);

        if signals.contains(GridSignals::CONTENT) {
            self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
        }
        self.grid.invalidate_paint_cache();
        self.grid.paint(model, &frame);
        if signals.overlay_dirty() {
            self.overlay.paint(&self.overlays, model, &frame);
        }

        self.last_frame = Some(frame);
    }
}
