//! Merge viewport impact: how the captured merge table interacts with the
//! attempt's geometry.
//!
//! Merge pixels and merge hit geometry commit together, so an attempt that
//! could show a new merge state must not take a fast path that repaints
//! covered cells over the merge, or that commits a table the pixels were not
//! painted from. This module owns that decision because it is the geometry
//! boundary: it inspects the committed frame's layout and the captured
//! viewport, and returns a small planning result that the pure planner
//! consumes without touching `Chrome` at all.

use crate::address::RCRange;
use crate::chrome::{Chrome, GridLayout};
use crate::frame::inputs::FrameInputs;
use crate::geometry::slot::scroll_first;
use crate::model::sheet::merges::MergeTable;

/// How the captured merge table interacts with this attempt's geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MergeImpact {
    /// No merge intersects either the committed or the candidate visible area,
    /// and the committed table's digest is unchanged. Every existing strategy
    /// stays selectable.
    None,
    /// A merge intersects the committed or the candidate visible area. A
    /// `Damage`/`Blit` fast path would repaint covered cells over the merge,
    /// so the grid rebuilds.
    Visible,
    /// The captured table's digest differs from the committed one. Any merged
    /// or unmerged region may have changed, so the grid rebuilds; this
    /// subsumes a row-scoped damage estimate.
    Changed,
}

impl MergeImpact {
    /// Classify the attempt. A missing committed frame counts as `Changed`: a
    /// first frame must not take a path that assumes prior merge pixels or
    /// prior merge hit geometry.
    pub(crate) fn classify(committed: Option<&Chrome>, inputs: &FrameInputs) -> Self {
        let Some(committed) = committed else {
            return MergeImpact::Changed;
        };
        if committed.merges().digest() != inputs.merges().digest() {
            return MergeImpact::Changed;
        }
        if committed
            .merges()
            .intersects_visible(committed.grid_layout())
        {
            return MergeImpact::Visible;
        }
        // The candidate visible area is not built yet, so approximate it from
        // the committed frame's band lengths and the captured scroll origin.
        // See `axis_window` for the deliberate over-approximation.
        let rows = &committed.pane_set.rows;
        let cols = &committed.pane_set.cols;
        let (r1, r2) = axis_window(
            rows.frozen.len() as i32,
            rows.scroll.len() as i32,
            committed.pane_set.top_row(),
            scroll_first(inputs.frozen_rows(), inputs.view().top_row),
        );
        let (c1, c2) = axis_window(
            cols.frozen.len() as i32,
            cols.scroll.len() as i32,
            committed.pane_set.left_column(),
            scroll_first(inputs.frozen_cols(), inputs.view().left_column),
        );
        if inputs.merges().intersects_rect(RCRange { r1, c1, r2, c2 }) {
            return MergeImpact::Visible;
        }
        MergeImpact::None
    }
}

/// The one merge operation that needs committed frame geometry. It lives at
/// the geometry boundary rather than with the stored values: the merge table
/// stays engine- and layout-free, and the resolved layout is applied here,
/// where `Chrome` is already in scope.
impl MergeTable {
    /// True when any merge overlaps a cell of `layout`. Both the committed and
    /// the candidate layout are probed this way, so scrolling into a merge and
    /// scrolling out of one both trigger a rebuild.
    pub fn intersects_visible(&self, layout: GridLayout) -> bool {
        layout
            .segments()
            .any(|segment| self.intersects_rect(segment.range()))
    }
}

/// Inclusive id window visible on one axis, as `(first, last)`: the frozen
/// band `1..=frozen_len` plus the scroll band. The committed scroll band is
/// exact; the candidate band is derived from the captured scroll origin and
/// may differ from the committed one by a row or column. Both bands are
/// widened by one id at the far edge, deliberately: a false `Visible` only
/// costs a rebuild, while a missed one could let a `Blit` shift stale merge
/// pixels into view.
fn axis_window(
    frozen_len: i32,
    scroll_len: i32,
    committed_top: i32,
    candidate_top: i32,
) -> (i32, i32) {
    let mut first = committed_top;
    let mut last = committed_top + scroll_len.max(1);
    if frozen_len > 0 {
        first = first.min(1);
        last = last.max(frozen_len);
    }
    first = first.min(candidate_top);
    last = last.max(candidate_top + scroll_len.max(1));
    (first, last)
}
