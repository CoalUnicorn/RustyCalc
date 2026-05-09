use crate::geometry::frame::slot::{ColSlot, RowSlot};
use crate::{
    geometry::{
        constants::{
            AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT,
            HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
        },
        frame::frozen::FrozenRC,
        pixel_rect::PixelRect,
        prim::Point,
    },
    theme::CanvasTheme,
    types::ui::{HitTest, ResizeTarget},
    CanvasModel, CanvasSize, CanvasView, RCRange,
};

pub mod frozen;
pub mod slot;

/// Per-frame geometric snapshot threaded into every render phase AND every
/// hit-test query.
///
/// Built once per tick by `FrameContext::current(model, canvas)` — both the
/// renderer (`paint_if_dirty`) and the input layer (`IronCanvas::hit_test`,
/// `cell_rect`, `resize_handle_at`) read the same snapshot, so what's painted
/// and what gets hit always agree. Bundles the per-axis slot vecs and the
/// resolved frozen-pane geometry so neither phase re-reads them from the
/// model mid-frame.
#[derive(Debug)]
pub(crate) struct FrameContext {
    pub sheet: u32,
    pub frozen: FrozenRC,
    pub frozen_rows: Vec<RowSlot>,
    pub scroll_rows: Vec<RowSlot>,
    pub frozen_cols: Vec<ColSlot>,
    pub scroll_cols: Vec<ColSlot>,
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

impl FrameContext {
    /// Build a per-frame snapshot from the model and canvas size.
    ///
    /// One model-walk per axis populates the four slot vecs (`frozen_rows`,
    /// `scroll_rows`, `frozen_cols`, `scroll_cols`); each scan breaks early at
    /// `canvas.{w,h}` or the sheet bound.
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
        let frozen = FrozenRC::from_model(model);

        let frozen_rows_count = frozen.frozen_rows_count();
        let frozen_cols_count = frozen.frozen_cols_count();

        // Frozen rows: absolute Y starts at the body origin
        // (HEADER_ROW_HEIGHT + HEADER_OFFSET) and grows. Matches
        // `Axis::strip_start()` so headers and the body grid stay aligned.
        let mut frozen_rows = Vec::with_capacity(frozen_rows_count as usize);
        let mut y_cursor = HEADER_ROW_HEIGHT + HEADER_OFFSET;
        for r in 1..=frozen_rows_count {
            let h = row_height(model, r);
            frozen_rows.push(RowSlot {
                row: r,
                top: y_cursor,
                height: h,
            });
            y_cursor += h;
        }

        // Frozen cols
        let mut frozen_cols = Vec::with_capacity(frozen_cols_count as usize);
        let mut x_cursor = HEADER_COL_WIDTH + HEADER_OFFSET;
        for c in 1..=frozen_cols_count {
            let w = col_width(model, c);
            frozen_cols.push(ColSlot {
                col: c,
                left: x_cursor,
                width: w,
            });
            x_cursor += w;
        }

        let row_first = (frozen_rows_count + 1).max(view.top_row);
        let col_first = (frozen_cols_count + 1).max(view.left_column);

