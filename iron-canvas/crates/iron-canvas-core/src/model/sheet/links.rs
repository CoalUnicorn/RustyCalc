//! Engine-neutral cell hyperlink data.
//!
//! Core never imports IronCalc or host types: an adapter resolves the engine's
//! link into [`CellLink`] once, at the model boundary, exactly as it already
//! resolves engine colors into `#RRGGBB` strings.
//!
//! Links are sparse. A sheet holds at most a few thousand of them, so the
//! committed state is a keyed index over absolute `(row, column)` addresses,
//! not a dense per-cell channel. Absolute keys need no blit shift.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::address::RCRange;
use crate::geometry::constants::{LAST_COLUMN, LAST_ROW};

/// Destination of a cell hyperlink.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    /// URL, `mailto:` URI, or external file target.
    External(String),
    /// Location in this workbook: a cell reference such as `Sheet1!A30`, or a
    /// defined name.
    Internal(String),
}

impl LinkTarget {
    /// The target string, without regard to its kind.
    pub fn as_str(&self) -> &str {
        match self {
            LinkTarget::External(target) | LinkTarget::Internal(target) => target,
        }
    }

    /// True for a target outside this workbook.
    pub fn is_external(&self) -> bool {
        matches!(self, LinkTarget::External(_))
    }
}

/// One cell hyperlink, as committed render state carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellLink {
    /// Single-cell address. The containing snapshot supplies the sheet.
    range: RCRange,
    target: LinkTarget,
    tooltip: Option<String>,
    /// A formula (`HYPERLINK`) owns a dynamic link. The host cannot edit or
    /// delete it; only the formula can.
    dynamic: bool,
    /// Resolved hyperlink color, `#RRGGBB`. Resolved at the model boundary
    /// from the workbook theme, as other engine colors already are.
    color: Option<String>,
    /// Digest of every field above. Folded into the cell fingerprint so a
    /// target change with an unchanged label forces a repaint. Precomputed
    /// once: the fingerprint path must not hash strings per frame.
    digest: u64,
}

impl CellLink {
    /// Build one link and precompute its [`Self::digest`].
    pub fn new(
        range: RCRange,
        target: LinkTarget,
        tooltip: Option<String>,
        dynamic: bool,
        color: Option<String>,
    ) -> Self {
        let digest = link_digest(
            range,
            &target,
            tooltip.as_deref(),
            dynamic,
            color.as_deref(),
        );
        Self {
            range,
            target,
            tooltip,
            dynamic,
            color,
            digest,
        }
    }

    /// Single-cell address of this link.
    pub fn range(&self) -> RCRange {
        self.range
    }

    pub fn target(&self) -> &LinkTarget {
        &self.target
    }

    pub fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    pub fn is_dynamic(&self) -> bool {
        self.dynamic
    }

    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    /// Digest of every field. `cell_digest` folds this single `u64` so the
    /// fingerprint covers presence, color, target, tooltip, and dynamic state.
    pub fn digest(&self) -> u64 {
        self.digest
    }
}

fn link_digest(
    range: RCRange,
    target: &LinkTarget,
    tooltip: Option<&str>,
    dynamic: bool,
    color: Option<&str>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    range.r1.hash(&mut hasher);
    range.c1.hash(&mut hasher);
    match target {
        LinkTarget::External(target) => {
            0u8.hash(&mut hasher);
            target.hash(&mut hasher);
        }
        LinkTarget::Internal(target) => {
            1u8.hash(&mut hasher);
            target.hash(&mut hasher);
        }
    }
    tooltip.hash(&mut hasher);
    dynamic.hash(&mut hasher);
    color.hash(&mut hasher);
    hasher.finish()
}

/// Why a captured link list is not a valid [`LinkIndex`]. The engine
/// guarantees every rule below; a custom JavaScript model does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkIndexError {
    /// The link's range is not one cell. Canvas link state is per-cell.
    NotSingleCell { row: i32, column: i32 },
    /// The link's address lies outside `1..=LAST_ROW` / `1..=LAST_COLUMN`.
    OutOfBounds { row: i32, column: i32 },
    /// Two links claim the same address.
    Duplicate { row: i32, column: i32 },
}

/// Committed link state for one sheet. Empty for a sheet with no links.
#[derive(Debug)]
pub struct LinkIndex {
    cells: HashMap<(i32, i32), Rc<CellLink>>,
    digest: u64,
}

