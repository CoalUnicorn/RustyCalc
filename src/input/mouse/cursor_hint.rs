//! Cursor hint computation for idle hover.
//!
//! `compute_cursor_hint` mirrors `handle_mousedown`'s hit-test priority
//! exactly so the cursor previews which mousedown branch would fire.
//! When you change priorities in `mousedown.rs`, update both files.
//!
//! It also reports whether the hit cell carries a committed link, so one
//! probe feeds both the cursor class and the hover tooltip.

use iron_canvas_core::chrome::hit::{HitTest, RefZone, ResizeTarget};
use iron_canvas_core::geometry::prim::{RectCorner, Side};

use crate::state::{CursorHint, WorkbookState};

use super::{CanvasHandle, with_canvas};

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

pub(super) fn compute_cursor_hint(icv: CanvasHandle, x: f64, y: f64) -> HoverHint {
    if let Some(target) = with_canvas(icv, |ic| ic.resize_handle_at(x, y, HIT_ZONE)).flatten() {
        return HoverHint::plain(match target {
            ResizeTarget::ColumnEdge(_) => CursorHint::ColResize,
            ResizeTarget::RowEdge(_) => CursorHint::RowResize,
        });
    }
    match with_canvas(icv, |ic| ic.hit_test(x, y)).unwrap_or(HitTest::Outside) {
        HitTest::AutofillHandle { .. } => HoverHint::plain(CursorHint::Autofill),
        HitTest::FormulaRef { zone, .. } => HoverHint::plain(ref_zone_hint(zone)),
        HitTest::Cell { row, column } => {
            // Resolve the *logical* cell: a merged range is one cell whose
            // anchor owns the link, so probing the physical address would miss
            // a link the user can plainly see under the pointer. The reported
            // cell stays the physical one — the tooltip is positioned against
            // what the pointer is over, and drag/resize keep physical coords.
            let linked = with_canvas(icv, |ic| ic.display_cell_at(x, y))
                .flatten()
                .is_some_and(|cell| cell.link.is_some());
            HoverHint {
                cursor: if linked {
                    CursorHint::Pointer
                } else {
                    CursorHint::Cell
                },
                link_cell: linked.then_some((row, column)),
            }
        }
        HitTest::ColumnHeader(_) | HitTest::RowHeader(_) | HitTest::Corner | HitTest::Outside => {
            HoverHint::plain(CursorHint::Cell)
        }
    }
}

/// `Body` -> whole-range move; opposite-side `Edge`s share an axis
/// (top/bottom = NS, left/right = EW); diagonal `Corner` pairs share
/// a slope (TL<->BR = NWSE, TR<->BL = NESW).
fn ref_zone_hint(zone: RefZone) -> CursorHint {
    match zone {
        RefZone::Body => CursorHint::RefMove,
        RefZone::Edge(Side::Top | Side::Bottom) => CursorHint::RefExtendNS,
        RefZone::Edge(Side::Left | Side::Right) => CursorHint::RefExtendEW,
        RefZone::Corner(RectCorner::TopLeft | RectCorner::BottomRight) => CursorHint::RefCornerNwse,
        RefZone::Corner(RectCorner::TopRight | RectCorner::BottomLeft) => CursorHint::RefCornerNesw,
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
pub(crate) fn set_hover_probe(state: WorkbookState, icv: CanvasHandle, x: f64, y: f64) {
    state.hover_pointer.set(Some((x, y)));
    let probe = compute_cursor_hint(icv, x, y);
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
pub(crate) fn revalidate_hover(state: WorkbookState, icv: CanvasHandle) {
    match state.hover_pointer.get_untracked() {
        Some((x, y)) => set_hover_probe(state, icv, x, y),
        None => clear_hover(state),
    }
}

/// Pixel tolerance for column/row resize hit-test in the header area.
pub(super) const HIT_ZONE: f64 = 4.0;
