//! In-cell `<textarea>` overlay for formula / value editing.
//!
//! Mounts only while `WorkbookState.editing_cell` is `Some`. Owns:
//!
//! - the `<Show>` mount gate,
//! - the positioned `<textarea>` element (style computed from the selected
//!   cell rect),
//! - the auto-focus effect that runs only on `EditFocus::Cell` transitions
//!   (clicks or printable keys) — the formula bar owns its own focus.
//!
//! State-sync (on_input, on_keydown) is delegated to
//! [`crate::input::edit_sync`] so this component stays a thin DOM wrapper.
//! Point mode reads `edit.cursor` after every keystroke, so we must update
//! it atomically with `text` — which is exactly what `sync_edit` guarantees.

use leptos::prelude::*;

use crate::canvas::{cell_rect_at, selected_cell_rect};
use crate::input::edit_sync::{read_value_and_cursor, suppress_navigation_defaults, sync_edit};
use crate::model::FrontendModel;
use crate::state::{EditFocus, ModelStore, WorkbookState};

#[component]
pub fn FormulaTextArea() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Track only the `EditFocus` variant — not the text buffer — so this
    // memo stays stable while the user types and the auto-focus Effect
    // below fires only on focus *transitions*, not every keystroke.
    let focus_state = Memo::new(move |_| state.editing_cell.get().map(|e| e.focus));

    // Formula-bar-initiated edits already hold focus; stealing it back would
    // break the caret. Only seize focus when the edit was started on the cell
    // itself (click or printable key with `EditFocus::Cell`).
    Effect::new(move |_| {
        let Some(focus) = focus_state.get() else {
            return;
        };
        if focus != EditFocus::Cell {
            return;
        }
        let Some(ta) = state.cell_editor_ref.get() else {
            return;
        };
        ta.focus().ok();
        let len = ta.value().len() as u32;
        ta.set_selection_range(len, len).ok();
    });

    let cell_style = move || {
        let _ = state.events.navigation.get();
        // Use the editing cell's address, not the live cursor. During point-mode
        // navigation the cursor moves to referenced cells, but the textarea must
        // stay anchored to the cell where the edit started.
        let addr = state.editing_cell.get().map(|e| (e.address.row, e.address.column));
        let r = model.with_value(|m| match addr {
            Some((row, col)) => cell_rect_at(m, row, col),
            None => selected_cell_rect(m),
        });
        format!(
            "left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px;",
            r.x, r.y, r.width, r.height,
        )
    };

    let textarea_class = move || match state.editing_cell.get() {
        Some(e) if e.text.starts_with('=') && e.formula_analysis.has_any_error() => {
            "ce formula-error"
        }
        _ => "ce",
    };

    let text_value = move || state.editing_cell.get().map(|e| e.text).unwrap_or_default();

    let on_input = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some((value, cursor)) = read_value_and_cursor(&target) else {
            return;
        };
        let sheet_names = model.with_value(|m| m.get_sheet_names());
        sync_edit(state.editing_cell, value, cursor, &sheet_names);
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| suppress_navigation_defaults(&ev);

    view! {
        <Show when=move || state.editing_cell.get().is_some()>
            <textarea
                node_ref=state.cell_editor_ref
                class=textarea_class
                style=cell_style
                prop:value=text_value
                on:input=on_input
                on:keydown=on_keydown
            />
        </Show>
    }
}
