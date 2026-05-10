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
//! The grid splits into up to four quadrants (top_left, top_right,
//! bottom_left, bottom_right) based on frozen rows + columns. Each
//! quadrant is rendered by `render_pane()` against a different
//! `PaneRegion`; a thick separator line marks the freeze boundary. See
//! the diagram in `ARCHITECTURE.md` for the layout.

pub(crate) mod cache;
mod text;
pub(crate) mod text_paint;
mod viewport;

use std::cell::{Cell, RefCell};
use web_sys::CanvasRenderingContext2d;

use crate::chrome::Chrome;
use crate::geometry::prim::Axis;
use crate::layer::RenderOverlays;
use crate::painter::CanvasPainter;
use crate::renderer::cache::FrameCache;
use crate::CanvasModel;
pub(crate) use crate::cell::PaneCells;
pub(crate) use crate::chrome::PaneRegion;
pub(crate) use cache::ColNameIntern;
pub(crate) use cache::ColorIntern;
pub(crate) use cache::FontIntern;

#[cfg(test)]
pub(crate) use text_paint::{layout_into, TextLine};

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
    /// Canvas size and theme both live on the per-frame `FrameContext`,
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
                pane_styles: Cell::new(Vec::new()),
                pane_values: Cell::new(Vec::new()),
                pane_cell_types: Cell::new(Vec::new()),
            },
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

        self.render_pane(model, PaneRegion::top_left(), frame);
        self.render_pane(model, PaneRegion::top_right(), frame);
        self.render_pane(model, PaneRegion::bottom_left(), frame);
        self.render_pane(model, PaneRegion::bottom_right(), frame);

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.draw_frozen_separators(frame);

        self.render_headers_base(Axis::Row, frame);
        self.render_headers_base(Axis::Column, frame);

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
