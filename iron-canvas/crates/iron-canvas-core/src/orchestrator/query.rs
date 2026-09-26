use crate::chrome::hit::{HitTest, ResizeTarget};
use crate::decoration::selection::SelectionLayer;
use crate::geometry::CanvasMetrics;
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::model::autofit::{AutoFitError, fit_height};
use crate::painter::BlitPainter;
use crate::surface::Surface;
use crate::theme::CanvasTheme;

use super::Orchestrator;

/// Smallest band origin along one axis that shows `target` in full, given the
/// axis's frozen count, its scrollable `extent` in pixels, and where the band
/// currently starts.
///
/// The backward walk is bounded by how many slots fit in `extent`, so a jump of
/// 100k rows costs the same as a jump of one. Returning `current` unchanged is
/// the "already visible / nothing to do" answer. `None` when `measure`
/// reports a transient bridge failure — a scroll target derived from
/// fabricated heights is not trustworthy.
fn origin_showing(
    target: i32,
    current: i32,
    frozen: i32,
    extent: i32,
    mut measure: impl FnMut(i32) -> Option<i32>,
) -> Option<i32> {
    // A collapsed axis scrolls nowhere, and a frozen target is always painted.
    if extent <= 0 || target <= frozen {
        return Some(current);
    }
    if target < current {
        return Some(target); // scrolled past it — flush against the near edge
    }

    // Walk back from the target while the run still fits. `smallest` is then
    // the earliest origin that shows the target in full, so any origin at or
    // after it also shows it — hence the `max` rather than a second forward sum.
    // The loop floor also keeps `smallest` out of the frozen run, so the result
    // is a legal origin without clamping `current` on the way in.
    let mut smallest = target;
    let mut run = measure(target)?;
    while smallest > frozen + 1 {
        let previous = measure(smallest - 1)?;
        if i64::from(run) + i64::from(previous) > i64::from(extent) {
            break;
        }
        smallest -= 1;
        run += previous;
    }
    Some(current.max(smallest))
}

