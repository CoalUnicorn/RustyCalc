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
mod text;
mod viewport;

use ironcalc_base::UserModel;

use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use super::types::*;
use crate::theme::{CanvasTheme, FORMULA_REF_COLORS};

// Layout constants
pub(super) const SELECTION_BORDER_WIDTH: f64 = 2.0;
pub(super) const STANDARD_BORDER_WIDTH: f64 = 1.0;
pub(super) const MEDIUM_BORDER_WIDTH: f64 = 2.0;
pub(super) const THICK_BORDER_WIDTH: f64 = 3.0;
pub(super) const DASHED_BORDER_WIDTH: f64 = 1.5;
// With textBaseline:"middle", center_y is the em-square midpoint.
// The typographic baseline sits at ~center_y + font_size*0.15; placing
// the underline at 0.35x puts it just below the baseline, clear of the glyphs.

// CanvasRenderer

pub struct CanvasRenderer {
    ctx: CanvasRenderingContext2d,
    width: f64,
    height: f64,
    theme: CanvasTheme,
    /// Visible cell bounds - populated at the start of each `render()` call.
    /// Stored on the struct so internal helpers don't need it as a parameter.
    vis: VisibleRegion,
    /// Precomputed pixel offsets for visible rows/cols - populated alongside
    /// `vis`. Turns `cell_x`/`cell_y` from O(visible x R) into O(1).
    offsets: PixelOffsets,
}

impl CanvasRenderer {
    /// Bind a renderer to `canvas` and apply device-pixel-ratio scaling.
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
    pub fn new(canvas: &HtmlCanvasElement, theme: CanvasTheme) -> Self {
        let ctx = canvas
            .get_context("2d")
            .expect("getContext should not throw")
            .expect("2d context must exist")
            .unchecked_into::<CanvasRenderingContext2d>();

        let width = canvas.client_width() as f64;
        let height = canvas.client_height() as f64;
        let dpr = leptos::prelude::window().device_pixel_ratio();

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
            vis: VisibleRegion::default(),
            offsets: PixelOffsets::default(),
        }
    }

    // Entry point

    /// Renders only visible cells regardless of selection size.
    pub fn render(&mut self, model: &UserModel, overlays: &RenderOverlays) {
        // Calculate visible region FIRST - this is independent of selection
        self.vis = self.visible_cells(model);

        let ctx = &self.ctx;
        ctx.set_line_width(STANDARD_BORDER_WIDTH);
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.clear_rect(0.0, 0.0, self.width, self.height);

        // Performance check: log when rendering with large selections for debugging
        let view = model.get_selected_view();
        // let selection_size =
        //     (view.range[2] - view.range[0] + 1) as i64 * (view.range[3] - view.range[1] + 1) as i64;
        // DEBUG
        // if selection_size > 10_000 {
        //     web_sys::console::log_1(
        //         &format!(
        //             "Rendering with large selection: {} cells, visible: {}x{}",
        //             selection_size,
        //             self.vis.col_last - self.vis.col_first + 1,
        //             self.vis.row_last - self.vis.row_first + 1
        //         )
        //         .into(),
        //     );
        // }

        let sheet = view.sheet;

        // Frozen counts + pixel origin, computed once per frame.
        let frc = FrozenRC::from_model(model, sheet);
        // Build prefix-sum cache now that vis and sheet are both known.
        self.offsets = self.build_pixel_offsets(model, sheet);
        let vis = self.vis;

        // Cell texts are collected across ALL panes and rendered last (Phase 4)
        // so they always appear on top of backgrounds, selection fill, and headers.
        let mut cell_texts: Vec<CellText> = Vec::new();

        // Phase 1: Cell backgrounds + borders - four frozen-pane quadrants.
        // Performance note: Each pane is bounded by visible region, ensuring O(visible) complexity
        // regardless of selection size (whole sheet vs single cell).
        self.render_pane(model, sheet, &mut cell_texts, PaneRegion::top_left(&frc));

        self.draw_frozen_separators(&frc);

        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            PaneRegion::top_right(&frc, &vis),
        );
        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            PaneRegion::bottom_left(&frc, &vis),
        );
        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            PaneRegion::bottom_right(&frc, &vis),
        );

        // Phase 2: Headers + corner box
        self.render_row_headers(model, sheet, frc.rows, frc.offset.y);
        self.render_column_headers(model, sheet, frc.cols, frc.offset.x);

        self.draw_corner_box();

        // Phase 3: Selection outline
        self.draw_selection(model, sheet, frc.offset);
        if let Some(target) = overlays.extend_to {
            self.draw_extend_preview(model, sheet, frc.offset, target);
        }

        // Marching-ants border around the last Ctrl+C copied range.
        if let Some(ref cb) = overlays.clipboard {
            if cb.sheet == sheet {
                self.draw_dashed_range(
                    model,
                    sheet,
                    frc.offset,
                    cb.area.normalized(),
                    self.theme.selection_color,
                    DashFill::Outline,
                );
            }
        }

        // Point-mode range: blue dashed outline + light fill tint.
        if let Some(ref pr) = overlays.point_range {
            self.draw_dashed_range(
                model,
                sheet,
                frc.offset,
                pr.normalized(),
                self.theme.pointing,
                DashFill::Tinted,
            );
        }

        // Formula reference overlays — Phase 3, after point-mode.
        // Each ref gets its own color (dashed border + 8% fill tint).
        // Only refs on the current sheet are drawn; cross-sheet refs are silently skipped.
        for fr in &overlays.formula_refs {
            if fr.sheet_area.sheet == sheet {
                self.draw_dashed_range(
                    model,
                    sheet,
                    frc.offset,
                    fr.sheet_area.area.normalized(),
                    FORMULA_REF_COLORS[fr.color_idx % FORMULA_REF_COLORS.len()],
                    DashFill::Tinted,
                );
            }
        }

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
