//! Merge perimeter resolution: coalescing the logical sides into paint runs.

use crate::CanvasModel;
use crate::address::RCRange;
use crate::chrome::Chrome;
use crate::geometry::prim::Side;
use crate::model::fetched::Fetched;
use crate::painter::Painter;
use crate::renderer::RendererCore;
use crate::renderer::cache::ColorIntern;
use crate::renderer::cell::borders::ResolvedBorders;
use crate::renderer::prepared::SegmentData;
use crate::theme::CanvasTheme;

use super::prepare::visible_style;
use super::{BorderRun, MergeFragment, MergePerimeter};

/// Coalesce one logical side's runs from the **model**, for the overlay path
/// where no fetched style buffer exists.
///
/// `span` is `(fixed, first, last)`: the fixed id is the side's row (horizontal
/// sides) or column (vertical sides), and the walked ids are `first..=last` —
/// always a fragment's covered span, so the walk is viewport-bounded.
pub(super) fn model_runs(
    model: &dyn CanvasModel,
    frame: &Chrome,
    intern: &ColorIntern,
    span: (i32, i32, i32),
    horizontal: bool,
    side: Side,
    runs: &mut Vec<BorderRun>,
) -> Option<()> {
    let (fixed, first, last) = span;
    let theme = &frame.theme;
    for id in first..=last {
        let (row, col) = if horizontal { (fixed, id) } else { (id, fixed) };
        let paint = match model.get_cell_style(frame.sheet, row, col) {
            Fetched::Value(style) => {
                let resolved = ResolvedBorders::resolve(&style.border, theme, intern);
                match side {
                    Side::Top => resolved.top,
                    Side::Bottom => resolved.bottom,
                    Side::Left => resolved.left,
                    Side::Right => resolved.right,
                }
            }
            Fetched::Absent => None,
            // A transient failure must hold: a fabricated perimeter would
            // stroke a boundary the model does not have.
            Fetched::BridgeFailed => return None,
        };
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
    Some(())
}

/// Coalesce the visible cells of one logical side into border runs.
///
/// `cells` walks `(row, col)` pairs along the side; `id_of` yields the sheet
/// coordinate along the side's axis. A cell whose style was not read (invisible)
/// breaks the run — a fabricated run would paint a stroke the model does not
/// have. A side with no explicit border on any visible cell produces no runs.
pub(super) fn side_runs(
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
    /// Resolve the merge's perimeter from the model, for the overlay path.
    ///
    /// The grid path reads the already-fetched segment styles; the overlay has
    /// none, so it reads perimeter cells one at a time — bounded by each
    /// fragment's own covered span, never by the merge's declared size.
    pub(super) fn overlay_perimeter(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        fragments: &[MergeFragment],
    ) -> Option<MergePerimeter> {
        let mut sides = MergePerimeter {
            top: Vec::new(),
            bottom: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
        };
        for fragment in fragments {
            let covered = fragment.covered;
            if fragment.sides[1] {
                model_runs(
                    model,
                    frame,
                    &self.color_intern,
                    (covered.r1, covered.c1, covered.c2),
                    true,
                    Side::Top,
                    &mut sides.top,
                )?;
            }
            if fragment.sides[3] {
                model_runs(
                    model,
                    frame,
                    &self.color_intern,
                    (covered.r2, covered.c1, covered.c2),
                    true,
                    Side::Bottom,
                    &mut sides.bottom,
                )?;
            }
            if fragment.sides[0] {
                model_runs(
                    model,
                    frame,
                    &self.color_intern,
                    (covered.c1, covered.r1, covered.r2),
                    false,
                    Side::Left,
                    &mut sides.left,
                )?;
            }
            if fragment.sides[2] {
                model_runs(
                    model,
                    frame,
                    &self.color_intern,
                    (covered.c2, covered.r1, covered.r2),
                    false,
                    Side::Right,
                    &mut sides.right,
                )?;
            }
        }
        Some(sides)
    }
}

/// Build the four logical sides of `range` from the already-fetched segment
/// style buffers. No model reads: an invisible perimeter cell's border is
/// unknown, so a side is described only by the cells that were read.
pub(super) fn build_perimeter(
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
