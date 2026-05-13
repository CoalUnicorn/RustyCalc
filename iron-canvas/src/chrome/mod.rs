//! Per-frame snapshot of painted chrome geometry. The renderer and every
//! `IronCanvas` query read the same `Chrome`, so painted pixels and hit
//! zones cannot disagree.
//!
//! Pure-axis walks live on `PaneSet`; `Chrome` composes them whenever a
//! query spans both axes. See `ARCHITECTURE.md` for the build phases
//! (A–E) and the `is_still_valid` cache rules.

use crate::geometry::slot::{
    boundary_at, col_width, fill_axis, last_visible_id, pixel_to_id, row_height, scroll_first,
    slot_at, top_id, AxisSlot, ColSlot, RowSlot,
};
use crate::geometry::{
    constants::{
        AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_OFFSET,
        HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::Point,
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, RCRange};

pub(crate) mod pane_region;

pub(crate) use pane_region::PaneRegion;

/// Approx pixel width per digit at the bold 12px Inter header font.
/// Pessimistic enough that no row label clips inside the strip.
const APPROX_DIGIT_WIDTH_PX: i32 = 8;
/// Padding either side of the row-label inside the header strip.
const HEADER_LABEL_PAD_PX: i32 = 4;

#[derive(Debug)]
pub(crate) struct Chrome {
    pub sheet: u32,
    pub pane_set: PaneSet,
    /// Measured per frame from the widest visible row label.
    pub row_header_thickness: i32,
    pub col_header_thickness: i32,
    /// Top-left of the cell area; single source of truth for hit-test
    /// and viewport math.
    pub cell_origin: Point,
    /// Selection snapshot at paint time, raw `[r1, c1, r2, c2]` from
    /// `SelectedView.range`. Pins `autofill_handle` to the painted
    /// selection even if the model's selection has since moved.
    ///
    /// Invariant: no paint or hit-test code reads
    /// `model.get_selected_view().selection`; every consumer goes through
    /// this field, so painted and queried geometry stay in lockstep.
    pub selection_range: RCRange,
    /// Canvas size at build time. `is_still_valid` reads this to detect
    /// a resize.
    pub canvas_size: CanvasSize,
    /// Theme this frame was painted with. The renderer reads `frame.theme`
    /// directly; `IronCanvas::set_theme` marks both layers dirty on change,
    /// so the overlay-only fast path never paints against a stale theme.
    pub theme: CanvasTheme,
}

#[derive(Debug)]
pub(crate) struct PaneSet {
    pub frozen_rows: Vec<RowSlot>,
    pub scroll_rows: Vec<RowSlot>,
    pub frozen_offset_y: i32,
    pub frozen_cols: Vec<ColSlot>,
    pub scroll_cols: Vec<ColSlot>,
    pub frozen_offset_x: i32,
}

/// Cleared slot Vecs handed from the outgoing `Chrome` into the next one,
/// so the backing allocations cross the frame boundary and steady-state
/// rebuilds only allocate when row/column count outgrows capacity.
#[derive(Default)]
pub(crate) struct RecycledSlots {
    pub(crate) frozen_rows: Vec<RowSlot>,
    pub(crate) scroll_rows: Vec<RowSlot>,
    pub(crate) frozen_cols: Vec<ColSlot>,
    pub(crate) scroll_cols: Vec<ColSlot>,
}

impl RecycledSlots {
    fn from_pane_set(pane_set: PaneSet) -> Self {
        let PaneSet {
            mut frozen_rows,
            mut scroll_rows,
            mut frozen_cols,
            mut scroll_cols,
            ..
        } = pane_set;
        frozen_rows.clear();
        scroll_rows.clear();
        frozen_cols.clear();
        scroll_cols.clear();
        Self {
            frozen_rows,
            scroll_rows,
            frozen_cols,
            scroll_cols,
        }
    }
}

impl PaneSet {
    /// Fresh `PaneSet` reusing the previous frame's drained slot Vecs.
    /// `frozen_offset_*` are filled in by `fill_rows` / `fill_cols`.
    pub(crate) fn with_recycled(recycled: RecycledSlots) -> Self {
        PaneSet {
            frozen_rows: recycled.frozen_rows,
            scroll_rows: recycled.scroll_rows,
            frozen_offset_y: 0,
            frozen_cols: recycled.frozen_cols,
            scroll_cols: recycled.scroll_cols,
            frozen_offset_x: 0,
        }
    }

    /// Populate `frozen_rows`, `scroll_rows`, and `frozen_offset_y`
    /// (Phase B of `Chrome::build`; see `ARCHITECTURE.md`). Runs before
    /// the row-label measurement, so it does not depend on
    /// `row_header_thickness`. Reads `FROZEN_SEP` directly because the
    /// gap between frozen and scroll bands is the row axis's concern.
    pub(crate) fn fill_rows(
        &mut self,
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_y: i32,
        view_top_row: i32,
        canvas_h: f64,
    ) {
        self.frozen_rows.reserve(frozen_count as usize);
        let after_frozen = fill_axis(
            &mut self.frozen_rows,
            1..=frozen_count,
            origin_y,
            i32::MAX,
            |r| row_height(model, r),
        );
        self.frozen_offset_y = after_frozen + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let _ = fill_axis(
            &mut self.scroll_rows,
            scroll_first(frozen_count, view_top_row)..=LAST_ROW,
            self.frozen_offset_y,
            canvas_h.ceil() as i32,
            |r| row_height(model, r),
        );
    }

    /// Column-axis mirror of `fill_rows`. Runs as Phase D, using the
    /// cell-area X origin that already folds in the measured
    /// `row_header_thickness`.
    pub(crate) fn fill_cols(
        &mut self,
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_x: i32,
        view_left_column: i32,
        canvas_w: f64,
    ) {
        self.frozen_cols.reserve(frozen_count as usize);
        let after_frozen = fill_axis(
            &mut self.frozen_cols,
            1..=frozen_count,
            origin_x,
            i32::MAX,
            |c| col_width(model, c),
        );
        self.frozen_offset_x = after_frozen + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let _ = fill_axis(
            &mut self.scroll_cols,
            scroll_first(frozen_count, view_left_column)..=LAST_COLUMN,
            self.frozen_offset_x,
            canvas_w.ceil() as i32,
            |c| col_width(model, c),
        );
    }

    #[inline]
    pub(crate) fn frozen_rows_count(&self) -> i32 {
        self.frozen_rows.len() as i32
    }

    #[inline]
    pub(crate) fn frozen_cols_count(&self) -> i32 {
        self.frozen_cols.len() as i32
    }

    #[inline]
    fn row_slot(&self, row: i32) -> Option<&RowSlot> {
        slot_at(&self.frozen_rows, &self.scroll_rows, row)
    }

    #[inline]
    fn col_slot(&self, col: i32) -> Option<&ColSlot> {
        slot_at(&self.frozen_cols, &self.scroll_cols, col)
    }

    #[inline]
    pub(crate) fn top_row(&self) -> i32 {
        top_id(&self.scroll_rows)
    }

    #[inline]
    pub(crate) fn left_column(&self) -> i32 {
        top_id(&self.scroll_cols)
    }

    #[inline]
    pub(crate) fn last_visible_row(&self) -> i32 {
        last_visible_id(&self.scroll_rows)
    }

    #[inline]
    pub(crate) fn last_visible_col(&self) -> i32 {
        last_visible_id(&self.scroll_cols)
    }

    #[inline]
    pub(crate) fn row_in_frame(&self, row: i32) -> bool {
        self.row_slot(row).is_some()
    }

    #[inline]
    pub(crate) fn col_in_frame(&self, col: i32) -> bool {
        self.col_slot(col).is_some()
    }

    #[inline]
    pub(crate) fn row_extent_at(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.extent()).unwrap_or(0)
    }

    #[inline]
    pub(crate) fn col_extent_at(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.extent()).unwrap_or(0)
    }

    pub(crate) fn row_to_y(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.start()).unwrap_or(0)
    }

    pub(crate) fn col_to_x(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.start()).unwrap_or(0)
    }

    pub(crate) fn pixel_to_row(&self, y: i32) -> Option<i32> {
        pixel_to_id(&self.frozen_rows, &self.scroll_rows, y)
    }

    pub(crate) fn pixel_to_col(&self, x: i32) -> Option<i32> {
        pixel_to_id(&self.frozen_cols, &self.scroll_cols, x)
    }

    pub(crate) fn row_boundary_at(&self, y: i32, hit_zone: i32) -> Option<i32> {
        boundary_at(&self.frozen_rows, &self.scroll_rows, y, hit_zone)
    }

    pub(crate) fn col_boundary_at(&self, x: i32, hit_zone: i32) -> Option<i32> {
        boundary_at(&self.frozen_cols, &self.scroll_cols, x, hit_zone)
    }
}

