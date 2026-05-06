//! Canvas 2D renderer for the spreadsheet grid.
//!
//! This module is the only piece of RustyCalc that talks to the browser's
//! Canvas 2D API. Everything else - Leptos components, signals, event
//! handlers - lives in `src/components/`. The split is deliberate: Leptos
//! manages reactivity and DOM, but the actual cell grid is a `<canvas>`
//! element drawn imperatively, because HTML tables/divs can't keep up with
//! thousands of cells at 60fps.
//!
//! # Lifecycle
//!
//! Two stacked `<canvas>` elements are wrapped by [`crate::IronCanvas`];
//! each canvas owns a `LayerBase` (canvas + `PaintGate` + `RendererCore`).
//! `GridLayer` builds its 2D context with `alpha: false` (opaque, skips
//! alpha compositing); `OverlayLayer` uses `alpha: true, desynchronized: true`.
//! The renderer is **long-lived per layer** — it owns the 2D ctx, so the
//! cached fill/stroke/font/line-width state persists across frames.
//!
//! State pushes from JS mark layers dirty; `IronCanvas::paint_if_dirty`
//! drives each dirty layer's `paint`, which calls into [`RendererCore::render_grid`]
//! / [`RendererCore::render_overlays`].
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `paint_if_dirty` per dirty layer:
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
//! The grid supports frozen rows and columns (Excel's "Freeze Panes").
//! This splits the canvas into up to four quadrants:
//!
//! ```text
//! ┌    ┬      ┐
//! │ frozen/    │ frozen rows,     │
//! │ frozen     │ scrollable cols  │
//! ├    ┼      ┤
//! │ scrollable │ main scrollable  │
//! │ rows,      │ area             │
//! │ frozen cols│                  │
//! └    ┴      ┘
//! ```
//!
//! Each quadrant is rendered by `render_pane()` with different row/col
//! ranges and pixel offsets. A thick separator line marks the freeze
//! boundary.

mod cache;
mod cells;
mod headers;
mod overlays;
mod paint;
mod pane;
mod text;
mod text_paint;
mod viewport;

use std::cell::{Cell, RefCell};
use web_sys::js_sys;
use web_sys::CanvasRenderingContext2d;

use crate::geometry::frame::FrameContext;
use crate::geometry::prim::Axis;
use crate::layer::RenderOverlays;
use crate::renderer::cache::CachedColor;
use crate::renderer::cache::FrameCache;
use crate::renderer::cells::CellPaintsIter;
use crate::CanvasModel;
pub(crate) use cache::ColNameIntern;
pub(crate) use cache::FontIntern;
pub(crate) use pane::PaneRegion;

/// Shared renderer core. Holds the 2D ctx, dpr, paint caches, and font intern,
/// plus every drawing primitive. The two layer wrappers (`GridRenderer`,
/// `OverlayRenderer`) each own a `RendererCore` and re-export only the entry
/// point that belongs to their layer — `render_grid` for the grid, `render_overlays`
/// for the overlay. This keeps each layer's public surface honest: a grid layer
/// cannot accidentally call `render_overlays` and vice versa.
pub(crate) struct RendererCore {
    ctx: CanvasRenderingContext2d,
    dpr: i32,
    /// Cached dash pattern passed to `set_line_dash` on every dashed stroke
    /// (clipboard ants, point-mode range, formula refs).
    /// Single overlay pass can hit this N times per frame.
    /// Allocated once in `new()` so `rect_dashed`.
    dash_pattern: js_sys::Array,
    /// Empty array used to clear the dash pattern after a dashed stroke.
    dash_empty: js_sys::Array,
    frame_cache: FrameCache,
    /// Renderer-lifetime intern table for `ctx.font` strings. Lives outside
    /// `FrameCache` because identical fonts repeat across frames, not just
    /// within a single paint.
    pub(in crate::renderer) font_intern: FontIntern,
    /// Renderer-lifetime intern of column-letter labels. Same rationale as
    /// `font_intern` — column names repeat every frame; cache once, clone the
    /// `Rc<str>` thereafter.
    pub(in crate::renderer) col_intern: ColNameIntern,
}

