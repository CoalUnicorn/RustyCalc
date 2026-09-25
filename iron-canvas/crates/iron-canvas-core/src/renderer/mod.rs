//! Renderer core for the spreadsheet grid.
//!
//! # Lifecycle
//!
//! `Orchestrator<S>` (in [`crate::orchestrator`]) owns two
//! [`LayerBase<S, R>`](crate::surface::LayerBase) values: one for the grid,
//! one for the overlay. Each `LayerBase` holds a [`Surface`](crate::surface::Surface)
//! and a layer-specific renderer wrapping [`RendererCore`] — and no dirty
//! state of its own. In the wasm build the surface is
//! `iron_canvas_canvas2d::WebSurface`; both contexts keep alpha
//! (`alpha: true`) so empty buffers stay transparent. Sheet backgrounds
//! must remain opaque because background fills erase retained content.
//! The overlay also uses `desynchronized: true`. The renderer is long-lived per
//! layer, so the painter's cached fill/stroke/font/line-width state
//! survives across frames.
//!
//! State pushes from the host mark work on `Orchestrator`'s single pending
//! value. `Orchestrator::render_pending` picks a strategy from it and drives
//! the layers that strategy paints through their `LayerBase` paint method:
//! `paint_grid` / `paint_grid_blit` for the grid, `paint_overlay_layer`
//! for the overlay. The grid path calls into [`RendererCore::render_grid`];
//! the overlay path iterates the [`Layer`](crate::decoration::Layer)
//! decorations in `crate::decoration` and calls back into `RendererCore`
//! for the active-cell repaint and header highlights.
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `render_pending` per strategy:
//!
//! - [`RendererCore::render_grid`] paints cells (four frozen-pane
//!   quadrants, each running five cell sub-passes: bg, then CF decoration,
//!   then grid borders, then explicit borders, then text), then frozen
//!   separators, then headers, then the corner box.
//! - `LayerBase::paint_overlay_layer` orchestrates the decorations in
//!   `crate::decoration` (selection, autofill, clipboard, point-mode,
//!   formula-refs) plus header highlights.
//!
//! The cell sub-pass order is the contract: grid borders run across the
//! whole segment before explicit borders, so an explicit `right` on cell A
//! wins over cell B's grid `left` at the shared pixel column. Text runs last;
//! text that would escape its cell is clipped by the text painter.
//!
//! # Frozen panes
//!
//! The grid splits into up to four quadrants (`TopLeft`, `TopRight`,
//! `BottomLeft`, `BottomRight`) based on frozen rows and columns. Each
//! quadrant is rendered as one segment of the grid walk against a different
//! [`PaneRegion`](crate::chrome::PaneRegion); a thick separator line
//! marks the freeze boundary:
//!
//! ```text
//!           frozen cols │ scrollable cols
//!         ──────────────┼──────────────────
//! frozen   TopLeft      │ TopRight
//! rows     (static)     │ (scrolls in X)
//!         ──────────────┼──────────────────
//! scroll   BottomLeft   │ BottomRight
//! rows     (scrolls Y)  │ (scrolls in X and Y)
//! ```
//!
//! With no frozen rows or columns the grid is a single `BottomRight`
//! quadrant.

pub mod blit;
pub mod cache;
pub mod cell;
pub mod chrome;
pub mod diagnostics;
mod grid;
mod layers;
pub mod prepared;
mod repaint;
mod trace;
// `renderer/overlay/` has moved to `src/decoration/`. Each decoration is
// a struct that impls `Layer`; the orchestration that used to live in
// `RendererCore::render_overlays` is now in
// `LayerBase::paint_overlay_layer` (src/layer/mod.rs).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub use crate::chrome::PaneRegion;
use crate::renderer::cache::{FrameCache, GridCache};
pub use cache::ColorIntern;
pub use cache::FontIntern;

pub use self::cell::text::{TextLine, layout_into};
pub(crate) use self::layers::GridPaintOutcome;
pub use self::layers::{GridRenderer, LayerOps, OverlayRenderer};

use crate::frame::FrameTrace;
use crate::painter::Painter;
use crate::renderer::prepared::FetchedCells;
pub(crate) use crate::renderer::prepared::GridCacheCommit;

/// Shared renderer core. Holds the painter `P`, dpr, the per-frame
/// `FrameCache`, and the renderer-lifetime intern tables (font, column
/// labels, per-cell color overrides). The two layer wrappers
/// (`GridRenderer`, `OverlayRenderer`) each own a `RendererCore` and
/// re-export only what their layer is allowed to perform: `GridRenderer`
/// exposes `render_grid` + the four-phase pipeline; `OverlayRenderer`
/// exposes `painter()` + `repaint_active_cell` + `render_header_highlights`
/// for `LayerBase::paint_overlay_layer` to drive the decoration walk.
pub struct RendererCore<P: Painter> {
    /// The surface owns the painter as the semantic source of truth; the
    /// renderer holds a shared handle so paint methods reach the painter
    /// without re-borrowing through the surface on every call.
    pub painter: Rc<P>,
    dpr: f64,
    pub frame_cache: FrameCache,
    /// Renderer-lifetime exact-layout grid buffers and fingerprint truth.
    pub grid_cache: GridCache,
    /// Renderer-lifetime intern table for `ctx.font` strings. Lives outside
    /// `FrameCache` because identical fonts repeat across frames, not just
    /// within a single paint.
    pub font_intern: FontIntern,
    /// Renderer-lifetime intern of per-cell color overrides (border + text).
    /// Hot-path callers (`BorderPaint::resolve`, `CellTextStyle::resolve`)
    /// previously allocated a fresh `String` per cell per frame; the intern
    /// makes those calls `Rc::clone` after the first sighting of each color.
    pub color_intern: ColorIntern,
    /// This frame's paint attribution. `Cell` because every paint method runs
    /// on `&self` (the crate's paint-never-holds-`&mut` convention), and
    /// `FrameTrace` is `Copy`.
    trace: Cell<FrameTrace>,
    /// Dev-only structured capture state. `pub(crate)` so the gated
    /// capture methods in `renderer::diagnostics` can read it; zero-size
    /// contribution to production builds (feature-gated), and all writes
    /// are no-ops while its `enabled` flag is false.
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) diag: diagnostics::DiagState,
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
    pub fn resize_for_dpr(&mut self, dpr: f64) {
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
            dpr: 1.0,
            frame_cache: FrameCache {
                text_slots: Cell::new(Vec::new()),
                show_grid: Cell::new(true),
                text_lines: Cell::new(Vec::new()),
                wrap_buf: RefCell::new(String::new()),
                strip_scratch: RefCell::new(vec![FetchedCells::default()]),
            },
            grid_cache: GridCache::default(),
            font_intern: FontIntern::new(),
            color_intern: ColorIntern::new(),
            trace: Cell::new(FrameTrace::default()),
            #[cfg(feature = "dev-diagnostics")]
            diag: diagnostics::DiagState::default(),
        }
    }
}
