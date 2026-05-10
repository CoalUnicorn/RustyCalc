//! Top structural layer that owns chrome geometry plus the `FrozenRC` and
//! `PaneSet` child layers.
//!
//! `Chrome` is the per-frame snapshot threaded into every render phase AND
//! every hit-test query. Built once per tick by `Chrome::current(model,
//! canvas, theme)`; both the renderer (`paint_if_dirty`) and the input
//! layer (`IronCanvas::hit_test`, `cell_rect`, `resize_handle_at`) read
//! the same snapshot, so what's painted and what gets hit always agree.
//!
//! Slot vecs live on `PaneSet`; methods that walk them either read through
//! `self.pane_set.<vec>` for now or migrate per-axis to `PaneSet` as we
//! touch them (per the hybrid-axis-split rule).
//!
//! `row_header_thickness` is computed dynamically per frame: the row-header
//! strip widens as scroll position pushes more digits into row labels
//! (1 → 999 fits in the 30px default; 10 000 needs ≈40px; 1 048 576
//! needs ≈60px). Using a char-count approximation rather than threading
//! `TextMetrics` keeps the build path free of painter coupling.

use crate::geometry::frame::slot::{ColSlot, PaneColumns, PaneRows, RowSlot};
use crate::geometry::{
    constants::{
        AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP,
        HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::Point,
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, CanvasView, RCRange};

pub(crate) mod corner_box;
pub(crate) mod frozen_separator;
pub(crate) mod header_strip;
pub(crate) mod pane_region;

pub(crate) use pane_region::PaneRegion;

/// Approx pixel width per digit at the bold 12px Inter header font.
/// Pessimistic enough that no row label clips inside the strip.
const APPROX_DIGIT_WIDTH_PX: i32 = 8;
/// Padding on each side of the row-label text inside the header strip.
const HEADER_LABEL_PAD_PX: i32 = 4;

#[derive(Debug)]
pub(crate) struct Chrome {
    pub sheet: u32,
    pub pane_set: PaneSet,
    /// Width of the row-header strip — measured per frame from the widest
    /// visible row label so the digits never clip past row 999.
    pub row_header_thickness: i32,
    /// Height of the column-header strip. Static today (= `HEADER_ROW_HEIGHT`)
    /// but stored on Chrome so the day this becomes dynamic, only the
    /// assignment in `Chrome::current` changes — paint code already reads
    /// `frame.col_header_thickness`.
    pub col_header_thickness: i32,
    /// Pixel origin where the cell area begins. Single source of truth used
    /// by hit-test and viewport math instead of recomputing
    /// `header + outer_offset` at every site.
    pub cell_origin: Point,
    /// Active selection at paint time, raw `[r1, c1, r2, c2]` from
    /// `SelectedView.range`. Snapshotting it here keeps `autofill_handle`
    /// pure (no model read) and pins the handle position to the *painted*
    /// selection, even if the model's selection mutated between paint and
    /// the next hit-test.
    pub selection_range: RCRange,
    /// Canvas size at which this frame was built. Stored so `is_still_valid`
    /// can detect a resize without the orchestrator passing size separately.
    pub canvas_size: CanvasSize,
    /// Theme this frame was painted with. Snapshot mirrors `canvas_size`:
    /// renderer methods read `frame.theme.*` instead of holding a renderer
    /// field. `IronCanvas::set_theme` marks both layers dirty on change, so
    /// the overlay-only fast path never paints against a stale theme.
    pub theme: CanvasTheme,
}

#[derive(Debug)]
pub(crate) struct PaneSet {
    pub(crate) rows: PaneRows,
    pub(crate) cols: PaneColumns,
}

impl PaneSet {
    /// Walk the row axis: build the frozen row slots and the scrollable
    /// row slots, returning the pixel Y of the frozen-rows separator.
    /// Independent of `row_header_thickness` — runs in Chrome::current's
    /// Phase B, before the row-label measurement. Reads `FROZEN_SEP`
    /// directly because the gap between frozen and scroll bands is
    /// PaneSet's own structural concern, not chrome geometry.
    pub(crate) fn build_rows(
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_y: i32,
        view_top_row: i32,
        canvas_h: f64,
    ) -> PaneRows {
        let mut frozen_rows = Vec::with_capacity(frozen_count as usize);
        let mut y_cursor = origin_y;
        for r in 1..=frozen_count {
            let h = row_height(model, r);
            frozen_rows.push(RowSlot {
                row: r,
                top: y_cursor,
                height: h,
            });
            y_cursor += h;
        }
        let frozen_offset_y = y_cursor + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let row_first = (frozen_count + 1).max(view_top_row);
        let mut scroll_rows: Vec<RowSlot> = Vec::new();
        let mut y_cursor = frozen_offset_y;
        for row in row_first..=LAST_ROW {
            if f64::from(y_cursor) >= canvas_h || row == LAST_ROW {
                let h = row_height(model, row);
                scroll_rows.push(RowSlot {
                    row,
                    top: y_cursor,
                    height: h,
                });
                break;
            }
            let h = row_height(model, row);
            scroll_rows.push(RowSlot {
                row,
                top: y_cursor,
                height: h,
            });
            y_cursor += h;
        }
        PaneRows {
            frozen: frozen_rows,
            scroll: scroll_rows,
            frozen_offset_y,
        }
    }

    /// Walk the column axis using the cell-area X origin (which already
    /// folds in the freshly-measured `row_header_thickness`). Mirrors
    /// `build_rows` shape; runs in Chrome::current's Phase D after
    /// measurement.
    pub(crate) fn build_cols(
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_x: i32,
        view_left_column: i32,
        canvas_w: f64,
    ) -> PaneColumns {
        let mut frozen_cols = Vec::with_capacity(frozen_count as usize);
        let mut x_cursor = origin_x;
        for c in 1..=frozen_count {
            let w = col_width(model, c);
            frozen_cols.push(ColSlot {
                col: c,
                left: x_cursor,
                width: w,
            });
            x_cursor += w;
        }
        let frozen_offset_x = x_cursor + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let col_first = (frozen_count + 1).max(view_left_column);
        let mut scroll_cols: Vec<ColSlot> = Vec::new();
        let mut x_cursor = frozen_offset_x;
        for col in col_first..=LAST_COLUMN {
            if f64::from(x_cursor) >= canvas_w || col == LAST_COLUMN {
                let w = col_width(model, col);
                scroll_cols.push(ColSlot {
                    col,
                    left: x_cursor,
                    width: w,
                });
                break;
            }
            let w = col_width(model, col);
            scroll_cols.push(ColSlot {
                col,
                left: x_cursor,
                width: w,
            });
            x_cursor += w;
        }

        PaneColumns {
            frozen: frozen_cols,
            scroll: scroll_cols,
            frozen_offset_x,
        }
    }

    #[inline]
    pub(crate) fn frozen_rows_count(&self) -> i32 {
        self.rows.frozen.len() as i32
    }

    #[inline]
    pub(crate) fn frozen_cols_count(&self) -> i32 {
        self.cols.frozen.len() as i32
    }

    #[inline]
    fn row_slot(&self, row: i32) -> Option<&RowSlot> {
        if row <= self.frozen_rows_count() {
            self.rows.frozen.get((row - 1) as usize)
        } else {
            let first = self.rows.scroll.first()?.row;
            self.rows.scroll.get((row - first) as usize)
        }
    }

    #[inline]
    fn col_slot(&self, col: i32) -> Option<&ColSlot> {
        if col <= self.frozen_cols_count() {
            self.cols.frozen.get((col - 1) as usize)
        } else {
            let first = self.cols.scroll.first()?.col;
            self.cols.scroll.get((col - first) as usize)
        }
    }

    #[inline]
    pub(crate) fn top_row(&self) -> i32 {
        self.rows.scroll.first().map(|s| s.row).unwrap_or(1)
    }

    #[inline]
    pub(crate) fn left_column(&self) -> i32 {
        self.cols.scroll.first().map(|s| s.col).unwrap_or(1)
    }

    #[inline]
    pub(crate) fn last_visible_row(&self) -> i32 {
        self.rows
            .scroll
            .last()
            .map(|s| s.row)
            .unwrap_or_else(|| self.top_row())
    }

    #[inline]
    pub(crate) fn last_visible_col(&self) -> i32 {
        self.cols
            .scroll
            .last()
            .map(|s| s.col)
            .unwrap_or_else(|| self.left_column())
    }

    #[inline]
    pub(crate) fn row_in_frame(&self, row: i32) -> bool {
        row <= self.frozen_rows_count() || (row >= self.top_row() && row <= self.last_visible_row())
    }

    #[inline]
    pub(crate) fn col_in_frame(&self, col: i32) -> bool {
        col <= self.frozen_cols_count()
            || (col >= self.left_column() && col <= self.last_visible_col())
    }

    #[inline]
    pub(crate) fn row_extent_at(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.height).unwrap_or(0)
    }

    #[inline]
    pub(crate) fn col_extent_at(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.width).unwrap_or(0)
    }

    pub(crate) fn row_to_y(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.top).unwrap_or(0)
    }

    pub(crate) fn col_to_x(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.left).unwrap_or(0)
    }

    pub(crate) fn pixel_to_row(&self, y: i32) -> Option<i32> {
        for s in &self.rows.frozen {
            if y >= s.top && y < s.bottom() {
                return Some(s.row);
            }
        }
        for s in &self.rows.scroll {
            if y >= s.top && y < s.bottom() {
                return Some(s.row);
            }
        }
        None
    }

    pub(crate) fn pixel_to_col(&self, x: i32) -> Option<i32> {
        for s in &self.cols.frozen {
            if x >= s.left && x < s.right() {
                return Some(s.col);
            }
        }
        for s in &self.cols.scroll {
            if x >= s.left && x < s.right() {
                return Some(s.col);
            }
        }
        None
    }

    pub(crate) fn row_boundary_at(&self, y: i32, hit_zone: i32) -> Option<i32> {
        for s in &self.rows.frozen {
            if (s.bottom() - y).abs() <= hit_zone {
                return Some(s.row);
            }
        }
        for s in &self.rows.scroll {
            if (s.bottom() - y).abs() <= hit_zone {
                return Some(s.row);
            }
            if s.bottom() > y + hit_zone {
                break;
            }
        }
        None
    }

    pub(crate) fn col_boundary_at(&self, x: i32, hit_zone: i32) -> Option<i32> {
        for s in &self.cols.frozen {
            if (s.right() - x).abs() <= hit_zone {
                return Some(s.col);
            }
        }
        for s in &self.cols.scroll {
            if (s.right() - x).abs() <= hit_zone {
                return Some(s.col);
            }
            if s.right() > x + hit_zone {
                break;
            }
        }
        None
    }
}

