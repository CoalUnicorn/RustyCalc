use std::ops::RangeInclusive;

use crate::{renderer::cells::PaneCells, CanvasModel, Point, VisibleRegion};

use super::super::geometry::{
    CanvasSize, FrozenRC, HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT,
};
//  Pane rendering

/// Describes one of the four frozen-pane quadrants for `render_pane`.
///
/// Build with a named constructor so the quadrant name appears at the call site:
/// ```text
/// render_pane(model, sheet, &mut texts, PaneRegion::top_left(&frc));
/// render_pane(model, sheet, &mut texts, PaneRegion::bottom_right(&frc, &vis));
/// ```
#[derive(Clone)]
pub struct PaneRegion {
    pub rows: RangeInclusive<i32>,
    pub cols: RangeInclusive<i32>,
    pub origin: Point,
}

/// Outer edge of a cell rect that may be forced to stroke a border because
/// the cell sits on a pane boundary. Only `Right` and `Bottom` are valid -
/// left/top are inner edges resolved against neighbour cells inside the
/// pane.
// #[derive(Copy, Clone, PartialEq, Eq)]
// pub enum OuterEdge {
//     Right,
//     Bottom,
// }

impl PaneRegion {
    /// Frozen rows x frozen cols - top-left quadrant.
    pub(crate) fn top_left(frc: &FrozenRC) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            rows,
            cols,
            origin: Point {
                x: HEADER_COL_WIDTH + HEADER_OFFSET,
                y: HEADER_ROW_HEIGHT + HEADER_OFFSET,
            },
        }
    }

    /// Frozen rows x scrollable cols - top-right quadrant.
    pub(crate) fn top_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        PaneRegion {
            rows,
            cols: vis.first.column..=vis.last.column,
            origin: Point {
                x: frc.offset.x,
                y: HEADER_ROW_HEIGHT + HEADER_OFFSET,
            },
        }
    }

    /// Scrollable rows x frozen cols - bottom-left quadrant.
    pub(crate) fn bottom_left(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            rows: vis.first.row..=vis.last.row,
            cols,
            origin: Point {
                x: HEADER_COL_WIDTH + HEADER_OFFSET,
                y: frc.offset.y,
            },
        }
    }

    /// Scrollable rows x scrollable cols - main area.
    pub(crate) fn bottom_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        PaneRegion {
            rows: vis.first.row..=vis.last.row,
            cols: vis.first.column..=vis.last.column,
            origin: Point {
                x: frc.offset.x,
                y: frc.offset.y,
            },
        }
    }

    /// Outer borders this `(row, col)` must draw because it sits on a pane
    /// boundary. Empty slice for interior cells. Static slices - no
    /// allocation per cell.
    // pub(crate) fn outer_edges_at(&self, row: i32, col: i32) -> &'static [OuterEdge] {
    //     match (col == self.last_col, row == self.last_row) {
    //         (true, true) => &[OuterEdge::Right, OuterEdge::Bottom],
    //         (true, false) => &[OuterEdge::Right],
    //         (false, true) => &[OuterEdge::Bottom],
    //         (false, false) => &[],
    //     }
    // }

    /// Walk every visible cell in this pane, yielding pixel rect + outer
    /// edges per cell. Replaces the open-coded row/col iteration that used
    /// to live in `render_pane` / `render_pane_row`. The caller passes the
    /// canvas size so the walker can early-break past the canvas edge.
    pub(crate) fn cells<'a>(
        &'a self,
        model: &'a dyn CanvasModel,
        canvas: CanvasSize,
    ) -> PaneCells<'a> {
        PaneCells {
            pane: self,
            model,
            sheet: model.get_selected_sheet(),
            canvas,
            current_row: None,
            row_iter: self.rows.clone(),
            row_top: self.origin.y,
            col_iter: self.cols.clone(),
            col_left: self.origin.x,
        }
    }
}
