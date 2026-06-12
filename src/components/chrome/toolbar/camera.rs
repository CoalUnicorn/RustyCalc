//! Insert Camera: drops a floating live picture of the current selection.

use leptos::prelude::*;

use super::icon::{Icon, SheetIcon};
use crate::components::workbook::camera::{GRIP_H, MAX_H, MAX_W};
use crate::coord::SheetRange;
use crate::state::{CameraSpec, ModelStore, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn InsertCamera() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let on_click = move |_: web_sys::MouseEvent| {
        let source = model.with_value(|m| SheetRange::from_view(m));
        let area = source.area.normalized();

        // Size to the selection using the same constants the camera extractor
        // uses (iron_canvas_core::geometry::constants), clamped to a sane max.
        use iron_canvas_core::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
        let w = (area.width() as f64 * DEFAULT_COL_WIDTH).clamp(DEFAULT_COL_WIDTH, MAX_W);
        let h = (area.height() as f64 * DEFAULT_ROW_HEIGHT + GRIP_H)
            .clamp(DEFAULT_ROW_HEIGHT + GRIP_H, MAX_H);

        state.cameras.update(|cams| {
            let id = CameraSpec::next_id(cams);
            let mut spec = CameraSpec::new(id, source);
            spec.size = (w, h);
            // Stagger successive inserts so they don't fully overlap.
            spec.pos = (
                80.0 + cams.len() as f64 * 24.0,
                80.0 + cams.len() as f64 * 24.0,
            );
            cams.push(spec);
        });
        refocus_workbook();
    };

    view! {
        <button
            class="tb-btn"
            title="Insert camera - live picture of selection"
            on:click=on_click
        >
            <Icon icon=SheetIcon::Camera />
        </button>
    }
}
