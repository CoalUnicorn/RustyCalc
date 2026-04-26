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
//! The renderer is intentionally stateless between frames - it's
//! constructed, used, and dropped each redraw. This avoids stale-state
//! bugs: canvas size, DPR, and theme can change between frames.
//!
//! # Render pipeline
//!
//! `render()` runs four phases in order, each building on the previous:
//!
//! ```text
//! Phase 1 - Cell backgrounds and borders
//!   For each of the four frozen-pane quadrants, iterate visible cells.
//!   Paint the fill color, then resolve and draw all four border edges.
//!   Collect text layout (`CellText`) into a Vec for Phase 4.
//!
//! Phase 2 - Row and column headers
//!   Paint the grey header bars with row numbers and column letters (A, B, ...).
//!   Selected headers get a highlighted background.
//!
//! Phase 3 - Selection and overlays
//!   Draw the blue selection rectangle, autofill handle, clipboard marching
//!   ants, and point-mode range highlight on top of the cell grid.
//!
//! Phase 4 - Cell text
//!   Paint all collected `CellText` entries last so text always appears
//!   above backgrounds, selection tint, and header lines.
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
//! Each cell has four border edges (left, top, right, bottom). The
//! renderer resolves each edge by checking, in order:
//! 1. The cell's own explicit border (from styling)
//! 2. The adjacent neighbour's matching border (left cell's right, etc.)
//! 3. The background color of either cell (for a clean edge between fills)
//! 4. The grid line color (thin grey default)
//!
//! # Key types
//!
//! - `CanvasRenderer` - short-lived; created per frame from a canvas element
//! - `CellText` / `TextLine` - pre-computed text layout collected during
//!   Phase 1 and painted in Phase 4
//! - `RenderOverlays` - selection/clipboard/point-mode state passed in from
//!   the Worksheet component each frame
//! - `CanvasTheme` (`src/model/theme.rs`) - static color palette; the Canvas 2D
//!   API can't read CSS variables, so concrete color strings are needed

mod cells;
mod headers;
mod overlays;
mod paint;
mod text;
mod viewport;

use wasm_bindgen::JsCast;
use web_sys::js_sys;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use super::geometry::{CanvasSize, FrameContext, SheetViewport};
use super::types::*;
pub(crate) use crate::geometry::VisibleRegion;
use crate::renderer::text::CellText;
use crate::theme::CanvasTheme;
use crate::CanvasModel;
pub use overlays::AutofillTarget;

// Layout constants
pub(super) const SELECTION_BORDER_WIDTH: f64 = 2.0;
pub(super) const STANDARD_BORDER_WIDTH: f64 = 1.0;
pub(super) const MEDIUM_BORDER_WIDTH: f64 = 2.0;
pub(super) const THICK_BORDER_WIDTH: f64 = 3.0;
pub(super) const DASHED_BORDER_WIDTH: f64 = 1.5;

pub struct CanvasRenderer {
    ctx: CanvasRenderingContext2d,
    width: f64,
    height: f64,
    theme: CanvasTheme,
    /// Cached dash pattern passed to `set_line_dash` on every dashed stroke
    /// (clipboard ants, point-mode range, formula refs).
    /// Single overlay pass can hit this N times per frame.
    /// Allocated once in `new()` so `rect_dashed`.
    dash_pattern: js_sys::Array,
    /// Empty array used to clear the dash pattern after a dashed stroke.
    dash_empty: js_sys::Array,
}

impl CanvasRenderer {
    /// Package the canvas's logical pixel extent for pixel-space predicates
    /// like `PixelRect::intersects`.
    #[inline]
    pub(super) fn canvas_size(&self) -> CanvasSize {
        CanvasSize {
            w: self.width,
            h: self.height,
        }
    }

