use crate::{
    geometry::{
        constants::{
            AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
            LAST_COLUMN, LAST_ROW,
        },
        frame::{frozen::FrozenRC, pixel_offset::PixelOffsets},
        pixel_rect::PixelRect,
        prim::Point,
        utils::{col_width, row_height},
    },
    theme::CanvasTheme,
    types::ui::{HitTest, ResizeTarget},
    CanvasModel, CanvasSize, RCRange,
};

pub mod frozen;
pub mod pixel_offset;

/// The four index boundaries of the visible (scrollable) area.
#[derive(Debug, Default)]
pub struct VisibleCells {
    /// Top-left scrollable cell on screen.
    pub first: CellRC,
    /// Bottom-right scrollable cell on screen.
    pub last: CellRC,
}

#[derive(Debug, Default)]
pub struct CellRC {
    pub row: i32,
    pub column: i32,
}
/// Per-frame geometric snapshot threaded into every render phase AND every
/// hit-test query.
///
/// Built once per tick by `FrameContext::current(model, canvas)` — both the
/// renderer (`paint_if_dirty`) and the input layer (`IronCanvas::hit_test`,
/// `cell_rect`, `resize_handle_at`) read the same snapshot, so what's painted
/// and what gets hit always agree. Bundles the visible region, the
/// pixel-offset prefix-sum cache, and the resolved frozen-pane geometry so
/// neither phase re-reads them from the model mid-frame.
#[derive(Debug)]
pub(crate) struct FrameContext {
    pub sheet: u32,
    pub vis: VisibleCells,
    pub offsets: PixelOffsets,
    pub frozen: FrozenRC,
    pub top_row: i32, // from model.get_selected_view() — used by orchestrator change detection
    pub left_column: i32,
    /// Active selection at paint time, raw `[r1, c1, r2, c2]` from
    /// `SelectedView.range`. Snapshotting it here keeps `autofill_handle`
    /// pure (no model read) and pins the handle position to the *painted*
    /// selection, even if the model's selection mutated between paint and
    /// the next hit-test.
    pub selection_range: RCRange, //[i32; 4],
    /// Canvas size at which this frame was built. Stored so `is_still_valid`
    /// can detect a resize without the orchestrator passing size separately.
    pub canvas_size: CanvasSize,
    /// Theme this frame was painted with. Snapshot mirrors `canvas_size`:
    /// renderer methods read `frame.theme.*` instead of holding a renderer
    /// field. `IronCanvas::set_theme` marks both layers dirty on change, so
    /// the overlay-only fast path never paints against a stale theme.
    pub theme: CanvasTheme,
}

