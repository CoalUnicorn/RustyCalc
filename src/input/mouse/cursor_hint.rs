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

use crate::state::CursorHint;

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
            let link = with_canvas(icv, |ic| ic.link_at(row, column)).flatten().is_some();
            HoverHint {
                cursor: if link {
                    CursorHint::Pointer
                } else {
                    CursorHint::Cell
                },
                link_cell: link.then_some((row, column)),
            }
        }
        HitTest::ColumnHeader(_)
        | HitTest::RowHeader(_)
        | HitTest::Corner
        | HitTest::Outside => HoverHint::plain(CursorHint::Cell),
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

/// Pixel tolerance for column/row resize hit-test in the header area.
pub(super) const HIT_ZONE: f64 = 4.0;
