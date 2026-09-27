//! Engine-neutral merged-range geometry.
//!
//! Core never imports IronCalc: an adapter converts the engine's typed
//! `MergedCell { row, column, width, height }` into an inclusive [`RCRange`]
//! once, at the model boundary. The renderer only ever sees the rectangle.
//!
//! A merge is one logical cell: its top-left cell (the *anchor*) owns the
//! value, style, decoration, and link; every other cell of the rectangle is
//! covered. Hit queries resolve a covered cell to its anchor, and paint draws
//! the merge over the per-cell pass.
//!
//! The committed state is the merge list itself: lookups scan rectangles, so
//! construction is proportional to the number of merges and never to their
//! covered area. A full-column merge (`A1:A1048576`) therefore costs one
//! entry, not a million. Absolute addresses need no blit shift, and a sheet
//! holds few merges, so a scan answers a hit query in the same order as a map
//! probe would in practice. An interval tree is the deferred alternative if a
//! measurement ever justifies it.

use std::hash::{Hash, Hasher};

use crate::address::{CellCoord, RCRange};
use crate::chrome::GridLayout;
use crate::geometry::constants::{LAST_COLUMN, LAST_ROW};

/// One merged range. `range` is the full logical rectangle; `anchor` is its
/// top-left cell, the only cell a user can edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedRange {
    pub range: RCRange,
    pub anchor: CellCoord,
}

/// Why a captured merge list is not a valid [`MergeTable`]. The engine
/// guarantees every rule below; a custom JavaScript model does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeTableError {
    /// The range's address lies outside `1..=LAST_ROW` / `1..=LAST_COLUMN`.
    OutOfBounds { row: i32, column: i32 },
    /// Two ranges overlap. An overlapping table would make `anchor_at`
    /// order-dependent, so the whole list is rejected.
    Overlap {
        first: RCRange,
        second: RCRange,
    },
}

/// Committed merge state for one sheet, plus the address lookups queries need.
#[derive(Debug)]
pub struct MergeTable {
    /// The committed merges. Every lookup scans this list, so the table's
    /// memory and construction cost are proportional to the merge count — never
    /// to the covered area a merge declares.
    merges: Vec<MergedRange>,
    digest: u64,
}

impl Default for MergeTable {
    fn default() -> Self {
        Self::empty()
    }
}

impl MergeTable {
    /// The empty table. An empty `Vec` does not allocate, so this costs
    /// nothing as a `Chrome` default. Not a `const`: an empty table is built
    /// once per `Chrome`, mirroring `LinkIndex::empty`.
    pub fn empty() -> MergeTable {
        MergeTable {
            merges: Vec::new(),
            digest: 0,
        }
    }

    /// Build the validated table from the captured ranges. Rejects an address
    /// outside the worksheet bounds and any two ranges that overlap.
    pub fn from_ranges(ranges: Vec<RCRange>) -> Result<MergeTable, MergeTableError> {
        let mut merges: Vec<MergedRange> = Vec::with_capacity(ranges.len());
        for range in ranges {
            let range = range.normalized();
            if !(1..=LAST_ROW).contains(&range.r1)
                || !(1..=LAST_COLUMN).contains(&range.c1)
                || !(1..=LAST_ROW).contains(&range.r2)
                || !(1..=LAST_COLUMN).contains(&range.c2)
            {
                return Err(MergeTableError::OutOfBounds {
                    row: range.r1,
                    column: range.c1,
                });
            }
            merges.push(MergedRange {
                range,
                anchor: CellCoord {
                    row: range.r1,
                    col: range.c1,
                },
            });
        }
        // Pairwise overlap reject. A sheet holds few merges, so the quadratic
        // scan is cheaper than an interval tree; the alternative is deferred.
        if let Some((first, second)) = first_overlap(&merges) {
            return Err(MergeTableError::Overlap { first, second });
        }

        // Deterministic digest: the capture order is the engine's, which is
        // not a contract. Sort by the rectangle so an identical set hashes
        // identically however it arrives.
        let mut ordered: Vec<RCRange> = merges.iter().map(|merge| merge.range).collect();
        ordered.sort_unstable_by_key(|range| (range.r1, range.c1, range.r2, range.c2));
        let digest = if ordered.is_empty() {
            0
        } else {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            ordered.len().hash(&mut hasher);
            for range in &ordered {
                range.r1.hash(&mut hasher);
                range.c1.hash(&mut hasher);
                range.r2.hash(&mut hasher);
                range.c2.hash(&mut hasher);
            }
            hasher.finish()
        };

        Ok(MergeTable { merges, digest })
    }

    /// The anchor of the merge covering `(row, col)`, if any. Returns the
    /// anchor for both the anchor cell and any covered cell.
    ///
    /// A scan, not a map probe: `from_ranges` rejects overlapping ranges, so at
    /// most one merge can contain the address, and the merge list is small by
    /// construction.
    pub fn anchor_at(&self, row: i32, col: i32) -> Option<CellCoord> {
        self.merge_at(row, col).map(|merge| merge.anchor)
    }

    /// The merge covering `(row, col)`, if any.
    pub fn merge_at(&self, row: i32, col: i32) -> Option<&MergedRange> {
        self.merges
            .iter()
            .find(|merge| merge.range.contains(row, col))
    }

    pub fn is_empty(&self) -> bool {
        self.merges.is_empty()
    }

