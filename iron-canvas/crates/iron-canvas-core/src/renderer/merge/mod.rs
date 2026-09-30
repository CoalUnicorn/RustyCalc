//! Merged-range preparation and paint.
//!
//! A merged range is one logical cell: its anchor (top-left) owns the value,
//! style, decoration, and link, and the whole rectangle paints as a single
//! cell over the per-cell pass. The merge pass is the **last step inside**
//! `GroupClass::Cells`, after every segment's cells, so its fill covers the
//! covered cells' fills, interior grid strokes, interior explicit borders, and
//! covered-cell text. Frozen separators still paint after it.
//!
//! No suppression pass exists: painting the merge over the cells is the
//! cheaper and equally correct alternative. The wasted work is bounded by
//! (merge ∩ viewport); a later stage can suppress the covered cells' paint
//! once a raster-equivalence gate proves it.
//!
//! Preparation reads the model once — the anchor's content and every logical
//! row/column extent the layout needs — before any painter op. A failed read
//! returns `None`, so the whole attempt holds and the previous pixels and
//! query data stay installed.
//!
//! The module splits by responsibility: `geometry` derives the fragments,
//! `borders` coalesces the perimeter runs, `prepare` completes every model
//! read, and `paint` issues the painter ops.

use std::rc::Rc;

use crate::address::RCRange;
use crate::geometry::pixel_rect::PixelRect;
use crate::model::sheet::links::CellLink;
use crate::renderer::cell::borders::BorderPaint;
use crate::style::{CellDecoration, CellKind, CellStyle};

mod borders;
mod geometry;
mod paint;
mod prepare;

/// One merged range prepared for paint against one candidate frame.
pub(crate) struct PreparedMerge {
    /// Full logical rectangle in absolute canvas pixels, under the anchor's
    /// (or the first visible) segment's transform. May extend outside the
    /// canvas and outside every pane. Text layout uses this.
    pub(crate) logical_rect: PixelRect,
    /// One visible intersection per intersecting pane segment.
    pub(crate) fragments: Vec<MergeFragment>,
    /// Anchor data: fill, font, and CF decoration.
    pub(crate) style: CellStyle,
    pub(crate) value: String,
    pub(crate) cell_type: CellKind,
    pub(crate) decoration: Option<CellDecoration>,
    pub(crate) link: Option<Rc<CellLink>>,
    /// The logical perimeter, as paint runs.
    pub(crate) borders: MergePerimeter,
}

/// One merge fragment: the merge's visible intersection with one pane
/// segment's visible cell area.
pub(crate) struct MergeFragment {
    /// Intersection of the merge's logical rectangle (under this segment's
    /// transform) with the segment's visible cell area.
    pub(crate) rect: PixelRect,
    /// The merge's full logical rectangle under **this segment's** transform.
    /// Same size for every fragment of one merge, but a different origin
    /// whenever the segments' transforms differ — the frozen band and the
    /// scrolled band, including the address gap between them. Text laid out once
    /// in merge-local coordinates is translated by the difference between this
    /// and `PreparedMerge::logical_rect`, which is what keeps frozen-band and
    /// scrolled-band fragments from sharing one absolute position.
    pub(crate) logical_rect: PixelRect,
    /// Which logical sides coincide with this fragment's own sides, in
    /// `[left, top, right, bottom]` order. A fragment clipped by the viewport
    /// or by a frozen boundary has `false` on the clipped edge, which is
    /// exactly the edge that must not paint a perimeter stroke.
    pub(crate) sides: [bool; 4],
    /// The merge cells this fragment actually covers: the intersection of the
    /// merge with one layout segment. Bounded by the viewport, which is what
    /// lets the overlay path walk a side without touching every row of a merge
    /// larger than the screen.
    pub(crate) covered: RCRange,
}

/// The four logical sides of a merge, each as a coalesced run list.
pub(crate) struct MergePerimeter {
    /// Left-to-right runs along the top side.
    pub(crate) top: Vec<BorderRun>,
    /// Left-to-right runs along the bottom side.
    pub(crate) bottom: Vec<BorderRun>,
    /// Top-to-bottom runs along the left side.
    pub(crate) left: Vec<BorderRun>,
    /// Top-to-bottom runs along the right side.
    pub(crate) right: Vec<BorderRun>,
}

/// A coalesced border run along one logical side.
pub(crate) struct BorderRun {
    /// Inclusive id span along the side, in sheet coordinates.
    pub(crate) start: i32,
    pub(crate) end: i32,
    pub(crate) paint: BorderPaint,
}
