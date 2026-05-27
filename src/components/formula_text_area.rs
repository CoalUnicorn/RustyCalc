//! In-cell `<textarea>` overlay for formula / value editing.
//!
//! Mounts only while `WorkbookState.editing_cell` is `Some`. Owns:
//!
//! - the `<Show>` mount gate,
//! - the positioned `<textarea>` element (style computed from the selected
//!   cell rect),
//! - a sibling [`FormulaOverlay`] that renders colored ref tokens behind
//!   the transparent textarea text,
//! - the auto-focus effect that runs only on `EditFocus::Cell` transitions
//!   (clicks or printable keys) — the formula bar owns its own focus.
//!
//! State-sync (on_input, on_keydown) is delegated to
//! [`crate::input::edit_sync`] so this component stays a thin DOM wrapper.
//! Point mode reads `edit.cursor` after every keystroke, so we must update
//! it atomically with `text` — which is exactly what `sync_edit` guarantees.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::formula_overlay::FormulaOverlay;
use crate::input::edit_sync::{read_value_and_cursor, suppress_navigation_defaults, sync_edit};
use crate::input::mouse::CanvasHandle;
use crate::model::SheetQuery;
use crate::model::frontend_model::DefinedNameManager;
use crate::state::{EditFocus, ModelStore, WorkbookState};
use iron_canvas_core::PixelRect;

#[component]
pub fn FormulaTextArea() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let canvas_handle = expect_context::<CanvasHandle>();
    // Cache the overlay element so on_scroll doesn't query_selector at 60 Hz.
    let overlay_ref: NodeRef<leptos::html::Div> = NodeRef::new();

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

    let host_style = move || {
        let _ = state.events.navigation.get();
        // Use the editing cell's address, not the live cursor. During point-mode
        // navigation the cursor moves to referenced cells, but the textarea must
        // stay anchored to the cell where the edit started.
        let (row, column) = state
            .editing_cell
            .get()
            .map(|e| (e.address.row, e.address.column))
            .unwrap_or_else(|| {
                let view = model.with_value(|m| m.get_selected_view());
                (view.row, view.column)
            });
        let rect = canvas_handle
            .with_value(|slot| slot.as_ref().and_then(|ic| ic.cell_rect(row, column)))
            .unwrap_or(PixelRect::default());
        format!("{}", rect)
    };

    let is_error = move || {
        state
            .editing_cell
            .get()
            .map(|e| e.formula_analysis.has_any_error())
            .unwrap_or(false)
    };

    let text_value = move || state.editing_cell.get().map(|e| e.text).unwrap_or_default();

    let overlay_text =
        Signal::derive(move || state.editing_cell.get().map(|e| e.text).unwrap_or_default());
    let overlay_refs = Signal::derive(move || {
        state
            .editing_cell
            .get()
            .map(|e| e.formula_analysis.refs().to_vec())
            .unwrap_or_default()
    });

    let on_input = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some((value, cursor)) = read_value_and_cursor(&target) else {
            return;
        };
        let sheet_names = model.with_value(|m| m.get_sheet_names());
        let defined_names = model.with_value(|m| m.get_defined_names());
        sync_edit(
            state.editing_cell,
            value,
            cursor,
            &sheet_names,
            &defined_names,
        );
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| suppress_navigation_defaults(&ev);

    // Keep the overlay's scroll position glued to the textarea's. Scoped to
    // the immediate previous sibling so multiple hosts never cross-fire.
    let on_scroll = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some(ta) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() else {
            return;
        };
        if let Some(overlay) = overlay_ref.get() {
            overlay.set_scroll_top(ta.scroll_top());
            overlay.set_scroll_left(ta.scroll_left());
        }
    };

    view! {
        <Show when=move || state.editing_cell.get().is_some()>
            <div
                class="fe-host fe-host--cell"
                class:formula-error=is_error
                style=host_style
            >
                <FormulaOverlay node_ref=overlay_ref text=overlay_text refs=overlay_refs multiline=true />
                <textarea
                    node_ref=state.cell_editor_ref
                    class="ce fe-text"
                    prop:value=text_value
                    on:input=on_input
                    on:keydown=on_keydown
                    on:scroll=on_scroll
                />
            </div>
        </Show>
    }
}
