// See docs/leptos-patterns.md for component conventions.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::canvas::col_name;
use crate::model::FrontendModel;
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};

/// The formula bar: cell address label + content/formula input.
///
/// Layout: `[ A1 ▾ ][ fx ][ =SUM(A1:A10)__________________ ]`
///
/// When no edit is active, the input shows the raw content of the selected cell
/// (formula text, not the computed result). Clicking or typing in the input
/// starts an edit session with `EditFocus::FormulaBar`.
///
/// The text buffer is shared with `CellEditor` via `state.editing_cell` — both
/// components read/write the same `RwSignal`, so they stay in sync.
#[component]
pub fn FormulaBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let input_ref = state.formula_input_ref;

    let cell_address = move || {
        let _ = state.get_redraw();
        model.with_value(|m| {
            let ac = m.active_cell();
            format!("{}{}", col_name(ac.column), ac.row)
        })
    };

    // While editing: live edit buffer (shared with CellEditor).
    // Otherwise: raw cell content (formula text or literal).
    let display_text = move || {
        if let Some(edit) = state.get_editing_cell() {
            return edit.text;
        }
        let _ = state.get_redraw();
        model.with_value(|m| m.active_cell_content())
    };

    let is_editing = move || state.get_editing_cell().is_some();

    // Start an edit session with FormulaBar focus (so CellEditor doesn't
    // steal focus back), or switch focus if already editing.
    let on_focus = move |_: web_sys::FocusEvent| {
        if state.get_editing_cell_untracked().is_some() {
            state.update_editing_cell(|cell| {
                if let Some(c) = cell {
                    c.focus = EditFocus::FormulaBar;
                }
            });
            return;
        }
        model.with_value(|m| {
            let ac = m.active_cell();
            let text = m.active_cell_content();
            state.set_editing_cell(Some(EditingCell {
                sheet: ac.sheet,
                row: ac.row,
                col: ac.column,
                text,
                mode: EditMode::Edit,
                focus: EditFocus::FormulaBar,
            }));
        });
    };

    // Update the shared edit buffer (syncs with CellEditor).
    let on_input = move |ev: web_sys::Event| {
        let value = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|el| el.value())
            .unwrap_or_default();

        if state.get_editing_cell_untracked().is_some() {
            state.update_editing_cell(|cell| {
                if let Some(c) = cell {
                    c.text = value;
                }
            });
        } else {
            // First keystroke — Accept mode: arrows commit + navigate.
            model.with_value(|m| {
                let ac = m.active_cell();
                state.set_editing_cell(Some(EditingCell {
                    sheet: ac.sheet,
                    row: ac.row,
                    col: ac.column,
                    text: value,
                    mode: EditMode::Accept,
                    focus: EditFocus::FormulaBar,
                }));
            });
        }
    };

    // Suppress browser defaults; let the event bubble to Workbook
    // which commits or cancels via classify_key -> execute.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if matches!(ev.key().as_str(), "Enter" | "Tab" | "Escape") {
            ev.prevent_default();
        }
    };

    let input_class = move || {
        if is_editing() {
            "formula-bar-input editing"
        } else {
            "formula-bar-input"
        }
    };

    view! {
        <div id="formula-bar" class="formula-bar">
            <div class="formula-bar-address">{cell_address}</div>
            <div class="formula-bar-fx">"fx"</div>
            <input
                node_ref=input_ref
                type="text"
                class=input_class
                prop:value=display_text
                on:focus=on_focus
                on:input=on_input
                on:keydown=on_keydown
            />
        </div>
    }
}
