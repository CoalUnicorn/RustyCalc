use crate::{
    geometry::{
        constants::{FROZEN_SEP, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT},
        prim::Point,
        utils::{col_width, row_height},
    },
    CanvasModel,
};

/// Frozen rows and columns grouped with their pixel origin.
///
/// `rows` / `cols` are counts: today every freeze is anchored at the top-left
/// so `1..=rows` / `1..=cols` is the full extent. A future named-range-anchored
/// freeze would replace these counts with a richer shape.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenRC {
    pub rows: i32,
    pub cols: i32,
    pub offset: Point,
}

impl FrozenRC {
    /// Read frozen geometry from the currently-selected sheet on `model`.
    pub fn from_model(model: &dyn CanvasModel) -> Self {
        let sheet = model.get_selected_sheet();
        let rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let h: f64 = (1..=rows).map(|r| row_height(model, r)).sum();
        let w: f64 = (1..=cols).map(|c| col_width(model, c)).sum();
        FrozenRC {
            rows,
            cols,
            offset: Point {
                x: HEADER_COL_WIDTH + w + if cols > 0 { FROZEN_SEP } else { 0.0 },
                y: HEADER_ROW_HEIGHT + h + if rows > 0 { FROZEN_SEP } else { 0.0 },
            },
        }
    }

    #[inline]
    pub fn frozen_rows_count(&self) -> i32 {
        self.rows
    }

    #[inline]
    pub fn frozen_cols_count(&self) -> i32 {
        self.cols
    }
}
