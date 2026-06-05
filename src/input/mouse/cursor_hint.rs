//! Cursor hint computation for idle hover.
//!
//! `compute_cursor_hint` mirrors `handle_mousedown`'s hit-test priority
//! exactly so the cursor previews which mousedown branch would fire.
//! When you change priorities in `mousedown.rs`, update both files.

use iron_canvas_core::types::ui::{HitTest, RectCorner, RefZone, ResizeTarget, Side};

use crate::state::CursorHint;

use super::{CanvasHandle, with_canvas};

pub(super) fn compute_cursor_hint(icv: CanvasHandle, x: f64, y: f64) -> CursorHint {
    if let Some(target) = with_canvas(icv, |ic| ic.resize_handle_at(x, y, HIT_ZONE)).flatten() {
        return match target {
            ResizeTarget::ColumnEdge(_) => CursorHint::ColResize,
            ResizeTarget::RowEdge(_) => CursorHint::RowResize,
        };
    }
    match with_canvas(icv, |ic| ic.hit_test(x, y)).unwrap_or(HitTest::Outside) {
        HitTest::AutofillHandle { .. } => CursorHint::Autofill,
        HitTest::FormulaRef { zone, .. } => ref_zone_hint(zone),
        HitTest::Cell { .. }
        | HitTest::ColumnHeader(_)
        | HitTest::RowHeader(_)
        | HitTest::Corner
        | HitTest::Outside => CursorHint::Cell,
    }
}

/// `Body` → whole-range move; opposite-side `Edge`s share an axis
/// (top/bottom = NS, left/right = EW); diagonal `Corner` pairs share
/// a slope (TL↔BR = NWSE, TR↔BL = NESW).
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
