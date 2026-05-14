//! Renderer core for the spreadsheet grid.
//!
//! # Lifecycle
//!
//! Two stacked `<canvas>` elements are wrapped by [`crate::IronCanvas`];
//! each canvas owns a `LayerBase` (canvas + `PaintGate` + a layer renderer
//! wrapping `RendererCore`). `GridLayer` builds its 2D context with
//! `alpha: false` (opaque, skips alpha compositing); `OverlayLayer` uses
//! `alpha: true, desynchronized: true`. The renderer is **long-lived per
//! layer**, so the painter's cached fill/stroke/font/line-width state
//! persists across frames.
//!
//! State pushes from JS mark layers dirty; `IronCanvas::paintIfDirty`
//! drives each dirty layer's `paint`, which calls into [`RendererCore::render_grid`]
//! / [`RendererCore::render_overlays`].
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `paintIfDirty` per dirty layer:
//!
//! - [`RendererCore::render_grid`] — cells (4 frozen-pane quadrants, each
//!   running 4 cell sub-passes: bg -> grid borders -> explicit borders -> text),
//!   frozen separators, headers, corner box.
//! - [`RendererCore::render_overlays`] — selection rectangle + autofill handle,
//!   header highlights, extend preview, clipboard marching ants, point-mode
//!   range, formula-ref highlights.
//!
//! The cell sub-pass order matters: grid borders run across the whole pane
//! before explicit borders so an explicit `right` on cell A wins over cell B's
//! grid `left` at the shared pixel column. Text runs last so overflow is never
//! clipped by a neighbour's bg.
//!
//! # Frozen panes
//!
//! The grid splits into up to four quadrants (top_left, top_right,
//! bottom_left, bottom_right) based on frozen rows + columns. Each
//! quadrant is rendered by `render_pane()` against a different
//! `PaneRegion`; a thick separator line marks the freeze boundary. See
//! the diagram in `ARCHITECTURE.md` for the layout.

pub(crate) mod cache;
pub(crate) mod cell;
pub(crate) mod chrome;
pub(crate) mod overlay;

use std::cell::{Cell, RefCell};
use web_sys::CanvasRenderingContext2d;

use crate::chrome::{BlitPlan, Chrome};
pub(crate) use crate::chrome::PaneRegion;
use crate::geometry::prim::Axis;
use crate::layer::RenderOverlays;
use crate::painter::CanvasPainter;
use crate::renderer::cache::{FrameCache, PaneCache};
use crate::CanvasModel;
pub(crate) use cache::ColNameIntern;
pub(crate) use cache::ColorIntern;
pub(crate) use cache::FontIntern;

#[cfg(test)]
pub(crate) use self::cell::text::{layout_into, TextLine};

use crate::painter::Painter;

/// Shared renderer core. Holds the painter `P`, dpr, the per-frame
/// `FrameCache`, and the renderer-lifetime intern tables (font, column
/// labels, per-cell color overrides). The two layer wrappers
/// (`GridRenderer`, `OverlayRenderer`) each own a `RendererCore` and
/// re-export only the entry point that belongs to their layer:
/// `render_grid` for the grid, `render_overlays` for the overlay. A grid
/// layer cannot call `render_overlays` and vice versa.
pub(crate) struct RendererCore<P: Painter> {
    pub(crate) painter: P,
    dpr: i32,
    pub(crate) frame_cache: FrameCache,
    /// Renderer-lifetime per-pane bulk-fetch buffers + last-fetched range.
    /// Sibling of the intern tables below; survives across frames so
    /// `render_pane` can short-circuit when a pane's address didn't
    /// change (Stage 3.2) or strip-fetch the new band (Stage 3.3).
    pub(crate) pane_cache: PaneCache,
    /// Renderer-lifetime intern table for `ctx.font` strings. Lives outside
    /// `FrameCache` because identical fonts repeat across frames, not just
    /// within a single paint.
    pub(crate) font_intern: FontIntern,
    /// Renderer-lifetime intern of column-letter labels. Same rationale as
    /// `font_intern` — column names repeat every frame; cache once, clone the
    /// `Rc<str>` thereafter.
    pub(crate) col_intern: ColNameIntern,
    /// Renderer-lifetime intern of per-cell color overrides (border + text).
    /// Hot-path callers (`BorderPaint::resolve`, `CellTextStyle::resolve`)
    /// previously allocated a fresh `String` per cell per frame; the intern
    /// makes those calls `Rc::clone` after the first sighting of each color.
    pub(crate) color_intern: ColorIntern,
}

impl<P: Painter> RendererCore<P> {
    pub(crate) fn painter(&self) -> &P {
        &self.painter
    }
}

