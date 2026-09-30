//! Immutable worksheet render metadata for one paint attempt.
//!
//! One value carries every worksheet-level index the grid needs — today the
//! link index and the merge table. A candidate frame, its committed pixels,
//! and the queries that resolve against them read the same value, so paint and
//! hit geometry cannot disagree about which revision of the metadata they
//! describe.
//!
//! `Rc` throughout: a repeated attempt that reuses a capture is one refcount
//! bump, not a deep clone of both indexes.

use std::rc::Rc;

use crate::model::sheet::links::LinkIndex;
use crate::model::sheet::merges::MergeTable;

/// The engine-neutral worksheet metadata captured for one attempt.
#[derive(Clone)]
pub(crate) struct SheetMetadata {
    links: Rc<LinkIndex>,
    merges: Rc<MergeTable>,
}

impl SheetMetadata {
    pub(crate) fn new(links: Rc<LinkIndex>, merges: Rc<MergeTable>) -> Self {
        Self { links, merges }
    }

    /// The captured link index for the sheet. Empty when the sheet has none.
    pub(crate) fn links(&self) -> &Rc<LinkIndex> {
        &self.links
    }

    /// The captured merge table for the sheet. Empty when the sheet has none.
    pub(crate) fn merges(&self) -> &Rc<MergeTable> {
        &self.merges
    }
}