impl<S> Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
    pub fn canvas_size(&self) -> CanvasSize {
        self.metrics.unwrap_or_else(CanvasMetrics::unresized).size()
    }

    /// Validated canvas metrics for the committed configuration. Before the
    /// first `resize` this is the zero-size, DPR-1.0 default — the same pair
    /// the renderer assumed before the type carried an invariant.
    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics.unwrap_or_else(CanvasMetrics::unresized)
    }

    pub fn theme(&self) -> &CanvasTheme {
        &self.theme
    }

    pub fn selection(&self) -> &SelectionLayer {
        self.decos.selection()
    }

    /// Surface introspection — direct access to the grid surface for
    /// callers that read or drive it outside the paint pipeline. Two
    /// consumer classes use it: this crate's recorder integration tests
    /// (inspecting emitted `DrawOp`s) and `iron-canvas-web`'s `dev-tools`
    /// recording/playback. Gated behind `surface-introspection` so the
    /// prod build doesn't carry the symbol.
    #[cfg(feature = "surface-introspection")]
    pub fn grid_surface(&self) -> &S {
        &self.grid.surface
    }

    /// Overlay-surface counterpart to [`Self::grid_surface`]; same
    /// `surface-introspection` gate and the same two consumer classes.
    #[cfg(feature = "surface-introspection")]
    pub fn overlay_surface(&self) -> &S {
        &self.overlay.surface
    }

    // Query API. All queries resolve against `last_frame`, the snapshot
    // emitted by the most recent `render_pending`. Before the first paint
    // `last_frame` is `None` and every query returns its absent variant.

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        let Some(frame) = self.last_frame.as_ref() else {
            return HitTest::Outside;
        };
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        // No live selection -> pass a zero range; the decoration layers that
        // consult `sel` (autofill, formula-refs) treat it as "no anchor"
        // and naturally fall through to the frame's pure cell hit-test.
        let sel = self.decos.selection().selection_range.unwrap_or_default();
        // Custom band first — front-to-back is reverse insertion order,
        // mirroring its paint position above every built-in.
        for (_, layer) in self.decos.custom_layers().iter().rev() {
            if let Some(hit) = layer.hit_test(frame, sel, xi, yi) {
                return hit;
            }
        }
        for layer in self.decos.hit_order() {
            if let Some(hit) = layer.hit_test(frame, sel, xi, yi) {
                return hit;
            }
        }
        frame.hit_test(xi, yi)
    }

    /// Resolve a pixel coordinate to a cell (row, column), bypassing every
    /// decoration layer. The layer-aware `hit_test` is the right tool for
    /// pointer events that start interactions (mousedown), but a drag
    /// already in flight needs the underlying cell *regardless* of which
    /// overlay rectangle the cursor happens to be over — otherwise an
    /// overlay (e.g. `FormulaRefsLayer`) shadows its own cell and the host
    /// can't read pointer motion that re-enters the overlay's bounds.
    /// Returns `None` before the first paint or when the cursor falls in
    /// chrome / off-grid.
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        let frame = self.last_frame.as_ref()?;
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        let row = frame.pane_set.rows.pixel_to_id(yi)?;
        let col = frame.pane_set.cols.pixel_to_id(xi)?;
        Some((row, col))
    }

    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.last_frame.as_ref()?.resize_handle_at(
            x.round() as i32,
            y.round() as i32,
            tolerance.round() as i32,
        )
    }

    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.last_frame.as_ref()?.cell_rect(row, column)
    }

    /// Canvas-space rect of the scrollable pane — everything past the frozen
    /// bands, running to the canvas edge.
    ///
    /// Edge-triggered host behaviour (autoscroll while dragging a selection)
    /// must measure against this, not against the canvas: the near edges sit
    /// `frozen_offset` in from the origin on each axis, which is header
    /// thickness on an unfrozen sheet but header + frozen band + separator
    /// once panes are frozen. `None` before the first paint.
    /// Frozen bands wider or taller than the canvas leave no scrollable
    /// extent at all; the rect collapses to zero rather than going negative,
    /// and callers must treat a zero extent as "nothing scrolls on this axis".
    pub fn scroll_pane_rect(&self) -> Option<PixelRect> {
        let frame = self.last_frame.as_ref()?;
        let top_left = Point {
            x: frame.pane_set.cols.frozen_offset,
            y: frame.pane_set.rows.frozen_offset,
        };
        let (canvas_w, canvas_h) = frame.canvas_size().to_logical_extent();
        // The frame's own canvas size, not `self.metrics()` — a resize between
        // the last paint and this query must not be mixed into a snapshot answer.
        Some(PixelRect {
            top_left,
            width: (canvas_w - top_left.x).max(0),
            height: (canvas_h - top_left.y).max(0),
        })
    }

    /// The scroll origin the renderer will actually honour for the model's
    /// current view — `scroll_first` applied to both axes.
    ///
    /// A scroll band never starts inside the frozen run, but nothing stops a
    /// model's `top_row` from sitting there (freezing panes does not move it).
    /// The renderer clamps silently, so the model can hold a value that
    /// disagrees with every painted pixel. Hosts write this back *before* any
    /// navigation that computes from `top_row` — page up/down derives its new
    /// selection from it, so a correction afterwards arrives too late.
    ///
    /// Reads the live model rather than the painted frame on purpose: a scroll
    /// made since the last paint is legitimate and must survive the sync.
    /// `None` when there is no model or no view.
    pub fn legal_scroll_origin(&self) -> Option<(i32, i32)> {
        let model = self.model.as_deref()?;
        let view = model.get_selected_view()?;
        let frozen_rows = model.get_frozen_rows_count(view.sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(view.sheet).unwrap_or(0);
        Some((
            crate::geometry::slot::scroll_first(frozen_rows, view.top_row),
            crate::geometry::slot::scroll_first(frozen_cols, view.left_column),
        ))
    }

    /// Minimal `(top_row, left_column)` that brings `(row, column)` fully
    /// inside the scroll pane, or `None` when it already is (or when there is
    /// no painted frame or model to measure against).
    ///
    /// Answers from painted geometry, so it accounts for the frozen bands,
    /// measured header thickness, hidden rows and a partial trailing row —
    /// none of which the model's `window_width`/`window_height` arithmetic can
    /// see. A target inside a frozen band never scrolls its axis; a target
    /// taller or wider than the pane aligns to the pane's near edge.
    pub fn scroll_to_show(&self, row: i32, column: i32) -> Option<(i32, i32)> {
        let frame = self.last_frame.as_ref()?;
        let model = self.model.as_deref()?;
        let view = model.get_selected_view()?;
        let pane = self.scroll_pane_rect()?;

        let top = origin_showing(
            row,
            view.top_row,
            frame.pane_set.rows.frozen_count(),
            pane.height,
            |id| crate::geometry::slot::row_height(model, view.sheet, id).extent(),
        )?;
        let left = origin_showing(
            column,
            view.left_column,
            frame.pane_set.cols.frozen_count(),
            pane.width,
            |id| crate::geometry::slot::col_width(model, view.sheet, id).extent(),
        )?;
        ((top, left) != (view.top_row, view.left_column)).then_some((top, left))
    }

    /// Auto-fit width for `col`: widest formatted value across the
    /// `[first_row, last_row]` used-row span, plus padding.
    ///
    /// `Ok(None)` when no scanned cell in `col` has text — nothing to fit to.
    /// `Err(AutoFitError)` when there is no model, or a model read fails: a
    /// host must not treat a failed read as "no content". Pure measurement —
    /// the consumer applies the returned extent.
    pub fn fit_column_width(
        &self,
        col: i32,
        first_row: i32,
        last_row: i32,
    ) -> Result<Option<f64>, AutoFitError> {
        let model = self.model.as_deref().ok_or(AutoFitError::NoModel)?;
        let metrics = self.grid.surface.painter();
        crate::model::autofit::fit_width(model, metrics, col, first_row, last_row)
    }

    /// Auto-fit height for `row`: tallest font across the `[first_col,
    /// last_col]` used-column span, plus padding. Same
    /// [`AutoFitError`] semantics as `fit_column_width`.
    pub fn fit_row_height(
        &self,
        row: i32,
        first_col: i32,
        last_col: i32,
    ) -> Result<Option<f64>, AutoFitError> {
        let model = self.model.as_deref().ok_or(AutoFitError::NoModel)?;
        let metrics = self.grid.surface.painter();
        fit_height(model, metrics, row, first_col, last_col)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.last_frame
            .as_ref()?
            .autofill_handle(self.decos.selection().selection_range?)
    }
}