impl<P: Painter> RendererCore<P> {
    /// Wipe the per-frame paint state and restore the sticky text defaults
    /// the renderer assumes at every entry point. Routed through the
    /// `Painter` trait so any backend (Canvas-2D today, Recorder/SVG later)
    /// gets the same reset semantics.
    pub(crate) fn invalidate_paint_cache(&mut self) {
        self.painter.invalidate_cache();
        self.painter.reset_text_defaults();
    }

    /// React to a backing-store resize: push the new DPR through the
    /// painter's transform, store it for snap math, and clear caches.
    pub(crate) fn resize_for_dpr(&mut self, dpr: i32) {
        self.painter.apply_dpr_transform(dpr);
        self.dpr = dpr;
        self.invalidate_paint_cache();
    }

    /// Layer-friendly constructor: caller owns canvas sizing + DPR scaling.
    /// Canvas size and theme both live on the per-frame `Chrome`,
    /// not on the renderer.
    pub(crate) fn for_layer(painter: P) -> Self {
        Self {
            painter,
            dpr: 1,
            frame_cache: FrameCache {
                text_slots: Cell::new(Vec::new()),
                show_grid: Cell::new(true),
                label_buf: RefCell::new(String::new()),
                text_lines: Cell::new(Vec::new()),
                wrap_buf: RefCell::new(String::new()),
            },
            pane_cache: PaneCache::default(),
            font_intern: FontIntern::new(),
            col_intern: ColNameIntern::new(),
            color_intern: ColorIntern::new(),
        }
    }

    /// Paint the grid layer: cells (per quadrant), frozen separators,
    /// headers, corner box. Does **not** clear the canvas — caller owns
    /// the clear so layer-owned renderers can paint a background fill
    /// instead.
    pub(crate) fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) {
        self.painter.begin_group("grid");
        // Cache the per-sheet grid-line toggle once for this frame so the
        // hot per-cell `paint_borders_grid` walk doesn't re-enter the model.
        // Falls back to "show" on model failure, matching Excel's default-on.
        let sheet = model.get_selected_sheet();
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));

        // `frame.stale_panes` is `ALL` after `Chrome::next_frame`; Stage 3.3
        // narrows it on the blit fast-path so only the genuinely-stale
        // quadrants run their 4-pass walk.
        for pane in frame.stale_panes.iter() {
            self.render_pane(model, pane, frame);
        }

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.draw_frozen_separators(frame);

        self.render_headers_base(Axis::Row, frame);
        self.render_headers_base(Axis::Column, frame);

        self.draw_corner_box(frame);
        self.painter.end_group();
    }

    /// Like `render_grid`, but assumes the BottomRight kept band was just
    /// preserved by a `Painter::blit` shift and only the
    /// `plan.repaint_strip` region needs new pixels in that pane. Caller
    /// (orchestrator) must set `frame.slots_reused = true` so render_pane's
    /// fingerprint-mismatch branch fills the pane bg — the clip then
    /// restricts that fill to the strip alone, leaving the blitted kept
    /// band intact.
    ///
    /// Cross-axis panes (frozen-rows or frozen-cols quadrants whose RCRange
    /// didn't change) fingerprint-match and skip cleanly under the same
    /// `slots_reused` path. The two cross-pane fills that don't skip
    /// (BottomLeft on a Row scroll, TopRight on a Column scroll) repaint
    /// fully — Stage 3's strip-fetch will narrow them.
    /// Walk each pane whose cached buffer data shifts along `plan.axis`
    /// and rotate the kept-band entries in place so only the freshly-
    /// revealed strip slots remain `None`. `render_pane` then fetches
    /// just the strip instead of the full pane.
    ///
    /// Defensive: if `pane_buf.range` is stale enough that the dimensions
    /// no longer line up (orthogonal axis differs, or the extent on the
    /// scroll axis changed), drop the cached range so render_pane falls
    /// through to a full fetch. This can happen when the previous paint
    /// of that pane was before a canvas resize.
    ///
    /// Always runs at the top of `render_grid_blit` — the caller's
    /// painter `blit` must come *before* this so the kept pixels are in
    /// their new position; this rotates the cached cell data to match.
    fn prepare_blit_cache(&self, frame: &Chrome, plan: &BlitPlan) {
        for pane in plan.shift_panes().iter() {
            let pane_buf = self.pane_cache.pane(pane);
            let Some(new_range) = pane.range(frame) else {
                pane_buf.range.set(None);
                continue;
            };
            let _ = pane_buf.try_shift(new_range, plan.axis);
        }
    }

    pub(crate) fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) {
        self.prepare_blit_cache(frame, plan);
        self.painter.begin_group("grid");
        let sheet = model.get_selected_sheet();
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));

        // Iterate `stale_panes` in render order. The strip clip wraps
        // BottomRight when (and only when) it's in the mask — Stage 3.3
        // may drop BottomRight from the mask if the blit + strip-fetch
        // proves the kept band's painted pixels are already correct.
        for pane in frame.stale_panes.iter() {
            if matches!(pane, PaneRegion::BottomRight) {
                self.painter.push_clip(plan.repaint_strip);
                self.render_pane(model, pane, frame);
                self.painter.pop_clip();
            } else {
                self.render_pane(model, pane, frame);
            }
        }

        self.draw_frozen_separators(frame);
        // Headers shift along the scroll axis only: row labels move with
        // vertical scroll (Axis::Row), col letters with horizontal scroll
        // (Axis::Column). The cross-axis strip's pixels are unchanged.
        self.render_headers_base(plan.axis, frame);
        self.draw_corner_box(frame);
        self.painter.end_group();
    }

    /// Paint the overlay layer: selection outline + autofill handle, header
    /// highlights, extend preview, clipboard marching ants, point-mode range,
    /// formula-ref highlights. Does **not** clear the canvas — caller owns
    /// the clear (overlay layer needs transparent bg).
    pub(crate) fn render_overlays(
        &mut self,
        model: &dyn CanvasModel,
        overlays: &RenderOverlays,
        frame: &Chrome,
    ) {
        self.painter.begin_group("overlay");
        self.draw_selection(model, frame);
        // Header highlights live on the overlay so nav events skip the grid repaint.
        self.render_header_highlights(Axis::Row, frame);
        self.render_header_highlights(Axis::Column, frame);
        if let Some(target) = overlays.extend_to {
            self.draw_extend_preview(model, frame, target);
        }

        // Secondary overlays: clipboard marching ants, point-mode range,
        // formula-ref highlights. Each no-ops if its data is absent or lives
        // on another sheet.
        self.draw_clipboard_overlay(model, frame, overlays.clipboard.as_ref());
        self.draw_point_overlay(frame, overlays.point_range);

        if !overlays.formula_refs.is_empty() {
            self.draw_formula_ref_overlays(model, frame, &overlays.formula_refs);
        };
        self.painter.end_group();
    }
}

