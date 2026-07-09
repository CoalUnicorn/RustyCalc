//! Pure derivation of the header span a structural action targets.
//!
//! A click on column C affects the whole selected column range *only* when
//! C lies inside a selection that spans every row (a "full-column"
//! selection); otherwise it affects C alone. The row axis is the mirror.
//! Extracted from `contextmenu.rs` so mousedown-resize and resize-by-value
//! share one definition.

use crate::coord::CellArea;
use iron_canvas_core::geometry::constants::{LAST_COLUMN, LAST_ROW};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Col,
    Row,
}

/// Inclusive `(first, last)` span of headers a click on `idx` targets.
/// `area` is the current selection (normalized internally). Returns the
/// selection's range on `axis` when `idx` is inside a full-strip selection,
/// else `(idx, idx)`.
pub fn full_header_span(area: CellArea, idx: i32, axis: Axis) -> (i32, i32) {
    let area = area.normalized();
    match axis {
        Axis::Col => {
            if area.r2 >= LAST_ROW && area.c1 <= idx && idx <= area.c2 {
                (area.c1, area.c2)
            } else {
                (idx, idx)
            }
        }
        Axis::Row => {
            if area.c2 >= LAST_COLUMN && area.r1 <= idx && idx <= area.r2 {
                (area.r1, area.r2)
            } else {
                (idx, idx)
            }
        }
    }
}