impl RendererCore {
    /// Borrow the canvas 2D context (for measurement + font setup during
    /// paint resolution in `crate::types`).
    #[inline]
    pub(crate) fn ctx_ref(&self) -> &CanvasRenderingContext2d {
        &self.ctx
    }

    /// Stream of `CellPaint` for a pane. Each yielded paint carries the
    /// cell's address, pixel rect, and `Style` (fetched from the model
    /// during iteration). Border + text resolution happens later, inside
    /// `render_pane`'s deferred sub-passes.
    pub(super) fn paints_in<'a>(
        &'a self,
        model: &'a dyn CanvasModel,
        pane: &'a PaneRegion,
        frame: &'a FrameContext,
    ) -> CellPaintsIter<'a> {
        CellPaintsIter::new(model, pane, frame)
    }

    /// Layer-friendly constructor: caller owns canvas sizing + DPR scaling.
    ///
    /// Used by `GridLayer` / `OverlayLayer`, which build their own ctx with
    /// alpha/desynchronized options and apply DPR scale in their own
    /// `resize()`. This keeps a long-lived `RendererCore` whose ctx is the
    /// layer's ctx — paint caches survive across frames. Canvas size and
    /// theme both live on the per-frame `FrameContext`, not on the renderer.
    pub(crate) fn for_layer(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            ctx,
            dpr: 1,
            dash_pattern: js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into()),
            dash_empty: js_sys::Array::new(),
            frame_cache: FrameCache {
                last_fill: Cell::new(CachedColor::Empty),
                last_stroke: Cell::new(CachedColor::Empty),
                last_line_width: Cell::new(0.0),
                last_font: Cell::new(CachedColor::Empty),
                text_slots: Cell::new(Vec::new()),
                show_grid: Cell::new(true),
                label_buf: RefCell::new(String::new()),
                text_lines: Cell::new(Vec::new()),
            },
            font_intern: FontIntern::new(),
            col_intern: ColNameIntern::new(),
        }
    }

    /// Sync device-pixel ratio after a layer resize. The snap helpers read
    /// `dpr` so this must be called whenever the backing-store DPR changes.
    pub(crate) fn set_dpr(&mut self, dpr: i32) {
        self.dpr = dpr;
    }

    /// Snap a coordinate to the center of the nearest device pixel.
    /// Applied to stroke axes so 1-px lines land on one physical pixel
    /// rather than bleeding across two.
    #[inline]
    fn snap_stroke(&self, coord: f64) -> f64 {
        ((coord * f64::from(self.dpr)) + 1.0) / f64::from(self.dpr)
    }

    /// Reset per-frame ctx state caches to their initial sentinels.
    ///
    /// Required after any `canvas.set_width/set_height` because that mutation
    /// resets all 2D ctx state (fill, stroke, font, line width, text alignment);
    /// without invalidation the cache would skip writes that the ctx has actually
    /// forgotten and the next paint would use the wrong style.
    pub(crate) fn invalidate_paint_cache(&mut self) {
        self.frame_cache.last_fill.set(CachedColor::Empty);
        self.frame_cache.last_stroke.set(CachedColor::Empty);
        self.frame_cache.last_font.set(CachedColor::Empty);
        self.frame_cache.last_line_width.set(0.0);
        // Sticky text defaults wiped by set_width/set_height — restore here
        // so per-frame render_grid / render_overlays calls don't need to.
        self.ctx.set_text_align("center");
        self.ctx.set_text_baseline("middle");
    }

    /// Paint the grid layer: cells (per quadrant), frozen separators,
    /// headers, corner box. Does **not** clear the canvas — caller owns
    /// the clear so layer-owned renderers can paint a background fill
    /// instead.
    pub(crate) fn render_grid(&mut self, model: &dyn CanvasModel, frame: &FrameContext) {
        // Cache the per-sheet grid-line toggle once for this frame so the
        // hot per-cell `paint_borders_grid` walk doesn't re-enter the model.
        // Falls back to "show" on model failure, matching Excel's default-on.
        let sheet = model.get_selected_sheet();
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));

        self.render_pane(model, PaneRegion::top_left(&frame.frozen), frame);
        self.render_pane(
            model,
            PaneRegion::top_right(&frame.frozen, &frame.vis),
            frame,
        );
        self.render_pane(
            model,
            PaneRegion::bottom_left(&frame.frozen, &frame.vis),
            frame,
        );
        self.render_pane(
            model,
            PaneRegion::bottom_right(&frame.frozen, &frame.vis),
            frame,
        );

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.draw_frozen_separators(frame);

        self.render_headers_base(Axis::Row, frame);
        self.render_headers_base(Axis::Column, frame);

        self.draw_corner_box(frame);
    }

    /// Paint the overlay layer: selection outline + autofill handle, header
    /// highlights, extend preview, clipboard marching ants, point-mode range,
    /// formula-ref highlights. Does **not** clear the canvas — caller owns
    /// the clear (overlay layer needs transparent bg).
    pub(crate) fn render_overlays(
        &mut self,
        model: &dyn CanvasModel,
        overlays: &RenderOverlays,
        frame: &FrameContext,
    ) {
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
    }
}

