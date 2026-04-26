use crate::CanvasModel;

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct RCRange {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}
impl RCRange {
    pub fn rows(self) -> std::ops::RangeInclusive<i32> {
        std::ops::RangeInclusive::new(self.r1, self.r2)
    }
    pub fn columns(self) -> std::ops::RangeInclusive<i32> {
        std::ops::RangeInclusive::new(self.c1, self.c2)
    }

    pub fn height(self) -> i32 {
        self.r2 - self.r1 + 1
    }

    pub fn width(self) -> i32 {
        self.c2 - self.c1 + 1
    }
    /// Swap corners so `r1 <= r2` and `c1 <= c2`.
    pub fn normalized(self) -> Self {
        Self {
            r1: self.r1.min(self.r2),
            c1: self.c1.min(self.c2),
            r2: self.r1.max(self.r2),
            c2: self.c1.max(self.c2),
        }
    }

    pub fn is_single_cell(self) -> bool {
        self.r1 == self.r2 && self.c1 == self.c2
    }

    pub fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        self.rows()
            .flat_map(move |row| self.columns().map(move |col| (row, col)))
    }

    pub fn contains(self, row: i32, col: i32) -> bool {
        (self.r1..=self.r2).contains(&row) && (self.c1..=self.c2).contains(&col)
    }

    pub fn from_cell(row: i32, col: i32) -> Self {
        Self {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        }
    }

    pub fn with_sheet(self, sheet: u32) -> SheetArea {
        SheetArea { sheet, range: self }
    }
}
impl From<[i32; 4]> for RCRange {
    fn from(range: [i32; 4]) -> Self {
        Self {
            r1: range[0],
            c1: range[1],
            r2: range[2],
            c2: range[3],
        }
    }
}

impl RCRange {
    pub fn from_view(model: &dyn CanvasModel) -> Self {
        Self::from(model.get_selected_view().range)
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct CellAddress {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct SheetArea {
    pub sheet: u32,
    pub range: RCRange,
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct FormulaRef {
    pub sheet_area: SheetArea, // what to outline
    pub color_idx: usize,      // index into FORMULA_REF_COLORS
    pub active: bool,          // emphasize the ref under cursor
}

pub struct CssColor(pub String);

impl CssColor {
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() {
            Self("#000000".to_owned())
        } else {
            Self(s.to_lowercase())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_swaps_corners_when_inverted() {
        let r = RCRange {
            r1: 5,
            c1: 7,
            r2: 2,
            c2: 3,
        }
        .normalized();
        assert_eq!(r, RCRange { r1: 2, c1: 3, r2: 5, c2: 7 });
    }

    #[test]
    fn normalized_is_idempotent_on_already_ordered() {
        let r = RCRange { r1: 1, c1: 2, r2: 3, c2: 4 };
        assert_eq!(r.normalized(), r);
    }

    #[test]
    fn width_and_height_are_inclusive() {
        let r = RCRange { r1: 2, c1: 5, r2: 4, c2: 8 };
        assert_eq!(r.height(), 3);
        assert_eq!(r.width(), 4);
    }

    #[test]
    fn is_single_cell_only_when_corners_match() {
        assert!(RCRange::from_cell(7, 9).is_single_cell());
        assert!(!RCRange { r1: 1, c1: 1, r2: 1, c2: 2 }.is_single_cell());
    }

    #[test]
    fn contains_respects_inclusive_bounds() {
        let r = RCRange { r1: 2, c1: 3, r2: 4, c2: 5 };
        assert!(r.contains(2, 3));
        assert!(r.contains(4, 5));
        assert!(r.contains(3, 4));
        assert!(!r.contains(1, 3));
        assert!(!r.contains(5, 5));
    }

    #[test]
    fn cells_walks_row_major() {
        let r = RCRange { r1: 1, c1: 1, r2: 2, c2: 2 };
        let cells: Vec<_> = r.cells().collect();
        assert_eq!(cells, vec![(1, 1), (1, 2), (2, 1), (2, 2)]);
    }

    #[test]
    fn from_array_maps_in_order_r1_c1_r2_c2() {
        let r: RCRange = [3, 5, 7, 9].into();
        assert_eq!(r, RCRange { r1: 3, c1: 5, r2: 7, c2: 9 });
    }

    #[test]
    fn with_sheet_attaches_sheet_id() {
        let area = RCRange::from_cell(2, 3).with_sheet(7);
        assert_eq!(area.sheet, 7);
        assert_eq!(area.range, RCRange::from_cell(2, 3));
    }

    #[test]
    fn css_color_empty_string_falls_back_to_black() {
        assert_eq!(CssColor::new("").as_str(), "#000000");
    }

    #[test]
    fn css_color_lowercases_hex_input() {
        assert_eq!(CssColor::new("#FF00AA").as_str(), "#ff00aa");
    }
}