#[cfg(test)]
mod tests {
    use super::origin_showing;

    /// Uniform rows, so `extent / 20` is how many fit and every expectation
    /// below is arithmetic a reader can redo in their head.
    fn rows_20(_id: i32) -> Option<i32> {
        Some(20)
    }

    #[test]
    fn origin_showing_does_not_overflow_on_large_valid_extents() {
        assert_eq!(origin_showing(3, 1, 0, 100, |_| Some(i32::MAX)), Some(3));
    }

    #[test]
    fn stays_put_when_there_is_nothing_to_scroll() {
        // A collapsed axis has nowhere to put the target.
        assert_eq!(origin_showing(50, 7, 0, 0, rows_20), Some(7));
        assert_eq!(origin_showing(50, 7, 0, -100, rows_20), Some(7));
        // A frozen target is painted whatever the scrollable band shows.
        assert_eq!(origin_showing(2, 7, 3, 500, rows_20), Some(7));
    }

    #[test]
    fn flushes_against_the_near_edge_when_scrolled_past() {
        assert_eq!(origin_showing(5, 20, 0, 500, rows_20), Some(5));
    }

    /// The trailing `max` earns its keep here: the walk finds row 8 as the
    /// earliest origin that fits, but the band already sits at 10 and already
    /// shows row 12 — scrolling back to 8 would be visible, pointless motion.
    #[test]
    fn leaves_an_already_visible_target_alone() {
        assert_eq!(origin_showing(12, 10, 0, 100, rows_20), Some(10));
    }

    /// A target past the far edge pulls the origin forward — to the *smallest*
    /// origin that still shows the target whole, not merely to the target.
    #[test]
    fn walks_back_to_the_smallest_origin_that_shows_the_target() {
        // Five 20 px rows fill 100, so 26..=30 is the earliest band showing 30.
        // An implementation that stopped after one step would answer 29.
        assert_eq!(origin_showing(30, 2, 0, 100, rows_20), Some(26));
    }

    /// Rows of 8/19/30/41/52 px on a five-row cycle, so a walk that assumed a
    /// uniform height cannot land on the right origin by symmetry.
    fn rows_uneven(id: i32) -> Option<i32> {
        Some(8 + id.rem_euclid(5) * 11)
    }

    /// The walk accumulates real heights: rows 30 (8 px) and 29 (52 px) fill 60
    /// of the 100 px band, and taking row 28 (41 px) too would overflow it.
    #[test]
    fn walk_sums_actual_row_heights_rather_than_assuming_uniform_rows() {
        assert_eq!(origin_showing(30, 2, 0, 100, rows_uneven), Some(29));
    }
}