        // Scrollable rows: walk until the next slot would start at or past canvas.h.
        // Match existing semantics: the row whose top crosses the bottom edge IS
        // pushed (partial visibility), then we stop.
        let mut scroll_rows: Vec<RowSlot> = Vec::new();
        let mut y_cursor = frozen.offset.y;
        for row in row_first..=LAST_ROW {
            if f64::from(y_cursor) >= canvas.h || row == LAST_ROW {
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

        // Scrollable cols: same shape.
        let mut scroll_cols: Vec<ColSlot> = Vec::new();
        let mut x_cursor = frozen.offset.x;
        for col in col_first..=LAST_COLUMN {
            if f64::from(x_cursor) >= canvas.w || col == LAST_COLUMN {
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

        FrameContext {
            sheet: model.get_selected_sheet(),
            frozen,
            frozen_rows,
            scroll_rows,
            frozen_cols,
            scroll_cols,
            selection_range: view.selection,
            canvas_size: canvas,
            theme: theme.clone(),
        }
    }

    /// True when the painted geometry is identical to the current model state.
    ///
    /// Checks scroll origin, frozen band counts, sheet, and canvas size — the
    /// inputs that determine the slot vecs and visible-region indices. When
    /// all match, the overlay layer can repaint against this frame without
    /// rebuilding via `FrameContext::current`. Selection is *not* part of this
    /// predicate — refresh it via `refresh_overlay_inputs` after a positive
    /// answer, before painting the overlay.
    pub(crate) fn is_still_valid(&self, model: &dyn CanvasModel, size: CanvasSize) -> bool {
        if size != self.canvas_size {
            return false;
        }
        // None ⇒ JS bridge transient; force a full rebuild (which itself
        // tolerates None via the fresh-model fallback in `current`).
        let Some(view) = model.get_selected_view() else { return false };
        let sheet = model.get_selected_sheet();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let want_top = (frozen_rows + 1).max(view.top_row);
        let want_left = (frozen_cols + 1).max(view.left_column);
        if self.top_row() != want_top || self.left_column() != want_left {
            return false;
        }
        frozen_rows == self.frozen.frozen_rows_count()
            && frozen_cols == self.frozen.frozen_cols_count()
            && sheet == self.sheet
    }

    /// Refresh frame fields that the overlay paints from but that are
    /// independent of the slot vecs. Call on the overlay-only fast path
    /// after `is_still_valid` returned true, before painting. Keeps the
    /// "snapshot of what's painted" invariant on `selection_range`: the
    /// orchestrator never reaches into the field directly.
    pub(crate) fn refresh_overlay_inputs(&mut self, model: &dyn CanvasModel) {
        // None ⇒ JS bridge transient; keep the last-known-good selection
        // rather than blanking it out for one frame.
        if let Some(view) = model.get_selected_view() {
            self.selection_range = view.selection;
        }
    }

    #[inline]
    fn row_slot(&self, row: i32) -> Option<&RowSlot> {
        if row <= self.frozen.frozen_rows_count() {
            self.frozen_rows.get((row - 1) as usize)
        } else {
            // Scroll rows are dense from `scroll_rows[0].row` upward.
            let first = self.scroll_rows.first()?.row;
            self.scroll_rows.get((row - first) as usize)
        }
    }

    #[inline]
    fn col_slot(&self, col: i32) -> Option<&ColSlot> {
        if col <= self.frozen.frozen_cols_count() {
            self.frozen_cols.get((col - 1) as usize)
        } else {
            let first = self.scroll_cols.first()?.col;
            self.scroll_cols.get((col - first) as usize)
        }
    }

    /// First scrollable row painted this frame (1-based). Falls back to `1`
    /// when the scrollable band is empty.
    #[inline]
    pub(crate) fn top_row(&self) -> i32 {
        self.scroll_rows.first().map(|s| s.row).unwrap_or(1)
    }

    #[inline]
    pub(crate) fn left_column(&self) -> i32 {
        self.scroll_cols.first().map(|s| s.col).unwrap_or(1)
    }

    #[inline]
    pub(crate) fn last_visible_row(&self) -> i32 {
        self.scroll_rows
            .last()
            .map(|s| s.row)
            .unwrap_or(self.top_row())
    }

    #[inline]
    pub(crate) fn last_visible_col(&self) -> i32 {
        self.scroll_cols
            .last()
            .map(|s| s.col)
            .unwrap_or(self.left_column())
    }

    // Pixel <-> cell mapping  (snapshot-only)
    //
    // Every method here reads exclusively from the slot vecs,
    // `self.frozen`, and `self.selection_range`. No model access —
    // what the renderer painted is what gets hit-tested.
    //
    // Off-frame inputs are clamped to the painted region (`pixel_to_*`)
    // or rejected with `None` (`cell_rect`, `autofill_handle`).

    #[inline]
    fn row_in_frame(&self, row: i32) -> bool {
        row <= self.frozen.frozen_rows_count()
            || (row >= self.top_row() && row <= self.last_visible_row())
    }

    #[inline]
    fn col_in_frame(&self, col: i32) -> bool {
        col <= self.frozen.frozen_cols_count()
            || (col >= self.left_column() && col <= self.last_visible_col())
    }

    /// Width of `col` from the snapshot — frozen-band or visible-band.
    #[inline]
    pub(crate) fn col_extent_at(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.width).unwrap_or(0)
    }

    /// Height of `row` from the snapshot.
    #[inline]
    pub(crate) fn row_extent_at(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.height).unwrap_or(0)
    }

    /// Left-edge X pixel of `col` at this frame's scroll/freeze.
    pub(crate) fn col_to_x(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.left).unwrap_or(0)
    }

