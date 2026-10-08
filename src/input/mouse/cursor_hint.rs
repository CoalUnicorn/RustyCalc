//! Cursor hint computation for idle hover.
//!
//! `compute_cursor_hint` mirrors `handle_mousedown`'s hit-test priority
//! exactly so the cursor previews which mousedown branch would fire.
//! When you change priorities in `mousedown.rs`, update both files.
//!
//! It also reports whether the hit cell carries a committed link, so one
//! probe feeds both the cursor class and the hover tooltip.

use iron_canvas::{RefCorner, RefZone};
use iron_canvas_core::scene_geometry::{GridHit, GridRange, GridResize};
use iron_canvas_core::{Point, Side};
use leptos::prelude::WithValue;

use crate::coord::CellArea;
use crate::scene::SceneHandle;
use crate::state::{CursorHint, ModelStore, WorkbookState};

use super::formula_ref::draggable_ref_indices;
use super::{CanvasHandle, with_canvas};

/// Pixel tolerance for column/row resize hit-test in the header area.
pub(super) const HIT_ZONE: i32 = 4;

/// Slack around the autofill handle square so the crosshair preview and the
/// grab both trigger on the same pixels.
const AUTOFILL_PAD: i32 = 3;

/// One idle-hover probe: the cursor style a mousedown here would start, plus
/// the link cell under the pointer (if any) for the hover tooltip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HoverHint {
    pub cursor: CursorHint,
    pub link_cell: Option<(i32, i32)>,
}

impl HoverHint {
    /// A probe with no link cell — every non-`Cell` hit.
    fn plain(cursor: CursorHint) -> Self {
        Self {
            cursor,
            link_cell: None,
        }
    }
}

/// Whether `point` falls on the selection's bottom-right autofill handle.
///
/// The scene geometry does not classify the handle (it is not part of the
/// grid hit surface), so the host derives it from the committed handle square
/// plus a small grab pad.
pub(super) fn autofill_hit(handle: &SceneHandle, model: ModelStore, point: Point) -> bool {
    let range: GridRange = model.with_value(|m| CellArea::from(m.get_selected_view().range).into());
    let Some(rect) = handle.autofill_handle_rect(range) else {
        return false;
    };
    point.x >= rect.left() - AUTOFILL_PAD
        && point.x < rect.right() + AUTOFILL_PAD
        && point.y >= rect.top() - AUTOFILL_PAD
        && point.y < rect.bottom() + AUTOFILL_PAD
}

pub(super) fn compute_cursor_hint(
    icv: CanvasHandle,
    model: ModelStore,
    x: f64,
    y: f64,
    draggable_refs: &[usize],
) -> HoverHint {
    let point = Point {
        x: x as i32,
        y: y as i32,
    };
    if let Some(target) = with_canvas(icv, |h| h.resize_target(point, HIT_ZONE)).flatten() {
        return HoverHint::plain(match target {
            GridResize::Column(_) => CursorHint::ColResize,
            GridResize::Row(_) => CursorHint::RowResize,
        });
    }
    if let Some(hit) = with_canvas(icv, |h| h.formula_ref_hit_test(point, draggable_refs)).flatten()
    {
        return HoverHint::plain(ref_zone_hint(hit.zone));
    }
    match with_canvas(icv, |h| h.hit_test(point)).flatten() {
        Some(GridHit::Cell(coord)) => {
            if with_canvas(icv, |h| autofill_hit(h, model, point)).unwrap_or(false) {
                return HoverHint::plain(CursorHint::Autofill);
            }
            // Resolve the *logical* cell: a merged range is one cell whose
            // anchor owns the link, so probing the physical address would miss
            // a link the user can plainly see under the pointer. The reported
            // cell stays the physical one — the tooltip is positioned against
            // what the pointer is over, and drag/resize keep physical coords.
            let linked = with_canvas(icv, |h| {
                h.display_cell_at(point)
                    .is_some_and(|cell| cell.link.is_some())
            })
            .unwrap_or(false);
            HoverHint {
                cursor: if linked {
                    CursorHint::Pointer
                } else {
                    CursorHint::Cell
                },
                link_cell: linked.then_some((coord.row, coord.col)),
            }
        }
        Some(GridHit::ColumnHeader(_) | GridHit::RowHeader(_) | GridHit::Corner) | None => {
            HoverHint::plain(CursorHint::Cell)
        }
    }
}

fn ref_zone_hint(zone: RefZone) -> CursorHint {
    match zone {
        RefZone::Body => CursorHint::RefMove,
        RefZone::Edge(Side::Top | Side::Bottom) => CursorHint::RefExtendNS,
        RefZone::Edge(Side::Left | Side::Right) => CursorHint::RefExtendEW,
        RefZone::Corner(RefCorner::TopLeft | RefCorner::BottomRight) => CursorHint::RefCornerNwse,
        RefZone::Corner(RefCorner::TopRight | RefCorner::BottomLeft) => CursorHint::RefCornerNesw,
    }
}

/// One idle-hover probe resolved against the committed frame.
///
/// Publishes the cursor class, the link cell under the pointer, and the
/// position a later revalidation re-probes from. Callers that already know the
/// pointer is idle use this; drag handling keeps its own priority.
///
/// The pointer position is stored before the probe, so a revalidation after a
/// commit re-probes the same position even when the pointer has not moved.
pub(crate) fn set_hover_probe(
    state: WorkbookState,
    model: ModelStore,
    icv: CanvasHandle,
    x: f64,
    y: f64,
) {
    state.hover_pointer.set(Some((x, y)));
    let draggable_refs = state
        .editing_cell
        .get_untracked()
        .map(|edit| draggable_ref_indices(edit.formula_analysis.refs()))
        .unwrap_or_default();
    let probe = compute_cursor_hint(icv, model, x, y, &draggable_refs);
    if state.hover_cursor.get_untracked() != probe.cursor {
        state.hover_cursor.set(probe.cursor);
    }
    if state.hover_link.get_untracked() != probe.link_cell {
        state.hover_link.set(probe.link_cell);
    }
}

/// Drop the hover: the pointer is no longer over the grid, so no revalidation
/// can apply until the next move.
pub(crate) fn clear_hover(state: WorkbookState) {
    state.hover_pointer.set(None);
    if state.hover_link.get_untracked().is_some() {
        state.hover_link.set(None);
    }
}

/// Re-probe the stored hover position against the frame that just committed.
///
/// A commit changes what an unmoved pointer sits over: a scroll moves the
/// cells, a sheet switch replaces them, and an edit adds or removes the link.
/// Both the cursor class and the hovered link cell are derived from committed
/// state, so both are re-derived here rather than from the input event that
/// scheduled the paint.
pub(crate) fn revalidate_hover(state: WorkbookState, model: ModelStore, icv: CanvasHandle) {
    match state.hover_pointer.get_untracked() {
        Some((x, y)) => set_hover_probe(state, model, icv, x, y),
        None => clear_hover(state),
    }
}
