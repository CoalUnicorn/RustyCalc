use ironcalc_base::{expressions::types::Area, UserModel};

use crate::state::EditingCell;

// SheetArea
#[derive(Clone, Debug, PartialEq)]
pub struct SheetArea {
    pub sheet: u32,
    pub area: CellArea,
}

impl SheetArea {
    pub fn new(sheet: u32, r1: i32, c1: i32, r2: i32, c2: i32) -> Self {
        Self {
            sheet,
            area: CellArea { r1, c1, r2, c2 },
        }
    }

    pub fn from_cell(sheet: u32, row: i32, col: i32) -> Self {
        Self {
            sheet,
            area: CellArea::from_cell(row, col),
        }
    }

    pub fn from_address(addr: CellAddress) -> Self {
        SheetArea {
            sheet: addr.sheet,
            area: CellArea::from_cell(addr.row, addr.column),
        }
    }

    pub fn from_view(model: &UserModel) -> Self {
        Self {
            sheet: model.get_selected_sheet(),
            area: CellArea::from_model(model),
        }
    }

    // pub fn contains_address(self, addr: CellAddress) -> bool {
    //     if addr.sheet == self.sheet {
    //         return self.area.contains(addr.row, addr.column)
    //     }
    //     false
    // }

    pub fn on_same_sheet(self, other: SheetArea) -> bool {
        self.sheet == other.sheet
    }

    pub fn to_ironcalc_area(&self) -> Area {
        CellArea::to_area(self.area, self.sheet)
    }
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
    pub fn height(self) -> i32 {
        self.r2 - self.r1 + 1
    }

    pub fn width(self) -> i32 {
        self.c2 - self.c1 + 1
    }
    pub fn is_single_cell(self) -> bool {
        self.r1 == self.r2 && self.c1 == self.c2
    }

    pub fn rows(self) -> std::ops::Range<i32> {
        self.r1..self.r2 + 1
    }
    pub fn columns(self) -> std::ops::Range<i32> {
        self.c1..self.c2 + 1
    }

    pub fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        self.rows()
            .flat_map(move |row| self.columns().map(move |col| (row, col)))
    }

    pub fn from_cell(row: i32, col: i32) -> Self {
        CellArea {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        }
    }

    pub fn contains(self, row: i32, col: i32) -> bool {
        (self.r1..=self.r2).contains(&row) && (self.c1..=self.c2).contains(&col)
    }

    pub fn normalized(self) -> Self {
        Self {
            r1: self.r1.min(self.r2),
            c1: self.c1.min(self.c2),
            r2: self.r1.max(self.r2),
            c2: self.c1.max(self.c2),
        }
    }

    pub fn with_sheet(self, sheet: u32) -> SheetArea {
        SheetArea { sheet, area: self }
    }

    pub fn from_model(model: &UserModel) -> Self {
        Self::from(model.get_selected_view().range)
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

    /// Clipboard
    /// Returns `(row_tiles, col_tiles)` if `src` tiles exactly into `self`
    /// with no remainder, or `None` if any dimension isn't an exact multiple.
    ///
    /// A 1×1 source always divides evenly, so it tiles into any destination.
    pub fn tile_reps_of(self, src: CellArea) -> Option<(i32, i32)> {
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

    pub fn on_sheet(self, sheet: u32) -> bool {
        self.sheet == sheet
    }

    /// Wrap this address as a 1×1 [`SheetArea`].
    pub fn to_sheet_area(self) -> SheetArea {
        SheetArea {
            sheet: self.sheet,
            area: CellArea::from_cell(self.row, self.column),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_includes_corners() {
        let a = CellArea {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 3,
        };
        assert!(a.contains(1, 1), "top-left");
        assert!(a.contains(3, 3), "bottom-right");
        assert!(!a.contains(4, 1), "outside");
    }

    #[test]
    fn contains_single_cell_area() {
        let a = CellArea::from_cell(5, 7);
        assert!(a.contains(5, 7));
        assert!(!a.contains(5, 8));
    }

    #[test]
    fn normalized_swaps_inverted_coords() {
        let a = CellArea {
            r1: 4,
            c1: 3,
            r2: 1,
            c2: 1,
        };
        assert_eq!(
            a.normalized(),
            CellArea {
                r1: 1,
                c1: 1,
                r2: 4,
                c2: 3
            }
        );
    }

    #[test]
    fn to_sheet_area_produces_single_cell() {
        let addr = CellAddress {
            sheet: 2,
            row: 4,
            column: 6,
        };
        let sa = addr.to_sheet_area();
        assert_eq!(sa.sheet, 2);
        assert_eq!(sa.area, CellArea::from_cell(4, 6));
        assert!(sa.area.is_single_cell());
    }
}
