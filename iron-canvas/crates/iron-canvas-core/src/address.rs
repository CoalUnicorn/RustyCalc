//! Cell-space addresses and areas.
//!
//! `RCRange`, `DenseRange`, `CellCoord`, `SheetArea`, `FormulaRef`,
//! `FormulaRefKind`, and `AutofillTarget`.

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

    /// True when both corners identify the same cell: `r1 == r2` and
    /// `c1 == c2`. Swapping the corners does not change the result.
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

/// An ordered, dense address rectangle: `r1 <= r2` and `c1 <= c2`.
///
/// [`RCRange`] permits either corner order on purpose — selection growth,
/// point-mode drags, and range arithmetic all hand back whichever corner came
/// first, and normalizing at every read would tax the whole crate for a
/// property only dense work needs. Dense work is that property: the bulk fetch
/// filling one `FetchedCells` bundle iterates `r1..=r2` and its index math
/// multiplies height by width, so a reversed range there is not a clampable
/// quirk — it is zero cells fetched and blank paint.
///
/// This type *parses* the permissive range once at the dense boundary
/// ([`DenseRange::from_rc`], or `RCRange::into`). Downstream code reads ordered
/// fields and needs no `max`/`min` repair. It is crate-private: dense fetch and
/// bundle indexing are execution details, not consumer API.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct DenseRange {
    r1: i32,
    c1: i32,
    r2: i32,
    c2: i32,
}

impl DenseRange {
    /// Normalize `range`'s corners into the dense invariant. This is the one
    /// place a dual-direction [`RCRange`] becomes an ordered rectangle.
    pub(crate) fn from_rc(range: RCRange) -> Self {
        let n = range.normalized();
        Self {
            r1: n.r1,
            c1: n.c1,
            r2: n.r2,
            c2: n.c2,
        }
    }

    /// Row count. Computed in `i64` so a full `i32` address span cannot
    /// overflow the subtraction the way the raw `r2 - r1 + 1` form could.
    #[inline]
    pub(crate) fn height(self) -> usize {
        usize::try_from(i64::from(self.r2) - i64::from(self.r1) + 1).unwrap_or(usize::MAX)
    }

    /// Column count; same `i64` width as [`Self::height`].
    #[inline]
    pub(crate) fn width(self) -> usize {
        usize::try_from(i64::from(self.c2) - i64::from(self.c1) + 1).unwrap_or(usize::MAX)
    }

    /// Addressed cell count for the four fetched channels.
    #[inline]
    pub(crate) fn addressed_cells(self) -> usize {
        self.height().saturating_mul(self.width())
    }

    /// Back to the permissive address type, e.g. for the model's bulk
    /// accessors (which keep their `RCRange` signatures) and the diagnostic
    /// wire shape.
    #[inline]
    pub(crate) fn as_rc(self) -> RCRange {
        RCRange {
            r1: self.r1,
            c1: self.c1,
            r2: self.r2,
            c2: self.c2,
        }
    }
}

impl From<RCRange> for DenseRange {
    fn from(range: RCRange) -> Self {
        Self::from_rc(range)
    }
}

/// Target cell of an in-progress autofill-handle drag.
#[derive(Copy, Clone, PartialEq)]
pub struct AutofillTarget {
    pub row: i32,
    pub col: i32,
}

/// Sheet coordinates of one cell, 1-based on both axes.
///
/// Named so a row/column pair that travels across module boundaries — selection
/// state to the overlay paint pass — cannot be transposed the way two adjacent
/// `i32` parameters can.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CellCoord {
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

    #[test]
    fn dense_full_address_span_never_wraps_to_zero() {
        let dense = DenseRange::from_rc(RCRange::from([i32::MIN, i32::MIN, i32::MAX, i32::MAX]));
        let expected = usize::try_from(1_u64 << 32).unwrap_or(usize::MAX);
        assert_eq!(dense.height(), expected);
        assert_eq!(dense.width(), expected);
        assert_eq!(dense.addressed_cells(), usize::MAX);
    }

    /// The dense boundary is where a dual-direction range stops being
    /// permissive: the parsed value counts its full area instead of the zero
    /// cells a raw `r1..=r2` walk would fetch.
    #[test]
    fn dense_range_parses_reversed_corners_into_full_area() {
        let dense = DenseRange::from_rc(RCRange {
            r1: 5,
            c1: 3,
            r2: 2,
            c2: 1,
        });

        assert_eq!(
            dense.as_rc(),
            RCRange {
                r1: 2,
                c1: 1,
                r2: 5,
                c2: 3
            }
        );
        assert_eq!(dense.height(), 4);
        assert_eq!(dense.width(), 3);
        assert_eq!(dense.addressed_cells(), 12);
    }

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
