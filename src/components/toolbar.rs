use leptos::prelude::*;

use crate::model::FrontendModel;
use crate::state::{ModelStore, WorkbookState};
use crate::util::warn_if_err;

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    view! {
        <div class="toolbar">
            <FreezePane />
        </div>
    }
}

#[component]
fn FreezePane() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let is_frozen = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.frozen_panes().is_frozen())
    };

    let on_freeze = move |_: web_sys::MouseEvent| {
        model.update_value(|m| {
            let sheet = m.get_selected_view().sheet;
            let fp = m.frozen_panes();

            if fp.is_frozen() {
                // Unfreeze: set both to 0.
                warn_if_err(m.set_frozen_rows_count(sheet, 0), "set_frozen_rows_count");
                warn_if_err(
                    m.set_frozen_columns_count(sheet, 0),
                    "set_frozen_columns_count",
                );
            } else {
                // Freeze at the active cell: rows above and columns left of
                // the cursor become frozen. Freezing at row 1 col 1 would
                // freeze nothing, so treat that as a no-op.
                let row = m.get_selected_view().row;
                let col = m.get_selected_view().column;
                if row > 1 || col > 1 {
                    warn_if_err(
                        m.set_frozen_rows_count(sheet, (row - 1).max(0)),
                        "set_frozen_rows_count",
                    );
                    warn_if_err(
                        m.set_frozen_columns_count(sheet, (col - 1).max(0)),
                        "set_frozen_columns_count",
                    );
                }
            }
        });
        state.request_redraw();
        crate::util::refocus_workbook();
    };

    let freeze_class = move || {
        if is_frozen() {
            "toolbar-btn active"
        } else {
            "toolbar-btn"
        }
    };

    let freeze_title = move || {
        if is_frozen() {
            "Unfreeze panes"
        } else {
            "Freeze panes above and left of active cell"
        }
    };

    let freeze_label = move || {
        if is_frozen() {
            "╔"
        } else {
            "╬"
        }
    };

    view! {
            <button class=freeze_class title=freeze_title on:click=on_freeze>
                {freeze_label}
            </button>
    }
}