impl FrameContext {
    /// Build a per-frame snapshot from the model and canvas size.
    ///
    /// Single-pass construction: frozen prefix sums, visible-region scan, and
    /// scrollable prefix sums are all built in one model-walk per axis instead
    /// of the two separate walks the old `compute_visible_region` +
    /// `compute_pixel_offsets` pair required.
    pub(crate) fn current(
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: CanvasTheme,
    ) -> Self {
        let view = model.get_selected_view();
        let frozen = FrozenRC::from_model(model);

        let frozen_rows = frozen.frozen_rows_count();
        let frozen_cols = frozen.frozen_cols_count();

        // Frozen prefix sums
        // One walk per axis; the totals (last entry) give the Y/X offset where
        // the scrollable band starts, avoiding a redundant sum in the scan below.
        let mut frozen_row_tops = Vec::with_capacity((frozen_rows + 1) as usize);
        let mut frozen_h = 0;
        for r in 1..=frozen_rows {
            frozen_row_tops.push(frozen_h);
            frozen_h += row_height(model, r);
        }
        frozen_row_tops.push(frozen_h);

        let mut frozen_col_lefts = Vec::with_capacity((frozen_cols + 1) as usize);
        let mut frozen_w = 0;
        for c in 1..=frozen_cols {
            frozen_col_lefts.push(frozen_w);
            frozen_w += col_width(model, c);
        }
        frozen_col_lefts.push(frozen_w);

        let row_first = (frozen_rows + 1).max(view.top_row);
        let col_first = (frozen_cols + 1).max(view.left_column);

        // Scrollable rows: visible extent + prefix-sum in one pass
        // `y` tracks the canvas Y of each row's top edge. When y reaches the
        // canvas bottom we record that row as `row_last` (it may be partially
        // visible), push its trailing entry, and stop. This exactly replicates
        // the semantics of the old two-pass pair.
        let mut row_tops: Vec<i32> = Vec::new();
        let mut row_last = row_first;
        let mut y = HEADER_ROW_HEIGHT + frozen_h;
        let mut acc = 0;
        for row in row_first..=LAST_ROW {
            if f64::from(y) >= canvas.h || row == LAST_ROW {
                row_last = row;
                row_tops.push(acc);
                acc += row_height(model, row);
                row_tops.push(acc); // trailing: bottom of row_last
                break;
            }
            row_tops.push(acc);
            let h = row_height(model, row);
            acc += h;
            y += h;
        }

        // Scrollable columns: same merged pattern
        let mut col_lefts: Vec<i32> = Vec::new();
        let mut col_last = col_first;
        let mut x = HEADER_COL_WIDTH + frozen_w;
        acc = 0;
        for col in col_first..=LAST_COLUMN {
            if f64::from(x) >= canvas.w || col == LAST_COLUMN {
                col_last = col;
                col_lefts.push(acc);
                acc += col_width(model, col);
                col_lefts.push(acc);
                break;
            }
            col_lefts.push(acc);
            let w = col_width(model, col);
            acc += w;
            x += w;
        }

        let vis = VisibleCells {
            first: CellRC {
                row: row_first,
                column: col_first,
            },
            last: CellRC {
                row: row_last,
                column: col_last,
            },
        };
        let offsets = PixelOffsets {
            row_start: row_first,
            row_tops,
            col_start: col_first,
            col_lefts,
            frozen_row_tops,
            frozen_col_lefts,
        };

        FrameContext {
            sheet: model.get_selected_sheet(),
            vis,
            offsets,
            frozen,
            top_row: view.top_row,
            left_column: view.left_column,
            selection_range: view.selection,
            canvas_size: canvas,
            theme,
        }
    }

    /// True when the painted geometry is identical to the current model state.
    ///
    /// Checks scroll origin, frozen band counts, sheet, and canvas size — the
    /// inputs that determine `PixelOffsets` and visible-region indices. When
    /// all match, the overlay layer can repaint against this frame without
    /// rebuilding via `FrameContext::current`. Selection is *not* part of this
    /// predicate — refresh it via `refresh_overlay_inputs` after a positive
    /// answer, before painting the overlay.
    pub(crate) fn is_still_valid(&self, model: &dyn CanvasModel, size: CanvasSize) -> bool {
        if size != self.canvas_size {
            return false;
        }
        let view = model.get_selected_view();
        if self.top_row != view.top_row || self.left_column != view.left_column {
            return false;
        }
        let sheet = model.get_selected_sheet();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        frozen_rows == self.frozen.frozen_rows_count()
            && frozen_cols == self.frozen.frozen_cols_count()
            && sheet == self.sheet
    }

    /// Refresh frame fields that the overlay paints from but that are
    /// independent of the prefix-sum tables. Call on the overlay-only fast
    /// path after `is_still_valid` returned true, before painting. Keeps the
    /// "snapshot of what's painted" invariant on `selection_range`: the
    /// orchestrator never reaches into the field directly.
    pub(crate) fn refresh_overlay_inputs(&mut self, model: &dyn CanvasModel) {
        self.selection_range = model.get_selected_view().selection;
    }

    // Pixel <-> cell mapping  (snapshot-only)
    //
    // Every method here reads exclusively from `self.offsets`,
    // `self.frozen`, `self.vis`, and `self.selection_range`. No model
    // access — what the renderer painted is what gets hit-tested.
    //
    // Off-frame inputs are clamped to the painted region (`pixel_to_*`)
    // or rejected with `None` (`cell_rect`, `autofill_handle`).

    #[inline]
    fn row_in_frame(&self, row: i32) -> bool {
        row <= self.frozen.frozen_rows_count()
            || (row >= self.vis.first.row && row <= self.vis.last.row)
    }

