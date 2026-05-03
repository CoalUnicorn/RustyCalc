//! Canvas 2D renderer for the spreadsheet grid.
//!
//! This module is the only piece of RustyCalc that talks to the browser's
//! Canvas 2D API. Everything else - Leptos components, signals, event
//! handlers - lives in `src/components/`. The split is deliberate: Leptos
//! manages reactivity and DOM, but the actual cell grid is a `<canvas>`
//! element drawn imperatively, because HTML tables/divs can't keep up with
//! thousands of cells at 60fps.
//!
//! # How it connects to Leptos
//!
//! The `Worksheet` component (`src/components/worksheet.rs`) owns the
//! `<canvas>` element and holds a `NodeRef` to it. Whenever
//! `state.redraw` (an `RwSignal<u32>`) increments, an `Effect` fires,
//! creates a fresh `CanvasRenderer` from the `NodeRef`, and calls
//! `renderer.render(model, overlays)`. That single call redraws everything.
//!
//!  Each canvas gets its own `LayerBase` (canvas + `PaintGate` + `CanvasRenderer`).
//!  `GridLayer` requests a 2D context with `alpha: false` (opaque, lets the
//!  browser skip alpha compositing); `OverlayLayer` requests
//!  `alpha: true, desynchronized: true`. The renderer constructed with
//!  `CanvasRenderer::for_layer` is **long-lived per layer** — it owns the 2D ctx,
//!  so the cached `set_fill`/`set_stroke`/`set_font`/`set_line_width` state
//!  persists across frames.
//!
//! # Render pipeline
//!
//! `render()` runs four phases in order, each building on the previous:
//!
//! ```text
//! TODO
//! ```
//!
//! Text is deferred to Phase 4 because earlier phases may paint over cells
//! (e.g. the selection fill tint covers an area). Drawing text last keeps
//! it readable.
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
//!
//! # Border resolution
//!
//! TODO

mod cache;
mod cells;
mod headers;
mod overlays;
mod paint;
mod pane;
mod text;
mod text_paint;
mod viewport;

use std::cell::Cell;
use web_sys::js_sys;
use web_sys::CanvasRenderingContext2d;

use super::geometry::CanvasSize;
use crate::geometry::frame::FrameContext;
use crate::geometry::prim::Axis;
use crate::layer::RenderOverlays;
use crate::renderer::cache::CachedColor;
use crate::renderer::cache::FrameCache;
use crate::renderer::cells::CellPaintsIter;
use crate::theme::CanvasTheme;
use crate::CanvasModel;
pub(crate) use pane::PaneRegion;

//#[derive(Clone)]
pub struct CanvasRenderer {
    ctx: CanvasRenderingContext2d,
    width: f64,
    height: f64,
    dpr: f64,
    theme: CanvasTheme,
    /// Cached dash pattern passed to `set_line_dash` on every dashed stroke
    /// (clipboard ants, point-mode range, formula refs).
    /// Single overlay pass can hit this N times per frame.
    /// Allocated once in `new()` so `rect_dashed`.
    dash_pattern: js_sys::Array,
    /// Empty array used to clear the dash pattern after a dashed stroke.
    dash_empty: js_sys::Array,
    frame_cache: FrameCache,
}

impl CanvasRenderer {
    /// Package the canvas's logical pixel extent for pixel-space predicates
    /// like `PixelRect::intersects`.
    #[inline]
    pub(crate) fn canvas_size(&self) -> CanvasSize {
        CanvasSize {
            w: self.width,
            h: self.height,
        }
    }

    /// Borrow the canvas 2D context (for measurement + font setup during
    /// paint resolution in `crate::types`).
    #[inline]
    pub(crate) fn ctx_ref(&self) -> &CanvasRenderingContext2d {
        &self.ctx
    }

    /// Borrow the active theme (for paint resolution in `crate::types`).
    #[inline]
    pub(crate) fn theme(&self) -> &CanvasTheme {
        &self.theme
    }

