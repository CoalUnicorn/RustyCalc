/// Inclusive rectangular range of cells. The two corners may be in either
/// order: the geometry accessors (`height`/`width`/`contains`/`cells`)
/// normalize internally, so they are correct regardless of corner order.
/// Call [`RCRange::normalized`] only when you need the raw fields ordered
/// (`r1 <= r2`, `c1 <= c2`).
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
        (self.r2 - self.r1).abs() + 1
    }

    pub fn width(self) -> i32 {
        (self.c2 - self.c1).abs() + 1
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

    /// Grow every address edge by `amount` without imposing worksheet bounds.
    pub fn grow_by(self, amount: i32) -> Self {
        assert!(amount >= 0, "RCRange growth must be non-negative");
        let normalized = self.normalized();
        Self {
            r1: normalized.r1.saturating_sub(amount),
            c1: normalized.c1.saturating_sub(amount),
            r2: normalized.r2.saturating_add(amount),
            c2: normalized.c2.saturating_add(amount),
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
        let n = self.normalized();
        n.rows()
            .flat_map(move |row| n.columns().map(move |col| (row, col)))
    }

    pub fn contains(self, row: i32, col: i32) -> bool {
        let n = self.normalized();
        (n.r1..=n.r2).contains(&row) && (n.c1..=n.c2).contains(&col)
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
/// `color_idx % FORMULA_REF_COLORS.len()` (see [`crate::theme`]).
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct FormulaRef {
    pub sheet_area: SheetArea,
    pub color_idx: usize,
    pub kind: FormulaRefKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    // A range with reversed corners must report the same geometry as its
    // normalized form — height/width stay positive, contains/cells aren't empty.
    #[test]
    fn reversed_range_geometry_matches_normalized() {
        let reversed = RCRange {
            r1: 5,
            c1: 3,
            r2: 2,
            c2: 1,
        };
        let forward = reversed.normalized();

        assert_eq!(reversed.height(), forward.height());
        assert_eq!(reversed.width(), forward.width());
        assert!(reversed.height() > 0 && reversed.width() > 0);

        assert!(reversed.contains(3, 2));
        assert!(!reversed.contains(6, 2));

        let cells: Vec<_> = reversed.cells().collect();
        assert_eq!(cells, forward.cells().collect::<Vec<_>>());
        assert!(!cells.is_empty());
    }

    #[test]
    fn grow_by_normalizes_and_saturates_without_clamping_to_one() {
        let grown = RCRange {
            r1: 3,
            c1: 2,
            r2: 1,
            c2: 4,
        }
        .grow_by(2);
        assert_eq!(
            grown,
            RCRange {
                r1: -1,
                c1: 0,
                r2: 5,
                c2: 6,
            }
        );

        assert_eq!(
            RCRange::from_cell(i32::MAX, i32::MIN).grow_by(1),
            RCRange {
                r1: i32::MAX - 1,
                c1: i32::MIN,
                r2: i32::MAX,
                c2: i32::MIN + 1,
            }
        );
    }

    #[test]
    #[should_panic(expected = "RCRange growth must be non-negative")]
    fn grow_by_rejects_negative_amounts() {
        let _ = RCRange::from_cell(1, 1).grow_by(-1);
    }
}
