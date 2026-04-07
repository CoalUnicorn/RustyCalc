use ironcalc_base::{expressions::types::Area, UserModel};

use crate::state::EditingCell;

// SheetArea
#[derive(Clone, PartialEq)]
pub struct SheetArea {
    pub sheet: u32,
    pub area: CellArea,
}

// CellArea

/// An axis-aligned cell range in ironcalc 1-based sheet coordinates.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CellArea {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

impl CellArea {
    pub fn height(self) -> usize {
        (self.r2 - self.r1 + 1) as usize
    }

    pub fn width(self) -> usize {
        (self.c2 - self.c1 + 1) as usize
    }

    pub fn rows(self) -> std::ops::Range<i32> {
        (self.r1..self.r2).into_iter()
    }
    pub fn columns(self) -> std::ops::Range<i32> {
        (self.c1..self.c2).into_iter()
    }

    pub fn from_cell(row: i32, col: i32) -> Self {
        CellArea {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        }
    }

    /// Return a new rect with the trailing corner (r2, c2) moved one step in
    /// the arrow-key direction. The anchor (r1, c1) is preserved. Clamps at 1.
    pub fn extend_trailing(self, key: &str) -> Self {
        let (r2, c2) = match key {
            "ArrowDown" => (self.r2 + 1, self.c2),
            "ArrowUp" => ((self.r2 - 1).max(1), self.c2),
            "ArrowLeft" => (self.r2, (self.c2 - 1).max(1)),
            "ArrowRight" => (self.r2, self.c2 + 1),
            _ => (self.r2, self.c2),
        };
        CellArea {
            r1: self.r1,
            c1: self.c1,
            r2,
            c2,
        }
    }

    /// Returns `(row_tiles, col_tiles)` if `src` tiles exactly into `self`
    /// with no remainder, or `None` if any dimension isn't an exact multiple.
    ///
    /// A 1×1 source always divides evenly, so it tiles into any destination.
    pub fn tile_reps_of(self, src: CellArea) -> Option<(usize, usize)> {
        let row_reps = self.height() / src.height();
        let col_reps = self.width() / src.width();
        let fills_exactly =
            row_reps * src.height() == self.height() && col_reps * src.width() == self.width();
        let dst_is_larger = row_reps > 1 || col_reps > 1;
        (fills_exactly && dst_is_larger).then_some((row_reps, col_reps))
    }

    /// Convert to the `(r1, c1, r2, c2)` tuple the ironcalc API expects.
    pub(crate) fn as_tuple(self) -> (i32, i32, i32, i32) {
        (self.r1, self.c1, self.r2, self.c2)
    }

    /// Convert to an ironcalc `Area` (top-left origin + dimensions) on the given sheet.
    pub fn to_area(self, sheet: u32) -> Area {
        Area {
            sheet,
            row: self.r1,
            column: self.c1,
            height: self.r2 - self.r1 + 1,
            width: self.c2 - self.c1 + 1,
        }
    }
}

impl From<(i32, i32, i32, i32)> for CellArea {
    fn from((r1, c1, r2, c2): (i32, i32, i32, i32)) -> Self {
        Self { r1, c1, r2, c2 }
    }
}

impl From<[i32; 4]> for CellArea {
    fn from(range: [i32; 4]) -> Self {
        Self {
            r1: range[0],
            c1: range[1],
            r2: range[2],
            c2: range[3],
        }
    }
}

impl From<CellArea> for [i32; 4] {
    fn from(a: CellArea) -> Self {
        [a.r1, a.c1, a.r2, a.c2]
    }
}

/// Sheet-relative cell address. All indices are 1-based.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellAddress {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
}

impl CellAddress {
    /// Read the address from the model's current selected view.
    pub fn from_view(model: &UserModel<'static>) -> Self {
        let m = model.get_selected_view();
        Self {
            sheet: m.sheet,
            row: m.row,
            column: m.column,
        }
    }

    /// Read the address from an in-progress [`EditingCell`].
    pub fn from_editing(cell: &EditingCell) -> Self {
        Self {
            sheet: cell.address.sheet,
            row: cell.address.row,
            column: cell.address.column,
        }
    }
}