impl Default for LinkIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl LinkIndex {
    /// The empty index. `HashMap::new` does not allocate, so this costs
    /// nothing as a `Chrome` default. Not a `const`: `HashMap::new` is not a
    /// const function, and an empty index is built once per `Chrome`.
    pub fn empty() -> LinkIndex {
        LinkIndex {
            cells: HashMap::new(),
            digest: 0,
        }
    }

    /// Build the validated index. Rejects a non-single-cell range, an address
    /// outside the worksheet bounds, and a duplicate address.
    pub fn from_cells(cells: Vec<CellLink>) -> Result<LinkIndex, LinkIndexError> {
        let mut index: HashMap<(i32, i32), Rc<CellLink>> = HashMap::with_capacity(cells.len());
        let mut ordered: Vec<(i32, i32)> = Vec::with_capacity(cells.len());
        for cell in cells {
            let range = cell.range;
            if !range.is_single_cell() {
                return Err(LinkIndexError::NotSingleCell {
                    row: range.r1,
                    column: range.c1,
                });
            }
            let address = range.normalized();
            if !(1..=LAST_ROW).contains(&address.r1) || !(1..=LAST_COLUMN).contains(&address.c1) {
                return Err(LinkIndexError::OutOfBounds {
                    row: address.r1,
                    column: address.c1,
                });
            }
            let key = (address.r1, address.c1);
            if index.insert(key, Rc::new(cell)).is_some() {
                return Err(LinkIndexError::Duplicate {
                    row: key.0,
                    column: key.1,
                });
            }
            ordered.push(key);
        }
        // Deterministic digest: the capture order is the engine's, which is
        // not a contract. Fold the precomputed per-link digests in address
        // order, so an identical set hashes identically however it arrives.
        ordered.sort_unstable();
        let digest = if ordered.is_empty() {
            0
        } else {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            ordered.len().hash(&mut hasher);
            for key in ordered {
                if let Some(cell) = index.get(&key) {
                    key.hash(&mut hasher);
                    cell.digest().hash(&mut hasher);
                }
            }
            hasher.finish()
        };
        Ok(LinkIndex {
            cells: index,
            digest,
        })
    }

    /// The link attached to `(row, col)`, if any.
    pub fn get(&self, row: i32, column: i32) -> Option<&Rc<CellLink>> {
        self.cells.get(&(row, column))
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Digest of the whole index. Compared once per attempt to decide whether
    /// the link set changed, and folded per cell into the grid fingerprint.
    pub fn digest(&self) -> u64 {
        self.digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(row: i32, column: i32, target: &str) -> CellLink {
        CellLink::new(
            RCRange::from_cell(row, column),
            LinkTarget::External(target.to_string()),
            None,
            false,
            None,
        )
    }

    #[test]
    fn rejects_duplicate_address() {
        let index = LinkIndex::from_cells(vec![link(2, 3, "a"), link(2, 3, "b")]);
        assert_eq!(
            index.unwrap_err(),
            LinkIndexError::Duplicate { row: 2, column: 3 }
        );
    }

    #[test]
    fn rejects_out_of_bounds_and_non_single_cell() {
        let oob = LinkIndex::from_cells(vec![link(0, 1, "a")]).unwrap_err();
        assert_eq!(oob, LinkIndexError::OutOfBounds { row: 0, column: 1 });

        let wide = CellLink::new(
            RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 2,
            },
            LinkTarget::External("a".to_string()),
            None,
            false,
            None,
        );
        assert_eq!(
            LinkIndex::from_cells(vec![wide]).unwrap_err(),
            LinkIndexError::NotSingleCell { row: 1, column: 1 }
        );
    }

    #[test]
    fn digest_changes_when_only_target_changes() {
        let a = LinkIndex::from_cells(vec![link(4, 4, "https://a.example")]).unwrap();
        let b = LinkIndex::from_cells(vec![link(4, 4, "https://b.example")]).unwrap();
        assert_ne!(a.digest(), b.digest());
        assert_ne!(a.get(4, 4).unwrap().digest(), b.get(4, 4).unwrap().digest());
    }

    #[test]
    fn digest_is_order_independent_and_empty_matches_constant() {
        let forward = LinkIndex::from_cells(vec![link(1, 1, "a"), link(9, 2, "b")]).unwrap();
        let reverse = LinkIndex::from_cells(vec![link(9, 2, "b"), link(1, 1, "a")]).unwrap();
        assert_eq!(forward.digest(), reverse.digest());
        assert_eq!(LinkIndex::from_cells(Vec::new()).unwrap().digest(), 0);
        assert_eq!(LinkIndex::empty().digest(), 0);
        assert!(LinkIndex::empty().is_empty());
    }
}
