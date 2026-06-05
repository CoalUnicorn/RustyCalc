//! Renderer core for the spreadsheet grid.
//!
//! # Lifecycle
//!
//! `Orchestrator<S>` (in [`crate::orchestrator`]) owns two
//! [`LayerBase<S, R>`](crate::layer::LayerBase) values: one for the grid,
//! one for the overlay. Each `LayerBase` holds a [`Surface`](crate::layer::Surface),
//! a [`PaintGate`](crate::layer::PaintGate), and a layer-specific renderer
//! wrapping [`RendererCore`]. In the wasm build the surface is
//! `iron_canvas_web::WebSurface`; the grid context uses `alpha: false`
//! (opaque, skips alpha compositing) and the overlay uses
//! `alpha: true, desynchronized: true`. The renderer is long-lived per
//! layer, so the painter's cached fill/stroke/font/line-width state
//! survives across frames.
//!
//! State pushes from the host mark layers dirty. `Orchestrator::paint_if_dirty`
//! drives each dirty layer through its `LayerBase` paint method:
//! `paint_grid` / `paint_grid_blit` for the grid, `paint_overlay_layer`
//! for the overlay. The grid path calls into [`RendererCore::render_grid`];
//! the overlay path iterates the [`Layer`](crate::decoration::Layer)
//! decorations in `crate::decoration` and calls back into `RendererCore`
//! for the active-cell repaint and header highlights.
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `paint_if_dirty` per dirty layer:
//!
//! - [`RendererCore::render_grid`] paints cells (four frozen-pane
//!   quadrants, each running four cell sub-passes: bg, then grid
//!   borders, then explicit borders, then text), then frozen separators,
//!   then headers, then the corner box.
//! - `LayerBase::paint_overlay_layer` orchestrates the decorations in
//!   `crate::decoration` (selection, autofill, clipboard, point-mode,
//!   formula-refs) plus header highlights.
//!
//! The cell sub-pass order is the contract: grid borders run across the
//! whole pane before explicit borders, so an explicit `right` on cell A
//! wins over cell B's grid `left` at the shared pixel column. Text runs
//! last so overflow is never clipped by a neighbour's bg.
//!
//! # Frozen panes
//!
//! The grid splits into up to four quadrants (`TopLeft`, `TopRight`,
//! `BottomLeft`, `BottomRight`) based on frozen rows and columns. Each
//! quadrant is rendered by `render_pane()` against a different
//! [`PaneRegion`](crate::chrome::PaneRegion); a thick separator line
//! marks the freeze boundary. See the diagram in `ARCHITECTURE.md` for
//! the layout.

pub mod cache;
pub mod cell;
pub mod cf_types;
pub mod frame;
// `renderer/overlay/` has moved to `src/layer/decoration/`. Each
// decoration is now a struct that impls `Layer`; the orchestration that
// used to live in `RendererCore::render_overlays` is now in
// `OverlayLayer::paint`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::CanvasModel;
pub use crate::chrome::PaneRegion;
use crate::chrome::{BlitPlan, Chrome};
use crate::geometry::prim::Axis;
use crate::renderer::cache::{FrameCache, PaneCache};
pub use cache::ColNameIntern;
pub use cache::ColorIntern;
pub use cache::FontIntern;

pub use self::cell::text::{TextLine, layout_into};

use crate::painter::{BlitPainter, GroupClass, Painter};

