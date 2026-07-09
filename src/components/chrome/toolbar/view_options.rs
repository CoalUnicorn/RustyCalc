//! View-tab toggles: row/column header visibility and gridline visibility.

use leptos::prelude::*;

use crate::events::{FormatEvent, SpreadsheetEvent};
use crate::input::error::FormatError;
use crate::model::{EvaluationMode, try_mutate};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
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

/// Gridline visibility toggle. Unlike headers, this is a persisted per-sheet
/// IronCalc property (`showGridLines`), so it reads/writes the model and lets
/// the renderer's `cache_show_grid` pick the flag up on the next repaint.
#[component]
pub fn GridLinesToggle() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Re-fires on format (this toggle) and navigation (sheet switch) so the
    // label always reflects the active sheet's flag.
    let visible = Memo::new(move |_| {
        let _ = state.events.format.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| {
            m.get_show_grid_lines(m.get_selected_sheet())
                .unwrap_or(true)
        })
    });

    let on_toggle = move |_: web_sys::MouseEvent| {
        let result = try_mutate(model, EvaluationMode::Deferred, |m| {
            let sheet = m.get_selected_sheet();
            let cur = m.get_show_grid_lines(sheet).unwrap_or(true);
            m.set_show_grid_lines(sheet, !cur)
                .map_err(FormatError::Engine)
        });
        if let Err(e) = result {
            state.status.set(Some(StatusMessage::Error(e.to_string())));
            refocus_workbook();
            return;
        }
        state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
            sheet: model.with_value(|m| m.get_selected_view().sheet),
            col: None,
            row: None,
        }));
        refocus_workbook();
    };

    view! {
        <button class="tb-btn" title="Show or hide gridlines" on:click=on_toggle>
            {move || if visible.get() { "☑ Gridlines" } else { "☐ Gridlines" }}
        </button>
    }
}
