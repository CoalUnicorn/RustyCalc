//! View-tab toggles. Currently just the row/column header visibility switch.

use leptos::prelude::*;

use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn ShowHeadersToggle() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let on_toggle = move |_: web_sys::MouseEvent| {
        state.show_headers.set(!state.show_headers.get_untracked());
        state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
            sheet: model.with_value(|m| m.get_selected_view().sheet),
            col: None,
            row: None,
        }));
        refocus_workbook();
    };

    view! {
        <button class="tb-btn" title="Show row & column headers" on:click=on_toggle>
            {move || if state.show_headers.get() { "☑ Headers" } else { "☐ Headers" }}
        </button>
    }
}
