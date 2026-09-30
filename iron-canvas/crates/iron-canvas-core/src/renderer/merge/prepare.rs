//! Merge preparation: every model read completes before any painter op.

use crate::CanvasModel;
use crate::address::{CellCoord, RCRange};
use crate::chrome::{Chrome, GridLayout};
use crate::model::fetched::Fetched;
use crate::model::sheet::merges::MergedRange;
use crate::painter::Painter;
use crate::renderer::RendererCore;
use crate::renderer::prepared::SegmentData;
use crate::style::{CellDecoration, CellKind, CellStyle};

use super::borders::build_perimeter;
use super::geometry::{ExtentCache, intersect, merge_geometry};
use super::{MergePerimeter, PreparedMerge};

/// No fetched segment buffers: the overlay path reads the model directly.
pub(super) const NO_SEGMENTS: [Option<SegmentData>; 4] = [None, None, None, None];

/// The anchor's paint inputs, resolved once per prepared merge.
pub(super) struct AnchorContent {
    style: CellStyle,
    value: String,
    cell_type: CellKind,
    decoration: Option<CellDecoration>,
}

/// The dense-buffer index of `(row, col)` in whichever prepared segment paints
/// it, or `None` when no segment does.
pub(super) fn buffered_anchor_content(
    segments: &[Option<SegmentData>; 4],
    anchor: CellCoord,
) -> Option<AnchorContent> {
    for data in segments.iter().flatten() {
        let range = data.segment.range();
        if !range.contains(anchor.row, anchor.col) {
            continue;
        }
        let cols = range.c2 - range.c1 + 1;
        let idx = ((anchor.row - range.r1) * cols + (anchor.col - range.c1)) as usize;
        let fetched = &data.fetched;
        return Some(AnchorContent {
            style: fetched
                .styles()
                .get(idx)
                .and_then(Fetched::value_ref)
                .cloned()
                .unwrap_or_default(),
            value: fetched
                .values()
                .get(idx)
                .and_then(Fetched::value_ref)
                .cloned()
                .unwrap_or_default(),
            cell_type: fetched
                .cell_types()
                .get(idx)
                .and_then(Fetched::value_ref)
                .copied()
                .unwrap_or(CellKind::Text),
            decoration: fetched
                .decorations()
                .get(idx)
                .and_then(Fetched::value_ref)
                .cloned(),
        });
    }
    None
}

/// The fetched style for `(row, col)`, when that cell is visible in one of the
/// prepared segments.
pub(super) fn visible_style(
    segments: &[Option<SegmentData>; 4],
    row: i32,
    col: i32,
) -> Option<&Fetched<CellStyle>> {
    for data in segments.iter().flatten() {
        let range = data.segment.range();
        if range.contains(row, col) {
            let cols = range.c2 - range.c1 + 1;
            let idx = ((row - range.r1) * cols + (col - range.c1)) as usize;
            return data.fetched.styles().get(idx);
        }
    }
    None
}

impl<P: Painter> RendererCore<P> {
    /// Prepare every merge for paint against `frame`, reading the model once.
    /// Returns `None` on any failed read, so the whole attempt holds.
    pub(crate) fn prepare_merges(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        layout: GridLayout,
        segments: &[Option<SegmentData>; 4],
    ) -> Option<Vec<PreparedMerge>> {
        if frame.merges().is_empty() {
            return Some(Vec::new());
        }
        let mut extents = ExtentCache::default();
        let mut prepared = Vec::new();
        for merge in frame.merges().iter() {
            // A merge with no in-frame cell is skipped before any model read:
            // the extent walk is O(height + width) and the anchor read is a
            // bridge crossing, and neither has an answer to contribute.
            if !layout
                .segments()
                .any(|segment| intersect(merge.range, segment.range()).is_some())
            {
                continue;
            }
            prepared.push(self.prepare_one_merge(
                model,
                frame,
                layout,
                segments,
                &mut extents,
                merge,
            )?);
        }
        Some(prepared)
    }