/// Decimal digit count, clamped to `≥ 1` so a zero input still reserves a slot.
fn digit_count(n: i32) -> i32 {
    let mut n = n.max(1);
    let mut d = 0;
    while n > 0 {
        d += 1;
        n /= 10;
    }
    d
}

/// Pixel width the row-header strip needs to fit the widest visible row
/// label. Uses a pessimistic char-count approximation to avoid threading
/// `TextMetrics` (and thus a painter dependency) into `Chrome::build`.
/// Floored at `HEADER_COL_WIDTH` so 3-digit labels never shrink the strip.
pub(crate) fn measure_row_header_width(max_visible_row: i32) -> i32 {
    let digits = digit_count(max_visible_row);
    let approx = digits * APPROX_DIGIT_WIDTH_PX + 2 * HEADER_LABEL_PAD_PX;
    approx.max(HEADER_COL_WIDTH)
}

impl Chrome {
    /// Build the next-frame `Chrome`. When `prev` is `Some`, the outgoing
    /// frame's slot Vec allocations are recycled so steady-state repaints
    /// don't reallocate the four pane-set buffers. `prev == None` is the
    /// first-frame path. See `ARCHITECTURE.md` for the A–E build phases.
    pub(crate) fn next_frame(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
    ) -> Self {
        let recycled = prev
            .map(|c| RecycledSlots::from_pane_set(c.pane_set))
            .unwrap_or_default();
        Self::build(model, canvas, theme, recycled)
    }

