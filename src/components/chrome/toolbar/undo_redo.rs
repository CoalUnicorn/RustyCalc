//! Undo / Redo toolbar buttons. Reads the shared `Memo<(can_undo, can_redo)>`
//! provided by `Toolbar` rather than recomputing the pair per button.

use leptos::prelude::*;

use super::icon::{Icon, IconName};
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn UndoRedo() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let undo_redo_state = expect_context::<Memo<(bool, bool)>>();

    let can_undo = move || undo_redo_state.with(|(undo, _)| *undo);
    let can_redo = move || undo_redo_state.with(|(_, redo)| *redo);

    let on_undo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::undo(), model, &state);
        refocus_workbook();
    };
    let on_redo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::redo(), model, &state);
        refocus_workbook();
    };

    view! {
        <button
            class="tb-btn"
            title="Undo (Ctrl+Z)"
            disabled=move || !can_undo()
            on:click=on_undo
        >
            <Icon name=IconName::Undo />
        </button>
        <button
            class="tb-btn"
            title="Redo (Ctrl+Y)"
            disabled=move || !can_redo()
            on:click=on_redo
        >
            <Icon name=IconName::Redo />
        </button>
    }
}