/// Shared renderer core. Holds the painter `P`, dpr, the per-frame
/// `FrameCache`, and the renderer-lifetime intern tables (font, column
/// labels, per-cell color overrides). The two layer wrappers
/// (`GridRenderer`, `OverlayRenderer`) each own a `RendererCore` and
/// re-export only what their layer is allowed to perform: `GridRenderer`
/// exposes `render_grid` + the four-phase pipeline; `OverlayRenderer`
/// exposes `painter()` + `repaint_active_cell` + `render_header_highlights`
/// for `OverlayLayer` to drive the decoration walk.
pub struct RendererCore<P: Painter> {
    /// The surface owns the painter as the semantic source of truth; the
    /// renderer holds a shared handle so paint methods reach the painter
    /// without re-borrowing through the surface on every call.
    pub painter: Rc<P>,
    dpr: i32,
    pub frame_cache: FrameCache,
    /// Renderer-lifetime per-pane bulk-fetch buffers + last-fetched range.
    /// Sibling of the intern tables below; survives across frames so
    /// `render_pane` can short-circuit when a pane's address didn't
    /// change (Stage 3.2) or strip-fetch the new band (Stage 3.3).
    pub pane_cache: PaneCache,
    /// Renderer-lifetime intern table for `ctx.font` strings. Lives outside
    /// `FrameCache` because identical fonts repeat across frames, not just
    /// within a single paint.
    pub font_intern: FontIntern,
    /// Renderer-lifetime intern of column-letter labels. Same rationale as
    /// `font_intern` — column names repeat every frame; cache once, clone the
    /// `Rc<str>` thereafter.
    pub col_intern: ColNameIntern,
    /// Renderer-lifetime intern of per-cell color overrides (border + text).
    /// Hot-path callers (`BorderPaint::resolve`, `CellTextStyle::resolve`)
    /// previously allocated a fresh `String` per cell per frame; the intern
    /// makes those calls `Rc::clone` after the first sighting of each color.
    pub color_intern: ColorIntern,
}

impl<P: Painter> RendererCore<P> {
    pub fn painter(&self) -> &P {
        self.painter.as_ref()
    }
}

impl<P: Painter> RendererCore<P> {
    /// Wipe the per-frame paint state and restore the sticky text defaults
    /// the renderer assumes at every entry point. Routed through the
    /// `Painter` trait so any backend (Canvas-2D today, Recorder/SVG later)
    /// gets the same reset semantics.
    pub fn invalidate_paint_cache(&mut self) {
        self.painter.invalidate_cache();
        self.painter.reset_text_defaults();
    }

    /// React to a backing-store resize: push the new DPR through the
    /// painter's transform, store it for snap math, and clear caches.
    pub fn resize_for_dpr(&mut self, dpr: i32) {
        self.painter.apply_dpr_transform(dpr);
        self.dpr = dpr;
        self.invalidate_paint_cache();
    }

    /// Layer-friendly constructor: caller owns canvas sizing + DPR scaling.
    /// Canvas size and theme both live on the per-frame `Chrome`,
    /// not on the renderer. Takes the painter as an `Rc` so the surface that
    /// owns the painter can hand the renderer its own owning handle.
    pub fn for_layer(painter: Rc<P>) -> Self {
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

    /// Paint the grid layer for a fresh / slots-reuse frame: cells (per
    /// quadrant), frozen separators, both header strips, corner box. Does
    /// **not** clear the canvas — caller owns the clear so layer-owned
    /// renderers can paint a background fill instead.
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) {
        self.painter.begin_group(GroupClass::Grid);
        self.cache_show_grid(model);

        // `frame.stale_panes` is `ALL` on Fresh; narrower on SlotsReuse —
        // either way each region listed needs its 4-pass walk.
        self.painter.begin_group(GroupClass::Cells);
        for pane in frame.stale_panes.regions() {
            self.render_pane(model, pane, frame);
        }
        self.painter.end_group();

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        self.painter.begin_group(GroupClass::Headers);
        if frame.row_header_thickness > 0 {
            self.render_headers_base(Axis::Row, frame);
        }
        if frame.col_header_thickness > 0 {
            self.render_headers_base(Axis::Column, frame);
        }
        self.painter.end_group();

        // Corner box is gated for *correctness*: at thickness 0 it would
        // still stroke 0.5px border lines spanning the full canvas.
        if frame.row_header_thickness > 0 && frame.col_header_thickness > 0 {
            self.painter.begin_group(GroupClass::Corner);
            self.draw_corner_box(frame);
            self.painter.end_group();
        }

        self.painter.end_group();
    }