    fn build(
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
        recycled: RecycledSlots,
    ) -> Self {
        // None ⇒ JS bridge transient (threw or shape malformed). Fall through
        // with the fresh-model default so the frame still builds; next animation
        // frame re-queries.
        let (top_row, left_column, selection) = match model.get_selected_view() {
            Some(v) => (v.top_row, v.left_column, v.selection),
            None => (
                1,
                1,
                RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 1,
                    c2: 1,
                },
            ),
        };
        let sheet = model.get_selected_sheet();

        // Phase A — frozen counts only.
        let frozen_row_count = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_col_count = model.get_frozen_columns_count(sheet).unwrap_or(0);

        let mut pane_set = PaneSet::with_recycled(recycled);

        // Phase B — row walk.
        let origin_y = HEADER_ROW_HEIGHT + HEADER_OFFSET;
        pane_set.fill_rows(model, frozen_row_count, origin_y, top_row, canvas.h);

        // Phase C — measure row_header_thickness from the last visible row label.
        let last_visible_row = pane_set
            .scroll_rows
            .last()
            .map(|s| s.row)
            .unwrap_or((frozen_row_count + 1).max(top_row));
        let row_header_thickness = measure_row_header_width(last_visible_row);

        // Phase D — col walk uses the measured width to anchor `origin_x`.
        let origin_x = row_header_thickness + HEADER_OFFSET;
        pane_set.fill_cols(model, frozen_col_count, origin_x, left_column, canvas.w);

        // Phase E — assemble. `cell_origin` reuses the locals from B/D so
        // there's a single source of truth for the cell-area top-left.
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
            selection_range: selection,
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
        let want_top = scroll_first(frozen_rows, view.top_row);
        let want_left = scroll_first(frozen_cols, view.left_column);
        if self.pane_set.top_row() != want_top || self.pane_set.left_column() != want_left {
            return false;
        }
        frozen_rows == self.pane_set.frozen_rows_count()
            && frozen_cols == self.pane_set.frozen_cols_count()
            && sheet == self.sheet
    }

    /// Refresh overlay-only fields (independent of the slot vecs). Call on
    /// the overlay-only fast path after `is_still_valid` returns true.
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

    /// Map a sheet-coordinate range to canvas pixel bounds, clamping
    /// oversized selections to the canvas edge. `None` when the range
    /// lies entirely outside the drawable fold. Pure `Chrome` math, no
    /// model access.
    pub(crate) fn range_rect(&self, range: RCRange) -> Option<PixelRect> {
        let p = &self.pane_set;
        let frozen_rows = p.frozen_rows_count();
        let frozen_cols = p.frozen_cols_count();

        if !self.range_intersects_fold(range, frozen_rows, frozen_cols) {
            return None;
        }

        let x = p.col_to_x(range.c1);
        let y = p.row_to_y(range.r1);
        let right = if range.c2 > p.last_visible_col() && range.c2 > frozen_cols {
            self.canvas_size.w as i32
        } else {
            p.col_to_x(range.c2) + p.col_extent_at(range.c2)
        };
        let bottom = if range.r2 > p.last_visible_row() && range.r2 > frozen_rows {
            self.canvas_size.h as i32
        } else {
            p.row_to_y(range.r2) + p.row_extent_at(range.r2)
        };
        Some(PixelRect {
            top_left: Point { x, y },
            width: right - x,
            height: bottom - y,
        })
    }

    /// True if `range` overlaps the drawable fold (scrollable viewport
    /// plus the frozen bands). Guards `range_rect`'s slot lookups against
    /// off-screen refs like `=BB3` when column BB is not visible.
    fn range_intersects_fold(&self, range: RCRange, frozen_rows: i32, frozen_cols: i32) -> bool {
        let p = &self.pane_set;
        if range.c1 > p.last_visible_col() && range.c1 > frozen_cols {
            return false;
        }
        if range.r1 > p.last_visible_row() && range.r1 > frozen_rows {
            return false;
        }
        if range.c2 < p.left_column() && range.c2 > frozen_cols {
            return false;
        }
        if range.r2 < p.top_row() && range.r2 > frozen_rows {
            return false;
        }
        true
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

    pub(crate) fn autofill_handle_rect(&self) -> Option<PixelRect> {
        let p = self.autofill_handle()?;
        Some(PixelRect {
            top_left: Point {
                x: p.x - AUTOFILL_HANDLE_PX,
                y: p.y - AUTOFILL_HANDLE_PX,
            },
            width: AUTOFILL_HANDLE_PX,
            height: AUTOFILL_HANDLE_PX,
        })
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
        if let Some(h) = self.autofill_handle_rect() {
            let pad = AUTOFILL_HIT_PAD_PX;
            if x >= h.top_left.x - pad
                && x <= h.right() + pad
                && y >= h.top_left.y - pad
                && y <= h.bottom() + pad
            {
                return HitTest::AutofillHandle { row, column };
            }
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
