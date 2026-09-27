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

use std::collections::HashMap;
use std::rc::Rc;

use crate::CanvasModel;
use crate::address::{CellCoord, RCRange};
use crate::chrome::{Chrome, GridLayout, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Point, Side};
use crate::geometry::slot::AxisSlot;
use crate::link::CellLink;
use crate::merge::MergedRange;
use crate::model::fetched::Fetched;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::cache::ColorIntern;
use crate::renderer::cell::borders::{BorderPaint, ResolvedBorders};
use crate::renderer::cell::cf::CfDecorationPaint;
use crate::renderer::cell::text::TextPaint;
use crate::renderer::prepared::SegmentData;
use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::theme::CanvasTheme;

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
    /// Which logical sides coincide with this fragment's own sides, in
    /// `[left, top, right, bottom]` order. A fragment clipped by the viewport
    /// or by a frozen boundary has `false` on the clipped edge, which is
    /// exactly the edge that must not paint a perimeter stroke.
    pub(crate) sides: [bool; 4],
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

/// Memoized row-height / column-width lookups for one preparation attempt.
///
/// The extent walks are O(height + width) per merge per attempt. A merge
/// larger than the viewport therefore costs one model read per logical row or
/// column. Indexed extent sums are the deferred optimization; a measurement
/// must justify them first.
#[derive(Default)]
struct ExtentCache {
    rows: HashMap<i32, i32>,
    cols: HashMap<i32, i32>,
}

impl ExtentCache {
    fn row_extent(&mut self, model: &dyn CanvasModel, sheet: u32, row: i32) -> Option<i32> {
        if let Some(px) = self.rows.get(&row) {
            return Some(*px);
        }
        let px = crate::geometry::slot::row_height(model, sheet, row).extent()?;
        self.rows.insert(row, px);
        Some(px)
    }

    fn col_extent(&mut self, model: &dyn CanvasModel, sheet: u32, col: i32) -> Option<i32> {
        if let Some(px) = self.cols.get(&col) {
            return Some(*px);
        }
        let px = crate::geometry::slot::col_width(model, sheet, col).extent()?;
        self.cols.insert(col, px);
        Some(px)
    }

    /// Sum of the full logical row heights `r1..=r2`, inclusive. An empty
    /// range sums to zero.
    fn row_span(
        &mut self,
        model: &dyn CanvasModel,
        sheet: u32,
        r1: i32,
        r2: i32,
    ) -> Option<i32> {
        let mut sum = 0i32;
        for row in r1..=r2 {
            sum = sum.checked_add(self.row_extent(model, sheet, row)?)?;
        }
        Some(sum)
    }

    /// Column mirror of [`Self::row_span`].
    fn col_span(
        &mut self,
        model: &dyn CanvasModel,
        sheet: u32,
        c1: i32,
        c2: i32,
    ) -> Option<i32> {
        let mut sum = 0i32;
        for col in c1..=c2 {
            sum = sum.checked_add(self.col_extent(model, sheet, col)?)?;
        }
        Some(sum)
    }
}

/// The anchor's paint inputs, resolved once per prepared merge.
struct AnchorContent {
    style: CellStyle,
    value: String,
    cell_type: CellKind,
    decoration: Option<CellDecoration>,
}

/// The dense-buffer index of `(row, col)` in whichever prepared segment paints
/// it, or `None` when no segment does.
fn buffered_anchor_content(
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

/// Inclusive intersection of two ranges, or `None` when they are disjoint.
fn intersect(a: RCRange, b: RCRange) -> Option<RCRange> {
    let a = a.normalized();
    let b = b.normalized();
    let r1 = a.r1.max(b.r1);
    let c1 = a.c1.max(b.c1);
    let r2 = a.r2.min(b.r2);
    let c2 = a.c2.min(b.c2);
    (r1 <= r2 && c1 <= c2).then_some(RCRange { r1, c1, r2, c2 })
}

/// Pixel rectangle covered by one pane segment's visible slots.
fn segment_cell_area(frame: &Chrome, region: PaneRegion) -> Option<PixelRect> {
    let rows = region.rows(frame);
    let cols = region.cols(frame);
    let first_row = rows.first()?;
    let last_row = rows.last()?;
    let first_col = cols.first()?;
    let last_col = cols.last()?;
    Some(PixelRect {
        top_left: Point {
            x: first_col.start(),
            y: first_row.start(),
        },
        width: last_col.end() - first_col.start(),
        height: last_row.end() - first_row.start(),
    })
}

/// The fetched style for `(row, col)`, when that cell is visible in one of the
/// prepared segments.
fn visible_style<'a>(
    segments: &'a [Option<SegmentData>; 4],
    row: i32,
    col: i32,
) -> Option<&'a Fetched<CellStyle>> {
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

