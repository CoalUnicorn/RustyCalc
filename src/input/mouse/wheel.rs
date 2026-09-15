//! `handle_wheel`: vertical scroll = page scroll; horizontal = one
//! column at a time.

use leptos::prelude::*;

use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::model::{ArrowKey, Navigator, PageDir};
use crate::state::{ModelStore, WorkbookState};

/// page scroll. Horizontal deltas scroll left/right by one column.
pub fn handle_wheel(ev: web_sys::WheelEvent, model: ModelStore, state: WorkbookState) {
    ev.prevent_default();
    let dy = ev.delta_y();
    let dx = ev.delta_x();
    model.update_value(|m| {
        if dx.abs() > dy.abs() {
            // Predominantly horizontal — trackpad swipe.
            if dx > 0.0 {
                m.nav_arrow(ArrowKey::Right);
            } else {
                m.nav_arrow(ArrowKey::Left);
            }
        } else if dy.abs() < 100.0 {
            // Small vertical delta — single-row scroll (trackpad).
            if dy > 0.0 {
                m.nav_arrow(ArrowKey::Down);
            } else {
                m.nav_arrow(ArrowKey::Up);
            }
        } else {
            // Large vertical delta — page scroll (mouse wheel).
            if dy > 0.0 {
                m.nav_page(PageDir::Down);
            } else {
                m.nav_page(PageDir::Up);
            }
        }
    });
    // Wheel navigation moves the selection (it routes through nav_arrow /
    // nav_page), so the viewport has to follow it like any other navigation.
    state.scroll_into_view.set_value(true);
    let (sheet, top_row, left_col) = model.with_value(|m| {
        let v = m.get_selected_view();
        (v.sheet, v.top_row, v.left_column)
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::ViewportScrolled {
            sheet,
            top_row,
            left_col,
        },
    ));
}