    /// Top-edge Y pixel of `row`.
    pub(crate) fn row_to_y(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.top).unwrap_or(0)
    }

    /// 1-based column at canvas X pixel `x`, or `None` if `x` is outside the
    /// painted region (frozen + scrollable bands). The snapshot only describes
    /// what was painted — clicks in the void are surfaced as `Outside`, not as
    /// the trailing visible cell.
    pub(crate) fn pixel_to_col(&self, x: i32) -> Option<i32> {
        for s in &self.frozen_cols {
            if x >= s.left && x < s.right() {
                return Some(s.col);
            }
        }
        for s in &self.scroll_cols {
            if x >= s.left && x < s.right() {
                return Some(s.col);
            }
        }
        None
    }

    pub(crate) fn pixel_to_row(&self, y: i32) -> Option<i32> {
        for s in &self.frozen_rows {
            if y >= s.top && y < s.bottom() {
                return Some(s.row);
            }
        }
        for s in &self.scroll_rows {
            if y >= s.top && y < s.bottom() {
                return Some(s.row);
            }
        }
        None
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
        for s in &self.frozen_cols {
            if (s.right() - x).abs() <= hit_zone {
                return Some(s.col);
            }
        }
        for s in &self.scroll_cols {
            if (s.right() - x).abs() <= hit_zone {
                return Some(s.col);
            }
            if s.right() > x + hit_zone {
                break;
            }
        }
        None
    }

    /// Row whose BOTTOM edge is within `hit_zone` px of `y`, or `None`.
    pub(crate) fn row_boundary_at(&self, y: i32, hit_zone: i32) -> Option<i32> {
        for s in &self.frozen_rows {
            if (s.bottom() - y).abs() <= hit_zone {
                return Some(s.row);
            }
        }
        for s in &self.scroll_rows {
            if (s.bottom() - y).abs() <= hit_zone {
                return Some(s.row);
            }
            if s.bottom() > y + hit_zone {
                break;
            }
        }
        None
    }

    // Hit-test dispatch

    /// Map `(x, y)` to what the user sees against this frame.
    ///
    /// Negative coordinates return `Outside` (off-canvas). Past the right /
    /// bottom edge of the painted region returns `Outside` — the canvas
    /// element's own bounds clip events before they reach us in practice.
    pub(crate) fn hit_test(&self, x: i32, y: i32) -> HitTest {
        if x < 0 || y < 0 {
            return HitTest::Outside;
        }
        if x < HEADER_COL_WIDTH + HEADER_OFFSET && y < HEADER_ROW_HEIGHT + HEADER_OFFSET {
            return HitTest::Corner;
        }
        if y < HEADER_ROW_HEIGHT + HEADER_OFFSET {
            return match self.pixel_to_col(x) {
                Some(c) => HitTest::ColHeader(c),
                None => HitTest::Outside,
            };
        }
        if x < HEADER_COL_WIDTH + HEADER_OFFSET {
            return match self.pixel_to_row(y) {
                Some(r) => HitTest::RowHeader(r),
                None => HitTest::Outside,
            };
        }
        let (Some(row), Some(column)) = (self.pixel_to_row(y), self.pixel_to_col(x)) else {
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

fn row_height(model: &dyn CanvasModel, row: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_row_height(sheet, row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
        .round() as i32
}

fn col_width(model: &dyn CanvasModel, col: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_column_width(sheet, col)
        .unwrap_or(DEFAULT_COL_WIDTH)
        .round() as i32
}