    /// Stream of resolved `CellPaint` for a pane. Each yielded paint is
    /// renderer-ready: bg + four borders + optional text are pre-resolved
    /// against neighbour styles, so the paint pass touches the model
    /// zero times.
    pub(super) fn paints_in<'a>(
        &'a self,
        model: &'a dyn CanvasModel,
        pane: &'a PaneRegion,
    ) -> CellPaintsIter<'a> {
        CellPaintsIter::new(self, model, pane)
    }

    /// Layer-friendly constructor: caller owns canvas sizing + DPR scaling.
    ///
    /// Used by `GridLayer` / `OverlayLayer`, which build their own ctx with
    /// alpha/desynchronized options and apply DPR scale in their own
    /// `resize()`. This keeps a long-lived `CanvasRenderer` whose ctx is the
    /// layer's ctx — paint caches survive across frames.
    pub(crate) fn for_layer(
        ctx: CanvasRenderingContext2d,
        css_w: f64,
        css_h: f64,
        theme: CanvasTheme,
    ) -> Self {
        Self {
            ctx,
            width: css_w,
            height: css_h,
            dpr: 1.0,
            theme,
            dash_pattern: js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into()),
            dash_empty: js_sys::Array::new(),
            frame_cache: FrameCache {
                last_fill: Cell::new(CachedColor::Empty),
                last_stroke: Cell::new(CachedColor::Empty),
                last_line_width: Cell::new(0.0),
                last_font: Cell::new(CachedColor::Empty),
                text_slots: Cell::new(Vec::new()),
                show_grid: Cell::new(true),
            },
        }
    }

    /// Sync logical canvas size after a layer resize. Caller is responsible
    /// for the actual `canvas.set_width/set_height` and DPR scale.
    pub(crate) fn set_size(&mut self, css_w: f64, css_h: f64) {
        self.width = css_w;
        self.height = css_h;
    }

    /// Sync device-pixel ratio after a layer resize. Must be called
    /// alongside `set_size` so snap helpers reflect the current DPR.
    pub(crate) fn set_dpr(&mut self, dpr: f64) {
        self.dpr = dpr;
    }

    /// Snap a coordinate to the center of the nearest device pixel.
    /// Applied to stroke axes so 1-px lines land on one physical pixel
    /// rather than bleeding across two.
    #[inline]
    fn snap_stroke(&self, coord: f64) -> f64 {
        ((coord * self.dpr).floor() + 1.0) / self.dpr
    }

    /// Snap a coordinate to the nearest device pixel boundary.
    /// Applied to text draw positions so glyphs don't smear across
    /// sub-pixel boundaries.
    #[inline]
    fn snap_pixel(&self, coord: f64) -> f64 {
        (coord * self.dpr).round() / self.dpr
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

    /// Live theme swap. `CanvasTheme` is `Copy` so this is a simple field
    /// replace.
    pub(crate) fn set_theme(&mut self, theme: CanvasTheme) {
        self.theme = theme;
    }

    /// Phases 1+2: cells (bg + borders + text), frozen separators, headers,
    /// corner box. Does **not** clear the canvas — caller owns the clear so
    /// layer-owned renderers can paint a background fill instead.
    pub(crate) fn render_grid(&mut self, model: &dyn CanvasModel, frame: &FrameContext) {
        // Cache the per-sheet grid-line toggle once for this frame so the
        // hot per-cell `paint_borders` walk doesn't re-enter the model. Falls
        // back to "show" on model failure, matching Excel's default-on.
        let sheet = model.get_selected_sheet();
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));

        // Phase 1: Cells — four frozen-pane quadrants. `render_pane` paints
        // bg+borders, then text on top. Each cell now owns its right+bottom
        // grid stroke directly (see `paint_borders`), so colored fills
        // overpaint the previous-cell's grid edge naturally and explicit
        // 2 px borders win by stroke width.
        self.render_pane(model, PaneRegion::top_left(&frame.frozen));
        self.render_pane(model, PaneRegion::top_right(&frame.frozen, &frame.vis));
        self.render_pane(model, PaneRegion::bottom_left(&frame.frozen, &frame.vis));
        self.render_pane(model, PaneRegion::bottom_right(&frame.frozen, &frame.vis));

        // Frozen separators paint AFTER cells so the thick divider wins its
        // pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.draw_frozen_separators(&frame.frozen);

        // Phase 2: Headers + corner box
        self.render_headers_base(Axis::Row, frame);
        self.render_headers_base(Axis::Column, frame);

        self.draw_corner_box();
    }

    /// Phase 3: selection outline, extend preview, clipboard ants,
    /// point-mode range, formula-ref highlights. Does **not** clear the
    /// canvas — caller owns the clear (overlay layer needs transparent bg).
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
        } else {
        };
    }
}