    /// Scroll-blit variant: caller's `Painter::blit` already shifted the
    /// kept band, so we rotate the cached pane buffers to match
    /// (`try_shift` per pane in `plan.shift_panes()`), wrap BottomRight in
    /// a clip to `plan.repaint_strip` so the strip alone is repainted, and
    /// only refresh the header strip on the scroll axis (the cross-axis
    /// header is unchanged).
    pub fn render_grid_blit(&self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        // Rotate cached pane buffers to follow the blit's pixel shift so
        // `render_pane_blit`'s strip-fetch only refills the revealed band.
        // Defensive: if a pane's prior range no longer aligns (canvas
        // resize, axis change), drop it so the fallback fetches in bulk.
        for pane in plan.shift_panes().regions() {
            let pane_buf = self.pane_cache.pane(pane);
            let Some(new_range) = pane.range(frame) else {
                pane_buf.range.set(None);
                continue;
            };
            let _ = pane_buf.try_shift(new_range, plan.axis);
        }

        self.painter.begin_group(GroupClass::Grid);
        self.cache_show_grid(model);

        self.painter.begin_group(GroupClass::Cells);
        for pane in frame.stale_panes.regions() {
            if matches!(pane, PaneRegion::BottomRight) {
                self.painter.push_clip(plan.repaint_strip);
                self.render_pane_blit(model, pane, frame, plan.repaint_strip);
                self.painter.pop_clip();
            } else {
                self.render_pane_blit(model, pane, frame, plan.repaint_strip);
            }
        }
        self.painter.end_group();

        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        // Only the scroll-axis header strip shifted; the cross-axis
        // strip's pixels are unchanged.
        self.painter.begin_group(GroupClass::Headers);
        let axis_thickness = match plan.axis {
            Axis::Row => frame.row_header_thickness,
            Axis::Column => frame.col_header_thickness,
        };
        if axis_thickness > 0 {
            self.render_headers_base(plan.axis, frame);
        }
        self.painter.end_group();

        // Corner box is gated for *correctness*: at thickness 0 it would
        // still stroke 0.5px border lines spanning the full canvas.
        if frame.row_header_thickness > 0 && frame.col_header_thickness > 0 {
            self.painter.begin_group(GroupClass::Corner);
            self.draw_corner_box(frame);
            self.painter.end_group();
        }

        self.painter.end_group();
    }

    /// Cache the per-sheet grid-line toggle once for this frame so the
    /// hot per-cell `paint_borders_grid` walk doesn't re-enter the model.
    /// Falls back to "show" on model failure, matching Excel's default-on.
    fn cache_show_grid(&self, model: &dyn CanvasModel) {
        let sheet = model.get_selected_sheet();
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));
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
/// inherent surface. `Painter` ties the renderer's painter type to the
/// `LayerBase`'s `Surface::P` at the type level.
pub trait LayerOps {
    type Painter: Painter;
    fn resize_for_dpr(&mut self, dpr: i32);
}

pub struct GridRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> GridRenderer<P> {
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) {
        self.core.render_grid(model, frame);
    }

    pub fn render_grid_blit(&self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        self.core.render_grid_blit(model, frame, plan);
    }

    /// Drop cached pane-buffer ranges for the masked panes. Plumbed through
    /// from `IronCanvas::paint_content` so a cell-content-changed regime
    /// can force the named panes to refetch on their next `render_pane`
    /// while unmasked panes keep their fingerprint-skip win.
    pub fn invalidate_pane_cache(&self, mask: crate::chrome::PaneRegionMask) {
        self.core.pane_cache.invalidate(mask);
    }

    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            core: RendererCore::for_layer(painter),
        }
    }

    pub fn painter(&self) -> &P {
        self.core.painter()
    }

    pub fn invalidate_paint_cache(&mut self) {
        self.core.invalidate_paint_cache();
    }
}

impl<P: BlitPainter> GridRenderer<P> {
    pub fn painter_blit(
        &self,
        src: crate::geometry::pixel_rect::PixelRect,
        dst: crate::geometry::pixel_rect::PixelRect,
    ) {
        self.core.painter().blit(src, dst);
    }
}

impl<P: Painter> LayerOps for GridRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: i32) {
        self.core.resize_for_dpr(dpr);
    }
}

pub struct OverlayRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> OverlayRenderer<P> {
    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            core: RendererCore::for_layer(painter),
        }
    }

    pub fn painter(&self) -> &P {
        self.core.painter()
    }

    pub fn render_header_highlights(
        &self,
        axis: crate::geometry::prim::Axis,
        frame: &Chrome,
        selection_range: crate::types::coord::RCRange,
    ) {
        self.core
            .render_header_highlights(axis, frame, selection_range);
    }

    pub fn repaint_active_cell(
        &self,
        model: &dyn CanvasModel,
        row: i32,
        column: i32,
        frame: &Chrome,
    ) {
        self.core.repaint_active_cell(model, row, column, frame);
    }
}

impl<P: Painter> LayerOps for OverlayRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: i32) {
        self.core.resize_for_dpr(dpr);
    }
}