    /// Every committed merge, in capture order. The merge paint pass walks
    /// this; per-cell queries use [`Self::anchor_at`]/[`Self::merge_at`].
    pub fn iter(&self) -> impl Iterator<Item = &MergedRange> {
        self.merges.iter()
    }

    /// Digest of the whole table. Compared once per attempt to decide whether
    /// the merge set changed.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// True when any merge overlaps a cell of `layout`. Both the committed and
    /// the candidate layout are probed this way, so scrolling into a merge and
    /// scrolling out of one both trigger a rebuild.
    pub fn intersects_visible(&self, layout: GridLayout) -> bool {
        self.merges.iter().any(|merge| {
            layout
                .segments()
                .any(|segment| ranges_overlap(merge.range, segment.range()))
        })
    }

    /// True when any merge overlaps the address rectangle `rect`. Used to probe
    /// the candidate visible area, which is not yet a built `GridLayout`.
    pub fn intersects_rect(&self, rect: RCRange) -> bool {
        self.merges
            .iter()
            .any(|merge| ranges_overlap(merge.range, rect))
    }
}

fn ranges_overlap(a: RCRange, b: RCRange) -> bool {
    let a = a.normalized();
    let b = b.normalized();
    a.r1 <= b.r2 && b.r1 <= a.r2 && a.c1 <= b.c2 && b.c1 <= a.c2
}

fn first_overlap(merges: &[MergedRange]) -> Option<(RCRange, RCRange)> {
    for (i, first) in merges.iter().enumerate() {
        for second in &merges[i + 1..] {
            if ranges_overlap(first.range, second.range) {
                return Some((first.range, second.range));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_at_resolves_anchor_and_covered_cells() {
        let table = MergeTable::from_ranges(vec![RCRange {
            r1: 2,
            c1: 3,
            r2: 4,
            c2: 5,
        }])
        .unwrap();
        let anchor = table.anchor_at(2, 3).unwrap();
        assert_eq!(anchor, CellCoord { row: 2, col: 3 });
        assert_eq!(table.anchor_at(4, 5), Some(anchor));
        assert_eq!(table.anchor_at(2, 4), Some(anchor));
        assert_eq!(table.anchor_at(9, 9), None);
        assert!(!table.is_empty());
    }

    #[test]
    fn rejects_out_of_bounds() {
        assert_eq!(
            MergeTable::from_ranges(vec![RCRange {
                r1: 0,
                c1: 1,
                r2: 1,
                c2: 1,
            }])
            .unwrap_err(),
            MergeTableError::OutOfBounds { row: 0, column: 1 }
        );
    }

    #[test]
    fn rejects_overlap() {
        let error = MergeTable::from_ranges(vec![
            RCRange {
                r1: 1,
                c1: 1,
                r2: 3,
                c2: 3,
            },
            RCRange {
                r1: 3,
                c1: 3,
                r2: 5,
                c2: 5,
            },
        ])
        .unwrap_err();
        assert!(matches!(error, MergeTableError::Overlap { .. }));
    }

    #[test]
    fn digest_is_order_independent_and_empty_matches_constant() {
        let forward = MergeTable::from_ranges(vec![
            RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 2,
            },
            RCRange {
                r1: 5,
                c1: 5,
                r2: 6,
                c2: 6,
            },
        ])
        .unwrap();
        let reverse = MergeTable::from_ranges(vec![
            RCRange {
                r1: 5,
                c1: 5,
                r2: 6,
                c2: 6,
            },
            RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 2,
            },
        ])
        .unwrap();
        assert_eq!(forward.digest(), reverse.digest());
        assert_ne!(forward.digest(), 0);
        assert_eq!(MergeTable::empty().digest(), 0);
    }

    /// A merge larger than any viewport is accepted and answers lookups at its
    /// far corner — without expanding into per-cell storage.
    #[test]
    fn a_large_merge_stores_one_entry_and_still_resolves() {
        let table = MergeTable::from_ranges(vec![RCRange {
            r1: 1,
            c1: 1,
            r2: 1000,
            c2: 50,
        }])
        .unwrap();
        assert_eq!(
            table.anchor_at(1000, 50),
            Some(CellCoord { row: 1, col: 1 })
        );
        assert_eq!(table.merges.len(), 1, "storage is per merge, not per cell");
    }

    /// A merge covering an entire column is a legal engine value. It must not
    /// expand into one entry per covered row: table construction and lookups
    /// stay proportional to the merge count.
    #[test]
    fn a_full_column_merge_costs_one_entry() {
        let table = MergeTable::from_ranges(vec![RCRange {
            r1: 1,
            c1: 1,
            r2: LAST_ROW,
            c2: 1,
        }])
        .unwrap();
        assert_eq!(table.merges.len(), 1);
        assert_eq!(
            table.anchor_at(LAST_ROW, 1),
            Some(CellCoord { row: 1, col: 1 })
        );
        assert_eq!(table.anchor_at(LAST_ROW, 2), None);
    }

    /// A whole-sheet rectangle is likewise one entry, even though it addresses
    /// far more cells than an index could hold.
    #[test]
    fn a_full_sheet_merge_costs_one_entry() {
        let table = MergeTable::from_ranges(vec![RCRange {
            r1: 1,
            c1: 1,
            r2: LAST_ROW,
            c2: LAST_COLUMN,
        }])
        .unwrap();
        assert_eq!(table.merges.len(), 1);
        assert_eq!(
            table.anchor_at(LAST_ROW, LAST_COLUMN),
            Some(CellCoord { row: 1, col: 1 })
        );
    }
}