/// Decimal digit count (≥ 1).
fn digit_count(n: i32) -> i32 {
    let mut n = n.max(1);
    let mut d = 0;
    while n > 0 {
        d += 1;
        n /= 10;
    }
    d
}

/// Width (in CSS px) the row-header strip needs to fit the widest label
/// in the visible row range. Pessimistic char-count approximation —
/// trades real font-metric accuracy for zero `TextMetrics` plumbing.
/// Always returns at least `HEADER_COL_WIDTH` so 3-digit labels never
/// shrink the strip below the historical default.
pub(crate) fn measure_row_header_width(max_visible_row: i32) -> i32 {
    let digits = digit_count(max_visible_row);
    let approx = digits * APPROX_DIGIT_WIDTH_PX + 2 * HEADER_LABEL_PAD_PX;
    approx.max(HEADER_COL_WIDTH)
}

impl Chrome {
    /// Build a per-frame snapshot via the phased A→E construction:
    /// A chrome geometry → B walk rows → C measure row_header_thickness
    /// → D walk cols (using measured width) → E assemble.
    pub(crate) fn current(
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
    ) -> Self {
        // None ⇒ JS bridge transient (threw or shape malformed). Fall through
        // with the fresh-model default so the frame still builds; next animation
        // frame re-queries.
        let view = model.get_selected_view().unwrap_or(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1,
            },
            top_row: 1,
            left_column: 1,
        });
        let sheet = model.get_selected_sheet();

        // Phase A — frozen counts only. Chrome-geometry constants
        // (HEADER_ROW_HEIGHT, HEADER_OFFSET, FROZEN_SEP) are read at the
        // point of need below; PaneSet no longer takes them as params.
        let frozen_row_count = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_col_count = model.get_frozen_columns_count(sheet).unwrap_or(0);

        // Phase B — row walk. `origin_y` is the top edge of the cell area;
        // static today (col header is fixed-height) but computed here so
        // the day it goes dynamic, only this line changes.
        let origin_y = HEADER_ROW_HEIGHT + HEADER_OFFSET;
        //let (frozen_rows, scroll_rows, frozen_offset_y) =
        let rows = PaneSet::build_rows(model, frozen_row_count, origin_y, view.top_row, canvas.h);

        // Phase C — measure row_header_thickness from the last visible row label.
        let last_visible_row = rows
            .scroll
            .last()
            .map(|s| s.row)
            .unwrap_or((frozen_row_count + 1).max(view.top_row));
        let row_header_thickness = measure_row_header_width(last_visible_row);

        // Phase D — col walk uses the measured width to anchor `origin_x`.
        let origin_x = row_header_thickness + HEADER_OFFSET;
        //let (frozen_cols, scroll_cols, frozen_offset_x)
        let columns = PaneSet::build_cols(
            model,
            frozen_col_count,
            origin_x,
            view.left_column,
            canvas.w,
        );

        // Phase E — assemble. `cell_origin` reuses the locals from B/D so
        // there's a single source of truth for the cell-area top-left.
        let pane_set = PaneSet {
            rows,
            cols: columns,
        };
        let col_header_thickness = HEADER_ROW_HEIGHT;
        let cell_origin = Point {
            x: origin_x,
            y: origin_y,
        };

        Chrome {
            sheet,
            pane_set,
            row_header_thickness,
            col_header_thickness,
            cell_origin,
            selection_range: view.selection,
            canvas_size: canvas,
            theme: theme.clone(),
        }
    }

    /// True when the painted geometry is identical to the current model state.
    pub(crate) fn is_still_valid(&self, model: &dyn CanvasModel, size: CanvasSize) -> bool {
        if size != self.canvas_size {
            return false;
        }
        let Some(view) = model.get_selected_view() else {
            return false;
        };
        let sheet = model.get_selected_sheet();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let want_top = (frozen_rows + 1).max(view.top_row);
        let want_left = (frozen_cols + 1).max(view.left_column);
        if self.pane_set.top_row() != want_top || self.pane_set.left_column() != want_left {
            return false;
        }
        frozen_rows == self.pane_set.frozen_rows_count()
            && frozen_cols == self.pane_set.frozen_cols_count()
            && sheet == self.sheet
    }

    /// Refresh frame fields that the overlay paints from but that are
    /// independent of the slot vecs. Call on the overlay-only fast path
    /// after `is_still_valid` returned true, before painting.
    pub(crate) fn refresh_overlay_inputs(&mut self, model: &dyn CanvasModel) {
        if let Some(view) = model.get_selected_view() {
            self.selection_range = view.selection;
        }
    }

    pub(crate) fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        let p = &self.pane_set;
        if !p.row_in_frame(row) || !p.col_in_frame(col) {
            return None;
        }
        Some(PixelRect {
            top_left: Point {
                x: p.col_to_x(col),
                y: p.row_to_y(row),
            },
            width: p.col_extent_at(col),
            height: p.row_extent_at(row),
        })
    }

    pub(crate) fn autofill_handle(&self) -> Option<Point> {
        let norm = self.selection_range.normalized();
        let r2 = norm.r2;
        let c2 = norm.c2;
        if r2 >= LAST_ROW || c2 >= LAST_COLUMN {
            return None;
        }
        let p = &self.pane_set;
        if !p.row_in_frame(r2) || !p.col_in_frame(c2) {
            return None;
        }
        Some(Point {
            x: p.col_to_x(c2) + p.col_extent_at(c2),
            y: p.row_to_y(r2) + p.row_extent_at(r2),
        })
    }

    pub(crate) fn autofill_handle_rect(&self) -> PixelRect {
        if let Some(p) = self.autofill_handle() {
            PixelRect {
                top_left: Point {
                    x: p.x - AUTOFILL_HANDLE_PX,
                    y: p.y - AUTOFILL_HANDLE_PX,
                },
                width: AUTOFILL_HANDLE_PX,
                height: AUTOFILL_HANDLE_PX,
            }
        } else {
            PixelRect {
                top_left: Point::default(),
                width: AUTOFILL_HANDLE_PX,
                height: AUTOFILL_HANDLE_PX,
            }
        }
    }

    pub(crate) fn hit_test(&self, x: i32, y: i32) -> HitTest {
        if x < 0 || y < 0 {
            return HitTest::Outside;
        }
        if x < self.cell_origin.x && y < self.cell_origin.y {
            return HitTest::Corner;
        }
        let p = &self.pane_set;
        if y < self.cell_origin.y {
            return match p.pixel_to_col(x) {
                Some(c) => HitTest::ColHeader(c),
                None => HitTest::Outside,
            };
        }
        if x < self.cell_origin.x {
            return match p.pixel_to_row(y) {
                Some(r) => HitTest::RowHeader(r),
                None => HitTest::Outside,
            };
        }
        let (Some(row), Some(column)) = (p.pixel_to_row(y), p.pixel_to_col(x)) else {
            return HitTest::Outside;
        };
        let h = self.autofill_handle_rect();
        let pad = AUTOFILL_HIT_PAD_PX;
        if x >= h.top_left.x - pad
            && x <= h.right() + pad
            && y >= h.top_left.y - pad
            && y <= h.bottom() + pad
        {
            return HitTest::AutofillHandle { row, column };
        }
        HitTest::Cell { row, column }
    }

    pub(crate) fn resize_handle_at(&self, x: i32, y: i32, tolerance: i32) -> Option<ResizeTarget> {
        if y < self.col_header_thickness && x > self.row_header_thickness {
            return self
                .pane_set
                .col_boundary_at(x, tolerance)
                .map(ResizeTarget::Column);
        }
        if x < self.row_header_thickness && y > self.col_header_thickness {
            return self
                .pane_set
                .row_boundary_at(y, tolerance)
                .map(ResizeTarget::Row);
        }
        None
    }
}

pub(crate) fn row_height(model: &dyn CanvasModel, row: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_row_height(sheet, row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
        .round() as i32
}

pub(crate) fn col_width(model: &dyn CanvasModel, col: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_column_width(sheet, col)
        .unwrap_or(DEFAULT_COL_WIDTH)
        .round() as i32
}
