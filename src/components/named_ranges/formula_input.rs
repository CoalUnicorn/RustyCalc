//! Formula `<textarea>` for the Manage Named Ranges dialog.
//!
//! Mirror of [`crate::components::formula_text_area::FormulaTextArea`] minus
//! the cell-editor concerns (canvas positioning, focus arbitration with the
//! grid, point-mode arming). Both editors share
//! [`crate::input::edit_sync::sync_edit`]: the trait bound
//! [`crate::input::edit_sync::FormulaEditState`] dispatches on the in-progress
//! state type, so analyze-on-keystroke validation behaves identically here.
//!
//! The error class reads [`crate::state::EditingDefinedName::formula_invalid`]
//! — the same predicate the Save button uses, minus the name-empty check.

use leptos::prelude::*;

use crate::input::edit_sync::{read_value_and_cursor, suppress_navigation_defaults, sync_edit};
use crate::model::FrontendModel;
use crate::state::{ModelStore, WorkbookState};

#[component]
pub fn FormulaInput() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Sheet + defined-name lists fed to `analyze_formula`. Memoised against
    // `events.content` so a keystroke doesn't re-walk the workbook.
    let analyzer_inputs = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| (m.get_sheet_names(), m.get_defined_names()))
    });

    let textarea_class = move || match state.editing_named_range.get() {
        Some(e) if e.formula_invalid() => "nrm-input nrm-input-error",
        _ => "nrm-input",
    };

    let text_value = move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.formula)
            .unwrap_or_default()
    };

    let on_input = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some((value, cursor)) = read_value_and_cursor(&target) else {
            return;
        };
        analyzer_inputs.with_untracked(move |(sheet_names, defined_names)| {
            sync_edit(
                state.editing_named_range,
                value,
                cursor,
                sheet_names,
                defined_names,
            );
        });
    };

    // Same Enter/Tab/Escape suppression as the cell editor — the dialog's
    // own buttons own commit/cancel, not the textarea default.
    let on_keydown = move |ev: web_sys::KeyboardEvent| suppress_navigation_defaults(&ev);

    view! {
        <textarea
            class=textarea_class
            prop:value=text_value
            on:input=on_input
            on:keydown=on_keydown
            rows="2"
            spellcheck="false"
            autocapitalize="off"
        />
    }
}
