//! Layout-compatibility classification for the retained grid cache.
//!
//! `GridCache::classify_layout` and the fingerprint row rotation ask the same
//! question about the same pair of layouts: can the committed address layout
//! carry the candidate's retained pixels? The answer is an address fact, not a
//! painting decision, so it lives in the cache tier next to
//! [`GridCache`](super::GridCache) rather than in the cell tier that consumes
//! the resulting pixels.

use crate::chrome::{GridLayout, PaneRegion};
use crate::geometry::prim::Axis;

#[derive(Clone, Copy)]
pub(crate) enum GridLayoutTransition {
    Exact,
    Shift { axis: Axis },
    Incompatible,
}

impl GridLayoutTransition {
    /// Classify only exact-layout compatibility. Buffer validity is an
    /// independent grid-cache fact and must not change this result.
    pub(crate) fn classify(committed: GridLayout, candidate: GridLayout) -> Self {
        if committed == candidate {
            return Self::Exact;
        }
        if committed.shape() != candidate.shape() {
            return Self::Incompatible;
        }

        let unchanged = |region| committed.segment(region) == candidate.segment(region);
        let shifted = |region: PaneRegion, axis: Axis| {
            let before = committed.segment(region);
            let after = candidate.segment(region);
            match (before, after) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    let before = before.range();
                    let after = after.range();
                    match axis {
                        Axis::Row => {
                            before.c1 == after.c1
                                && before.c2 == after.c2
                                && before.r2 - before.r1 == after.r2 - after.r1
                                && before.r1 != after.r1
                        }
                        Axis::Column => {
                            before.r1 == after.r1
                                && before.r2 == after.r2
                                && before.c2 - before.c1 == after.c2 - after.c1
                                && before.c1 != after.c1
                        }
                    }
                }
                (None, Some(_)) | (Some(_), None) => false,
            }
        };

        if unchanged(PaneRegion::TopLeft)
            && unchanged(PaneRegion::TopRight)
            && shifted(PaneRegion::BottomLeft, Axis::Row)
            && shifted(PaneRegion::BottomRight, Axis::Row)
        {
            return Self::Shift { axis: Axis::Row };
        }

        if unchanged(PaneRegion::TopLeft)
            && unchanged(PaneRegion::BottomLeft)
            && shifted(PaneRegion::TopRight, Axis::Column)
            && shifted(PaneRegion::BottomRight, Axis::Column)
        {
            return Self::Shift { axis: Axis::Column };
        }

        Self::Incompatible
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::cache::test_support::layout;

    #[test]
    fn layout_classifies_exact_row_column_and_incompatible() {
        let base = layout(10, 8, 2, 2);
        assert!(matches!(
            GridLayoutTransition::classify(base, base),
            GridLayoutTransition::Exact
        ));
        assert!(matches!(
            GridLayoutTransition::classify(base, layout(11, 8, 2, 2)),
            GridLayoutTransition::Shift { axis: Axis::Row }
        ));
        assert!(matches!(
            GridLayoutTransition::classify(base, layout(10, 9, 2, 2)),
            GridLayoutTransition::Shift { axis: Axis::Column }
        ));
        assert!(matches!(
            GridLayoutTransition::classify(base, layout(10, 8, 0, 2)),
            GridLayoutTransition::Incompatible
        ));
    }
}
