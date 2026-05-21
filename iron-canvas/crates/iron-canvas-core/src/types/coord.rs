/// Inclusive rectangular range of cells. The two corners may be in either
/// order — call [`RCRange::normalized`] if you need `r1 <= r2` and
/// `c1 <= c2`.
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
    ///
    /// # Examples
    ///
    /// ```
    /// use iron_canvas_core::RCRange;
    /// let backwards = RCRange { r1: 5, c1: 3, r2: 2, c2: 1 };
    /// let n = backwards.normalized();
    /// assert_eq!((n.r1, n.c1, n.r2, n.c2), (2, 1, 5, 3));
    /// ```
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

    /// Iterate all `(row, col)` pairs in the range, row-major.
    ///
    /// # Examples
    ///
    /// ```
    /// use iron_canvas_core::RCRange;
    /// let r = RCRange { r1: 1, c1: 1, r2: 2, c2: 2 };
    /// let v: Vec<_> = r.cells().collect();
    /// assert_eq!(v, vec![(1, 1), (1, 2), (2, 1), (2, 2)]);
    /// ```
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

/// Target cell of an in-progress autofill-handle drag.
#[derive(Copy, Clone, PartialEq)]
pub struct AutofillTarget {
    pub row: i32,
    pub col: i32,
}

/// An [`RCRange`] qualified with the sheet it lives on.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct SheetArea {
    pub sheet: u32,
    pub range: RCRange,
}

/// Origin of a [`FormulaRef`]. The renderer treats all kinds the same today;
/// `Direct` is the only draggable kind.
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub enum FormulaRefKind {
    /// `A1`, `Sheet2!B3:C5` — a `Node::ReferenceKind` / `Node::RangeKind`
    /// emission. Resolvable to coords; draggable in-place.
    #[default]
    Direct,
    /// `my_range` ident bound to a defined name. Not draggable — moving it
    /// would require rewriting the name binding, not the coord span.
    DefinedName,
    /// Parser bailed on the formula; the ref came from a fallback span.
    /// Not draggable.
    Unresolved,
}

/// One cell or range reference parsed out of an in-edit formula. The
/// renderer outlines `sheet_area` with the color slot at
/// `color_idx % FORMULA_REF_COLORS.len()` (see [`crate::theme`]); per-ref
/// active-emphasis is driven separately by [`crate::RenderOverlays::active_ref`].
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct FormulaRef {
    pub sheet_area: SheetArea,
    pub color_idx: usize,
    pub kind: FormulaRefKind,
}