    #[inline]
    fn col_in_frame(&self, col: i32) -> bool {
        col <= self.frozen.frozen_cols_count()
            || (col >= self.vis.first.column && col <= self.vis.last.column)
    }

    /// Width of `col` from the snapshot — frozen-band or visible-band.
    #[inline]
    pub(crate) fn col_extent_at(&self, col: i32) -> i32 {
        if col <= self.frozen.frozen_cols_count() {
            self.offsets.frozen_col_extent(col)
        } else {
            self.offsets.col_extent(col)
        }
    }

    /// Height of `row` from the snapshot.
    #[inline]
    pub(crate) fn row_extent_at(&self, row: i32) -> i32 {
        if row <= self.frozen.frozen_rows_count() {
            self.offsets.frozen_row_extent(row)
        } else {
            self.offsets.row_extent(row)
        }
    }

    /// Left-edge X pixel of `col` at this frame's scroll/freeze.
    /// Caller is expected to gate on `col_in_frame`; off-frame yields the
    /// cumulative-table fallback (`0.0`).
    pub(crate) fn col_to_x(&self, col: i32) -> i32 {
        if col <= self.frozen.frozen_cols_count() {
            HEADER_COL_WIDTH + self.offsets.frozen_col_left(col)
        } else {
            self.frozen.offset.x + self.offsets.col_left(col)
        }
    }

    /// Top-edge Y pixel of `row`.
    pub(crate) fn row_to_y(&self, row: i32) -> i32 {
        if row <= self.frozen.frozen_rows_count() {
            HEADER_ROW_HEIGHT + self.offsets.frozen_row_top(row)
        } else {
            self.frozen.offset.y + self.offsets.row_top(row)
        }
    }

    /// 1-based column at canvas X pixel `x`. Clamps to the painted region
    pub(crate) fn pixel_to_col(&self, x: i32) -> i32 {
        let frozen_cols = self.frozen.frozen_cols_count();
        if x < self.frozen.offset.x {
            let rel = x - HEADER_COL_WIDTH;
            for c in 1..=frozen_cols {
                if rel < self.offsets.frozen_col_lefts[c as usize] {
                    return c;
                }
            }
            return frozen_cols.max(1);
        }
        let rel = x - self.frozen.offset.x;
        let count = (self.vis.last.column - self.vis.first.column + 1) as usize;
        for i in 0..count {
            if rel < self.offsets.col_lefts[i + 1] {
                return self.offsets.col_start + i as i32;
            }
        }
        self.vis.last.column
    }

    /// 1-based row at canvas Y pixel `y`. Clamps to the painted region.
    pub(crate) fn pixel_to_row(&self, y: i32) -> i32 {
        let frozen_rows = self.frozen.frozen_rows_count();
        if y < self.frozen.offset.y {
            let rel = y - HEADER_ROW_HEIGHT;
            for r in 1..=frozen_rows {
                if rel < self.offsets.frozen_row_tops[r as usize] {
                    return r;
                }
            }
            return frozen_rows.max(1);
        }
        let rel = y - self.frozen.offset.y;
        let count = (self.vis.last.row - self.vis.first.row + 1) as usize;
        for i in 0..count {
            if rel < self.offsets.row_tops[i + 1] {
                return self.offsets.row_start + i as i32;
            }
        }
        self.vis.last.row
    }