    fn prepare_one_merge(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        layout: GridLayout,
        segments: &[Option<SegmentData>; 4],
        extents: &mut ExtentCache,
        merge: &MergedRange,
    ) -> Option<PreparedMerge> {
        let (fragments, logical_rect) = merge_geometry(model, frame, layout, extents, merge.range)?;

        // Anchor content. A transient failure holds the attempt: painting a
        // merge with fabricated content would cover the covered cells with
        // wrong pixels.
        let anchor = merge.anchor;
        let content = self.anchor_content(model, frame, segments, anchor)?;

        Some(PreparedMerge {
            logical_rect,
            fragments,
            style: content.style,
            value: content.value,
            cell_type: content.cell_type,
            decoration: content.decoration,
            link: frame.links().get(anchor.row, anchor.col).cloned(),
            borders: build_perimeter(frame, &self.color_intern, segments, merge.range),
        })
    }

    /// Prepare one merge for the **overlay** surface.
    ///
    /// The overlay repaints the active cell on top of the selection tint, and
    /// the grid surface was painted earlier on its own surface, so there are no
    /// fetched segment buffers here: the anchor's content comes from the
    /// model's single-cell accessors, and the perimeter has to be resolved from
    /// the model too. Geometry is the committed frame's, so the restored cell
    /// covers exactly the pixels the grid painted.
    ///
    /// Returns `None` on a failed read; the caller then leaves the grid's own
    /// pixels showing rather than painting a partial cell over them.
    pub(crate) fn prepare_overlay_merge(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        merge: &MergedRange,
    ) -> Option<PreparedMerge> {
        let layout = frame.grid_layout();
        let mut extents = ExtentCache::default();
        let (fragments, logical_rect) =
            merge_geometry(model, frame, layout, &mut extents, merge.range)?;
        let anchor = merge.anchor;
        // No segment buffers on the overlay path, so `anchor_content` takes its
        // scalar fallback by construction.
        let content = self.anchor_content(model, frame, &NO_SEGMENTS, anchor)?;
        let mut prepared = PreparedMerge {
            logical_rect,
            fragments,
            style: content.style,
            value: content.value,
            cell_type: content.cell_type,
            decoration: content.decoration,
            link: frame.links().get(anchor.row, anchor.col).cloned(),
            borders: MergePerimeter {
                top: Vec::new(),
                bottom: Vec::new(),
                left: Vec::new(),
                right: Vec::new(),
            },
        };
        prepared.borders = self.overlay_perimeter(model, frame, &prepared.fragments)?;
        Some(prepared)
    }

    /// Content of the merge's anchor, preferring the already-fetched segment
    /// buffers.
    ///
    /// The buffers hold exactly what the scalar accessors would return, so a
    /// merge whose anchor is visible costs no bridge crossing — the dense pane
    /// fetch must not degrade into per-cell reads (the browser harness asserts
    /// that bound). Only an anchor outside every painted segment — a merge
    /// scrolled partly out of view — falls back to the scalar accessors, which
    /// is the case the buffers cannot answer.
    fn anchor_content(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        segments: &[Option<SegmentData>; 4],
        anchor: CellCoord,
    ) -> Option<AnchorContent> {
        if let Some(content) = buffered_anchor_content(segments, anchor) {
            return Some(content);
        }
        let sheet = frame.sheet;
        let value = model.get_formatted_cell_value(sheet, anchor.row, anchor.col);
        let cell_type = model.get_cell_type(sheet, anchor.row, anchor.col);
        let style = model.get_cell_style(sheet, anchor.row, anchor.col);
        let decoration = model.get_extended_cell_style(sheet, anchor.row, anchor.col);
        if value.is_bridge_failed()
            || cell_type.is_bridge_failed()
            || style.is_bridge_failed()
            || decoration.is_bridge_failed()
        {
            return None;
        }
        self.trace_fetch(RCRange::from_cell(anchor.row, anchor.col));
        Some(AnchorContent {
            style: style.value().unwrap_or_default(),
            value: value.value().unwrap_or_default(),
            cell_type: cell_type.value().unwrap_or(CellKind::Text),
            decoration: decoration.value(),
        })
    }
}