// Layer-facing wrappers
//
// `GridRenderer` and `OverlayRenderer` each own a `RendererCore` and re-export
// only the operations their layer is allowed to perform. `LayerOps` is the
// shared subset (`ctx_ref` for clear/fill, `set_dpr`/`invalidate_paint_cache`
// for resize); the only divergence is the entry point — `render_grid` vs
// `render_overlays`. Calling one on the wrong layer is a compile error,
// not a runtime mistake.

/// The slice of `RendererCore` that both layers need access to during
/// `LayerBase::resize` and per-layer paint setup.
pub(crate) trait LayerOps {
    fn ctx_ref(&self) -> &CanvasRenderingContext2d;
    fn set_dpr(&mut self, dpr: i32);
    fn invalidate_paint_cache(&mut self);
}

pub(crate) struct GridRenderer {
    core: RendererCore,
}

impl GridRenderer {
    pub(crate) fn for_layer(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            core: RendererCore::for_layer(ctx),
        }
    }

    pub(crate) fn render_grid(&mut self, model: &dyn CanvasModel, frame: &FrameContext) {
        self.core.render_grid(model, frame);
    }
}

impl LayerOps for GridRenderer {
    fn ctx_ref(&self) -> &CanvasRenderingContext2d {
        self.core.ctx_ref()
    }
    fn set_dpr(&mut self, dpr: i32) {
        self.core.set_dpr(dpr);
    }
    fn invalidate_paint_cache(&mut self) {
        self.core.invalidate_paint_cache();
    }
}

pub(crate) struct OverlayRenderer {
    core: RendererCore,
}

impl OverlayRenderer {
    pub(crate) fn for_layer(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            core: RendererCore::for_layer(ctx),
        }
    }

    pub(crate) fn render_overlays(
        &mut self,
        model: &dyn CanvasModel,
        overlays: &RenderOverlays,
        frame: &FrameContext,
    ) {
        self.core.render_overlays(model, overlays, frame);
    }
}

impl LayerOps for OverlayRenderer {
    fn ctx_ref(&self) -> &CanvasRenderingContext2d {
        self.core.ctx_ref()
    }
    fn set_dpr(&mut self, dpr: i32) {
        self.core.set_dpr(dpr);
    }
    fn invalidate_paint_cache(&mut self) {
        self.core.invalidate_paint_cache();
    }
}