    /// Pixel rect of `(row, col)` if it falls inside this frame's painted
    /// region (frozen bands + visible scrollable area). Returns `None` for
    /// off-screen cells — the snapshot only describes what was painted.
    pub(crate) fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        if !self.row_in_frame(row) || !self.col_in_frame(col) {
            return None;
        }
        Some(PixelRect {
            top_left: Point {
                x: self.col_to_x(col),
                y: self.row_to_y(row),
            },
            width: self.col_extent_at(col),
            height: self.row_extent_at(row),
        })
    }

    /// Bottom-right pixel of the painted selection — the autofill handle
    /// anchor. `None` for full-row/column/sheet selections (trailing index
    /// at the spreadsheet bound) and for selections whose bottom-right is
    /// off-frame. Reads `selection_range` captured at paint time, so the
    /// handle position is locked to what's on screen even if the model's
    /// selection has since moved.
    pub(crate) fn autofill_handle(&self) -> Option<Point> {
        let norm = self.selection_range.normalized();
        let r2 = norm.r2;
        let c2 = norm.c2;
        if r2 >= LAST_ROW || c2 >= LAST_COLUMN {
            return None;
        }
        if !self.row_in_frame(r2) || !self.col_in_frame(c2) {
            return None;
        }
        Some(Point {
            x: self.col_to_x(c2) + self.col_extent_at(c2),
            y: self.row_to_y(r2) + self.row_extent_at(r2),
        })
    }

    /// Visual rect of the autofill handle — the small square stroked over
    /// the selection's bottom-right corner. Top-left sits exactly at
    /// `autofill_handle()` so the handle pokes outside the selection.
    /// Single source of truth: `draw_selection` paints from this rect and
    /// `hit_test` accepts clicks against an inflated copy of it.
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

    /// Column whose RIGHT edge is within `hit_zone` px of `x`, or `None`.
    pub(crate) fn col_boundary_at(&self, x: i32, hit_zone: i32) -> Option<i32> {
        let frozen_cols = self.frozen.frozen_cols_count();
        for c in 1..=frozen_cols {
            let cur_x = HEADER_COL_WIDTH + self.offsets.frozen_col_lefts[c as usize];
            if (cur_x - x).abs() <= hit_zone {
                return Some(c);
            }
        }
        let count = (self.vis.last.column - self.vis.first.column + 1) as usize;
        for i in 0..count {
            let cur_x = self.frozen.offset.x + self.offsets.col_lefts[i + 1];
            if (cur_x - x).abs() <= hit_zone {
                return Some(self.offsets.col_start + i as i32);
            }
            if cur_x > x + hit_zone {
                break;
            }
        }
        None
    }

    /// Row whose BOTTOM edge is within `hit_zone` px of `y`, or `None`.
    pub(crate) fn row_boundary_at(&self, y: i32, hit_zone: i32) -> Option<i32> {
        let frozen_rows = self.frozen.frozen_rows_count();
        for r in 1..=frozen_rows {
            let cur_y = HEADER_ROW_HEIGHT + self.offsets.frozen_row_tops[r as usize];
            if (cur_y - y).abs() <= hit_zone {
                return Some(r);
            }
        }
        let count = (self.vis.last.row - self.vis.first.row + 1) as usize;
        for i in 0..count {
            let cur_y = self.frozen.offset.y + self.offsets.row_tops[i + 1];
            if (cur_y - y).abs() <= hit_zone {
                return Some(self.offsets.row_start + i as i32);
            }
            if cur_y > y + hit_zone {
                break;
            }
        }
        None
    }

    // Hit-test dispatch

    /// Map `(x, y)` to what the user sees against this frame.
    ///
    /// Negative coordinates return `Outside` (off-canvas). Past the right /
    /// bottom edge the trailing visible cell is returned — the canvas
    /// element's own bounds clip the event before it reaches us in practice.
    pub(crate) fn hit_test(&self, x: i32, y: i32) -> HitTest {
        if x < 0 || y < 0 {
            return HitTest::Outside;
        }
        if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
            return HitTest::Corner;
        }
        if y < HEADER_ROW_HEIGHT {
            return HitTest::ColHeader(self.pixel_to_col(x));
        }
        if x < HEADER_COL_WIDTH {
            return HitTest::RowHeader(self.pixel_to_row(y));
        }
        let row = self.pixel_to_row(y);
        let column = self.pixel_to_col(x);
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

    /// Probe for a row/column resize handle near `(x, y)`. Dispatched by
    /// header strip — column boundaries are only hit-tested inside the
    /// column-header strip, and vice versa.
    pub(crate) fn resize_handle_at(&self, x: i32, y: i32, tolerance: i32) -> Option<ResizeTarget> {
        if y < HEADER_ROW_HEIGHT && x > HEADER_COL_WIDTH {
            return self.col_boundary_at(x, tolerance).map(ResizeTarget::Column);
        }
        if x < HEADER_COL_WIDTH && y > HEADER_ROW_HEIGHT {
            return self.row_boundary_at(y, tolerance).map(ResizeTarget::Row);
        }
        None
    }
}
