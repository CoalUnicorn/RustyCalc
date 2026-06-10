//! Freeze-panes toggle. Freezes rows/cols above and left of the active cell;
//! clicking again unfreezes. Carries its own layout memo rather than reading
//! the shared `ToolbarState`.

use leptos::prelude::*;

use super::icon::{Icon, SheetIcon};
use crate::events::*;
use crate::input::error::FormatError;
use crate::model::{EvaluationMode, ActiveCellQuery, try_mutate};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn FreezePane() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let is_frozen = Memo::new(move |_| {
        let _ = state.events.format.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| m.frozen_panes().is_frozen())
    });

    let on_freeze = move |_: web_sys::MouseEvent| {
        let result = try_mutate(
            model,
            EvaluationMode::Deferred,
            |m| -> Result<(), FormatError> {
                let sheet = m.get_selected_sheet();
                let fp = m.frozen_panes();
                if fp.is_frozen() {
                    m.set_frozen_rows_count(sheet, 0)
                        .map_err(FormatError::Engine)?;
                    m.set_frozen_columns_count(sheet, 0)
                        .map_err(FormatError::Engine)?;
                } else {
                    let row = m.get_selected_view().row;
                    let col = m.get_selected_view().column;
                    if row > 1 || col > 1 {
                        m.set_frozen_rows_count(sheet, (row - 1).max(0))
                            .map_err(FormatError::Engine)?;
                        m.set_frozen_columns_count(sheet, (col - 1).max(0))
                            .map_err(FormatError::Engine)?;
                    }
                }
                Ok(())
            },
        );
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
        <button
            class=move || if is_frozen.get() { "tb-btn active" } else { "tb-btn" }
            title=move || if is_frozen.get() {
                "Unfreeze panes"
            } else {
                "Freeze panes above and left of active cell"
            }
            on:click=on_freeze
        >
            <Icon icon=SheetIcon::Freeze />
        </button>
    }
}