/// Coalesce the visible cells of one logical side into border runs.
///
/// `cells` walks `(row, col)` pairs along the side; `id_of` yields the sheet
/// coordinate along the side's axis. A cell whose style was not read (invisible)
/// breaks the run — a fabricated run would paint a stroke the model does not
/// have. A side with no explicit border on any visible cell produces no runs.
fn side_runs(
    segments: &[Option<SegmentData>; 4],
    theme: &CanvasTheme,
    intern: &ColorIntern,
    cells: impl Iterator<Item = (i32, i32)>,
    id_of: impl Fn(i32, i32) -> i32,
    side: Side,
) -> Vec<BorderRun> {
    let mut runs: Vec<BorderRun> = Vec::new();
    for (row, col) in cells {
        let id = id_of(row, col);
        let paint = visible_style(segments, row, col)
            .and_then(Fetched::value_ref)
            .map(|style| ResolvedBorders::resolve(&style.border, theme, intern))
            .and_then(|resolved| match side {
                Side::Top => resolved.top,
                Side::Bottom => resolved.bottom,
                Side::Left => resolved.left,
                Side::Right => resolved.right,
            });
        let Some(paint) = paint else {
            continue;
        };
        if let Some(last) = runs.last_mut()
            && last.end + 1 == id
            && last.paint == paint
        {
            last.end = id;
            continue;
        }
        runs.push(BorderRun {
            start: id,
            end: id,
            paint,
        });
    }
    runs
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
                model, frame, layout, segments, &mut extents, merge,
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
        let sheet = frame.sheet;
        let range = merge.range;
        let row_total = extents.row_span(model, sheet, range.r1, range.r2)?;
        let col_total = extents.col_span(model, sheet, range.c1, range.c2)?;

        let mut fragments = Vec::new();
        let mut logical_rect: Option<PixelRect> = None;
        for grid_segment in layout.segments() {
            let Some(cell_range) = intersect(range, grid_segment.range()) else {
                continue;
            };
            let (row, col) = (cell_range.r1, cell_range.c1);
            // `cell_rect` is the in-frame check: the scroll band's address
            // range includes the address gap between the frozen band and the
            // scrolled-to id, and a cell in that gap has no pixels.
            let Some(cell) = frame.cell_rect(row, col) else {
                continue;
            };
            let rows_before = extents.row_span(model, sheet, range.r1, row - 1)?;
            let cols_before = extents.col_span(model, sheet, range.c1, col - 1)?;
            // The segment's own transform: a cell in the frozen band and one in
            // the scroll band resolve the same logical rectangle to different
            // pixels, so each fragment derives its own.
            let own_logical = PixelRect {
                top_left: Point {
                    x: cell.left() - cols_before,
                    y: cell.top() - rows_before,
                },
                width: col_total,
                height: row_total,
            };
            let Some(area) = segment_cell_area(frame, grid_segment.region()) else {
                continue;
            };
            let Some(rect) = own_logical.intersection(area) else {
                continue;
            };
            // The layout rectangle comes from the anchor's own segment when it
            // is visible; otherwise from the first visible fragment.
            if logical_rect.is_none() || cell_range.contains(range.r1, range.c1) {
                logical_rect = Some(own_logical);
            }
            fragments.push(MergeFragment {
                rect,
                sides: [
                    rect.left() == own_logical.left(),
                    rect.top() == own_logical.top(),
                    rect.right() == own_logical.right(),
                    rect.bottom() == own_logical.bottom(),
                ],
            });
        }

        // Anchor content. A transient failure holds the attempt: painting a
        // merge with fabricated content would cover the covered cells with
        // wrong pixels.
        let anchor = merge.anchor;
        let content = self.anchor_content(model, frame, segments, anchor)?;

        Some(PreparedMerge {
            logical_rect: logical_rect.unwrap_or(PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 0,
                height: 0,
            }),
            fragments,
            style: content.style,
            value: content.value,
            cell_type: content.cell_type,
            decoration: content.decoration,
            link: frame.links().get(anchor.row, anchor.col).cloned(),
            borders: build_perimeter(frame, &self.color_intern, segments, range),
        })
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

    /// Paint every prepared merge. The last step of the grid paint, inside
    /// `GroupClass::Cells`, after every segment's cells.
    pub(crate) fn paint_merges(&self, frame: &Chrome, merges: &[PreparedMerge]) {
        if merges.is_empty() {
            return;
        }
        let theme = &frame.theme;
        // One line buffer for the whole pass, so the pass allocates none.
        let mut text_lines = self.frame_cache.text_lines.take();
        for merge in merges {
            // Resolved once per merge and reused for every fragment: resolving
            // per fragment would allocate the data-bar color `Rc` twice.
            let decoration = merge
                .decoration
                .clone()
                .map(|deco| CfDecorationPaint::resolve(deco, &self.color_intern));
            for fragment in &merge.fragments {
                self.paint_merge_fragment(frame, merge, fragment, decoration.as_ref());
            }
            // Text is resolved once against the whole logical rectangle and
            // each fragment paints the same lines under its own clip; resolving
            // per fragment would lay the text out once per fragment and could
            // duplicate it.
            if let Some(text) = TextPaint::resolve_into(
                self,
                merge.logical_rect,
                merge.logical_rect,
                &merge.style,
                merge.value.clone(),
                merge.cell_type,
                merge.link.as_deref(),
                &mut text_lines,
            ) {
                for fragment in &merge.fragments {
                    self.painter.push_clip(fragment.rect);
                    self.paint_text(&text, theme, &text_lines);
                    self.painter.pop_clip();
                }
            }
        }
        self.frame_cache.text_lines.set(text_lines);
    }

    fn paint_merge_fragment(
        &self,
        frame: &Chrome,
        merge: &PreparedMerge,
        fragment: &MergeFragment,
        decoration: Option<&CfDecorationPaint>,
    ) {
        let theme = &frame.theme;
        // Same fill branch as `paint_bg`: a per-cell override or the theme
        // background.
        let color = match merge.style.fill_color.as_deref() {
            Some(c) => PaintColor::Borrowed(c),
            None => PaintColor::from_theme_str(&theme.cell_bg),
        };
        self.painter.rect_fill(fragment.rect, color);

        // The perimeter runs, gated on `fragment.sides` so a viewport-clipped
        // or freeze-clipped edge paints no stroke.
        self.paint_merge_side(frame, fragment, Side::Top, 1, &merge.borders.top);
        self.paint_merge_side(frame, fragment, Side::Bottom, 3, &merge.borders.bottom);
        self.paint_merge_side(frame, fragment, Side::Left, 0, &merge.borders.left);
        self.paint_merge_side(frame, fragment, Side::Right, 2, &merge.borders.right);

        if let Some(decoration) = decoration {
            decoration.paint(&*self.painter, fragment.rect);
        }
    }

    /// Paint the fragment's intersecting part of every run on one side. A run
    /// shorter than the fragment paints only its own span, and a run reaching
    /// beyond the fragment is clamped to the fragment's own rect — so two
    /// fragments of one merge never paint the same pixels twice.
    fn paint_merge_side(
        &self,
        frame: &Chrome,
        fragment: &MergeFragment,
        side: Side,
        side_index: usize,
        runs: &[BorderRun],
    ) {
        if !fragment.sides.get(side_index).copied().unwrap_or(false) {
            return;
        }
        let rect = fragment.rect;
        let horizontal = matches!(side, Side::Top | Side::Bottom);
        for run in runs {
            let subrect = if horizontal {
                let start = frame.pane_set.col_to_x(run.start);
                let end = frame.pane_set.col_to_x(run.end)
                    + frame.pane_set.col_extent_at(run.end);
                let left = start.max(rect.left());
                let right = end.min(rect.right());
                if left >= right {
                    continue;
                }
                PixelRect {
                    top_left: Point {
                        x: left,
                        y: rect.top(),
                    },
                    width: right - left,
                    height: rect.height,
                }
            } else {
                let start = frame.pane_set.row_to_y(run.start);
                let end =
                    frame.pane_set.row_to_y(run.end) + frame.pane_set.row_extent_at(run.end);
                let top = start.max(rect.top());
                let bottom = end.min(rect.bottom());
                if top >= bottom {
                    continue;
                }
                PixelRect {
                    top_left: Point {
                        x: rect.left(),
                        y: top,
                    },
                    width: rect.width,
                    height: bottom - top,
                }
            };
            self.paint_border(side, subrect, &run.paint);
        }
    }
}

/// Build the four logical sides of `range` from the already-fetched segment
/// style buffers. No model reads: an invisible perimeter cell's border is
/// unknown, so a side is described only by the cells that were read.
fn build_perimeter(
    frame: &Chrome,
    intern: &ColorIntern,
    segments: &[Option<SegmentData>; 4],
    range: RCRange,
) -> MergePerimeter {
    let theme = &frame.theme;
    MergePerimeter {
        top: side_runs(
            segments,
            theme,
            intern,
            (range.c1..=range.c2).map(|col| (range.r1, col)),
            |_, col| col,
            Side::Top,
        ),
        bottom: side_runs(
            segments,
            theme,
            intern,
            (range.c1..=range.c2).map(|col| (range.r2, col)),
            |_, col| col,
            Side::Bottom,
        ),
        left: side_runs(
            segments,
            theme,
            intern,
            (range.r1..=range.r2).map(|row| (row, range.c1)),
            |row, _| row,
            Side::Left,
        ),
        right: side_runs(
            segments,
            theme,
            intern,
            (range.r1..=range.r2).map(|row| (row, range.c2)),
            |row, _| row,
            Side::Right,
        ),
    }
}