    /// Bind a renderer to `canvas` and apply device-pixel-ratio scaling.
    ///
    /// `dpr` is injected by the caller (typically `window().device_pixel_ratio()`
    /// from the Leptos shell) so this module stays free of framework globals -
    /// a prerequisite for the future `rusty-calc-core` crate split.
    ///
    /// **Performance note:** `canvas.set_width()` / `set_height()` resets the
    /// entire canvas bitmap and all 2D context state - even when the value is
    /// unchanged.  On a 1920x1080 display at 2x DPR that is a ~32 MB backing
    /// store reallocation every frame, which causes >500 ms lag on rapid
    /// navigation (held arrow keys, resize drags).
    ///
    /// Fix: only resize when dimensions actually changed.  When the size is
    /// stable, reset only the transform matrix to the identity before
    /// re-applying the DPR scale.  `clear_rect` in `render()` handles the
    /// pixel clear without touching the backing store.
    #[allow(clippy::expect_used)]
    pub fn new(canvas: &HtmlCanvasElement, theme: CanvasTheme, dpr: f64) -> Self {
        let ctx = canvas
            .get_context("2d")
            .expect("getContext should not throw")
            .expect("2d context must exist")
            .unchecked_into::<CanvasRenderingContext2d>();

        let width = canvas.client_width() as f64;
        let height = canvas.client_height() as f64;

        let target_w = (width * dpr) as u32;
        let target_h = (height * dpr) as u32;

        if canvas.width() != target_w || canvas.height() != target_h {
            // Resize resets canvas bitmap + all context state; necessary here.
            canvas.set_width(target_w);
            canvas.set_height(target_h);
        } else {
            // Reset only the transform so the DPR scale below is applied to
            // the identity matrix, not accumulated across frames.
            ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
                .expect("set_transform should not fail");
        }
        ctx.scale(dpr, dpr).expect("scale should not fail");

        Self {
            ctx,
            width,
            height,
            theme,
            dash_pattern: js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into()),
            dash_empty: js_sys::Array::new(),
        }
    }

    /// Renders only visible cells regardless of selection size.
    pub fn render(&mut self, model: &dyn CanvasModel, overlays: &RenderOverlays) {
        // One snapshot of "where cells are drawn right now" - model + scroll
        // anchors + frozen geometry. The `frame` it produces bundles visible
        // region, pixel-offset prefix sums, and the resolved frozen pane shape
        // so every render phase reads a single source of truth.
        let viewport = SheetViewport::current(model);
        let frame = viewport.frame(self.canvas_size());

        let ctx = &self.ctx;
        ctx.set_line_width(STANDARD_BORDER_WIDTH);
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.clear_rect(0.0, 0.0, self.width, self.height);

        // Cell texts are collected across ALL panes and rendered last (Phase 4)
        // so they always appear on top of backgrounds, selection fill, and headers.
        let mut cell_texts: Vec<CellText> = Vec::new();

        // Phase 1: Cell backgrounds + borders - four frozen-pane quadrants.
        // Performance note: Each pane is bounded by visible region, ensuring O(visible) complexity
        // regardless of selection size (whole sheet vs single cell).
        self.draw_frozen_separators(&frame.frozen);

        self.render_pane(model, &mut cell_texts, PaneRegion::top_left(&frame.frozen));
        self.render_pane(
            model,
            &mut cell_texts,
            PaneRegion::top_right(&frame.frozen, &frame.vis),
        );
        self.render_pane(
            model,
            &mut cell_texts,
            PaneRegion::bottom_left(&frame.frozen, &frame.vis),
        );
        self.render_pane(
            model,
            &mut cell_texts,
            PaneRegion::bottom_right(&frame.frozen, &frame.vis),
        );

        // Phase 2: Headers + corner box
        self.render_headers(
            model,
            Axis::Row,
            &frame.vis,
            frame.frozen.row_band.as_ref(),
            frame.frozen.offset.origin.y,
        );
        self.render_headers(
            model,
            Axis::Column,
            &frame.vis,
            frame.frozen.col_band.as_ref(),
            frame.frozen.offset.origin.x,
        );

        self.draw_corner_box();

        // Phase 3: Selection outline
        self.draw_selection(model, &frame);
        if let Some(target) = overlays.extend_to {
            self.draw_extend_preview(model, &frame, target);
        }

        // Secondary overlays: clipboard marching ants, point-mode range,
        // formula-ref highlights. Each no-ops if its data is absent or lives
        // on another sheet.
        self.draw_clipboard_overlay(model, &frame, overlays.clipboard.as_ref());
        self.draw_point_overlay(model, &frame, overlays.point_range);
        self.draw_formula_ref_overlays(model, &frame, &overlays.formula_refs);

        // Phase 4: Cell text - always on top
        // Rendered after selection fill so text is readable over the blue tint,
        // and after the active-cell white-fill so text appears on a clean background.
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        for ct in &cell_texts {
            self.render_cell_text(ct);
        }
    }
}