// Layer-facing wrappers
//
// `GridRenderer` and `OverlayRenderer` each own a `RendererCore` and re-export
// only the operations their layer is allowed to perform. `LayerOps` is the
// paint-backend-agnostic subset (just `resize_for_dpr`); the Canvas-2D
// passthroughs (`ctx_ref` for the layer's own clear/fill, `invalidate_paint_cache`)
// live as inherent methods on the `<CanvasPainter>` impl so a future SvgPainter
// can satisfy `LayerOps` without `web_sys`.

/// Backend-agnostic resize hook. Called by `LayerBase::resize` whenever the
/// backing store's DPR changes; everything else stays on the wrapper's
/// inherent surface.
pub(crate) trait LayerOps {
    fn resize_for_dpr(&mut self, dpr: i32);
}

pub(crate) struct GridRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> GridRenderer<P> {
    pub(crate) fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) {
        self.core.render_grid(model, frame);
    }

    pub(crate) fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) {
        self.core.render_grid_blit(model, frame, plan);
    }

    pub(crate) fn painter_supports_blit(&self) -> bool {
        self.core.painter().supports_blit()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn pane_cache_range_debug(
        &self,
        region: crate::chrome::PaneRegion,
    ) -> Option<crate::types::coord::RCRange> {
        self.core.pane_cache.pane(region).range.get()
    }

    pub(crate) fn painter_blit(&self, src: crate::geometry::pixel_rect::PixelRect, dst: crate::geometry::pixel_rect::PixelRect) {
        self.core.painter().blit(src, dst);
    }
}

impl GridRenderer<CanvasPainter> {
    pub(crate) fn for_layer(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            core: RendererCore::for_layer(CanvasPainter::new(ctx)),
        }
    }

    pub(crate) fn ctx_ref(&self) -> &CanvasRenderingContext2d {
        self.core.painter().ctx()
    }

    pub(crate) fn invalidate_paint_cache(&mut self) {
        self.core.invalidate_paint_cache();
    }
}

impl<P: Painter> LayerOps for GridRenderer<P> {
    fn resize_for_dpr(&mut self, dpr: i32) {
        self.core.resize_for_dpr(dpr);
    }
}

pub(crate) struct OverlayRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl OverlayRenderer<CanvasPainter> {
    pub(crate) fn for_layer(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            core: RendererCore::for_layer(CanvasPainter::new(ctx)),
        }
    }

    pub(crate) fn ctx_ref(&self) -> &CanvasRenderingContext2d {
        self.core.painter().ctx()
    }
}

impl<P: Painter> OverlayRenderer<P> {
    pub(crate) fn render_overlays(
        &mut self,
        model: &dyn CanvasModel,
        overlays: &RenderOverlays,
        frame: &Chrome,
    ) {
        self.core.render_overlays(model, overlays, frame);
    }
}

impl<P: Painter> LayerOps for OverlayRenderer<P> {
    fn resize_for_dpr(&mut self, dpr: i32) {
        self.core.resize_for_dpr(dpr);
    }
}
