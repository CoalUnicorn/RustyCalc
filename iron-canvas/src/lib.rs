//! The spreadsheet's `<canvas>` surface.
//!
//! # The Concept - everything is a rectangle or a line
//!
//! Every visible artifact in the grid is one of two primitives:
//!
//! - [`PixelRect`] - cells, headers, corner box, selection fill, autofill
//!   handle, point-mode tint, clipboard marching-ants region, text clip.
//! - [`Line`] - border edges, frozen-pane separators, underline,
//!   strikethrough.
//!
//! No curves, no arbitrary paths. Border resolution becomes "pick a `Line`
//! and a color"; pane layout becomes "four `PixelRect`s side by side";
//! overlays compose by stacking `PixelRect`s.
//!
//! That constraint keeps the paint layer small: `rect_fill`, `rect_stroke`,
//! `rect_dashed`, `stroke_line`, `with_clip`. New visuals reduce to those
//! helpers or they don't ship.
//!
//! # Submodules
//!
//! - [`geometry`] - rect/line types and pixel↔cell coordinate math.
//! - [`types`] - renderer-internal shapes (panes, text layout, visible
//!   region) plus public overlay types.
//! - [`renderer`] - the four-phase render pipeline. See its module doc
//!   for the full walk-through.

pub mod geometry;
mod layer;
pub mod model;
mod orchestrator;
pub mod renderer;
pub mod style;
pub mod theme;
pub mod types;
pub mod wasm;

#[cfg(test)]
mod test;

pub use geometry::{
    col_name, col_width, row_height, CanvasSize, CellRC, FrozenRC, Line, PixelRect, Point, Span,
    VisibleCells, AUTOFILL_HANDLE_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP,
    HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
};

pub use orchestrator::IronCanvas;
pub use renderer::CanvasRenderer;
pub use types::RenderOverlays;

/// What the user sees at a given canvas point, against the last painted frame.
///
/// All variants carry **model coordinates** (row/column indices on the active
/// sheet) — the canvas-internal pixel mapping does not leak to callers. The
/// active sheet is whatever `IronCanvas` is reflecting at the time of the
/// query, so it is implicit and not encoded into the variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTest {
    Cell {
        row: i32,
        column: i32,
    },
    RowHeader(i32),
    ColHeader(i32),
    Corner,
    /// Cursor is on the autofill handle. Carries the cell under the cursor
    /// because callers always need both — the variant says "begin autofill",
    /// the fields say "drag-target starts here".
    AutofillHandle {
        row: i32,
        column: i32,
    },
    Outside,
}

/// A row or column boundary the cursor is currently within tolerance of.
///
/// Returned by `IronCanvas::resize_handle_at` for cursor-style and
/// drag-start decisions. Holds the index of the row/column **whose trailing
/// edge** the cursor is near (i.e. dragging right enlarges that row/column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeTarget {
    Column(i32),
    Row(i32),
}

// CanvasModel - read-only worksheet surface the renderer consumes
//
// Path A bridge: the renderer's eventual parameter type. Replaces direct
// `&UserModel` so the IronCalc webapp can plug in a JS-backed adapter that
// batch-fetches cell data into a frame snapshot before each `render_sheet()`.
//
// Method signatures match `ironcalc_base::UserModel` verbatim (T1-A) in a
// single trait (T2-A). RustyCalc gets a free `impl CanvasModel for UserModel`
// below.
//
// `SelectedView` is mirrored locally because `ironcalc_base` re-exports the
// upstream struct only under `#[cfg(test)]`. When that gate goes away
// upstream, this mirror and the field-copy in `get_selected_view` below can
// be deleted in favour of `pub use ironcalc_base::SelectedView;` - no other
// consumer needs to change.

use ironcalc_base::types::{CellType, Style};
use ironcalc_base::UserModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedView {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
    pub range: [i32; 4],
    pub top_row: i32,
    pub left_column: i32,
}

pub trait CanvasModel {
    fn get_selected_sheet(&self) -> u32;
    fn get_selected_view(&self) -> SelectedView;
    fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String>;
    fn get_show_grid_lines(&self, sheet: u32) -> Result<bool, String>;
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32)
        -> Result<String, String>;
}

impl<'a> CanvasModel for UserModel<'a> {
    fn get_selected_sheet(&self) -> u32 {
        UserModel::get_selected_sheet(self)
    }
    fn get_selected_view(&self) -> SelectedView {
        let v = UserModel::get_selected_view(self);
        SelectedView {
            sheet: v.sheet,
            row: v.row,
            column: v.column,
            range: v.range,
            top_row: v.top_row,
            left_column: v.left_column,
        }
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String> {
        UserModel::get_frozen_rows_count(self, sheet)
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String> {
        UserModel::get_frozen_columns_count(self, sheet)
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String> {
        UserModel::get_row_height(self, sheet, row)
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String> {
        UserModel::get_column_width(self, sheet, column)
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Result<bool, String> {
        UserModel::get_show_grid_lines(self, sheet)
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String> {
        UserModel::get_cell_style(self, sheet, row, column)
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String> {
        UserModel::get_cell_type(self, sheet, row, column)
    }
    fn get_formatted_cell_value(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<String, String> {
        UserModel::get_formatted_cell_value(self, sheet, row, column)
    }
}
