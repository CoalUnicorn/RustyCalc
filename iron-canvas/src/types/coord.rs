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
        model.get_selected_view().range
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct CellAddress {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
}

/// The target cell during an autofill-handle drag.
///
/// Replaces the anonymous `Option<(i32, i32)>` in `RenderOverlays` with a
/// named struct so the fields are self-documenting at every call site.
#[derive(Copy, Clone, PartialEq)]
pub struct AutofillTarget {
    pub row: i32,
    pub col: i32,
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

pub struct CssColor(String);

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

    pub fn into_string(self) -> String {
        self.0
    }
}
