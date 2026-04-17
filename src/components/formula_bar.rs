// See docs/leptos-patterns.md for component conventions.

use leptos::prelude::*;

use crate::canvas::col_name;
use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::input::edit_sync::{read_value_and_cursor, suppress_navigation_defaults, sync_edit};
use crate::input::formula_analysis::analyze_formula;
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
/// The text buffer is shared with `CellEditor` via `state.editing_cell` - both
/// components read/write the same `RwSignal`, so they stay in sync.
#[component]
pub fn FormulaBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let input_ref = state.formula_input_ref;

    let cell_address = move || {
        // Subscribe to navigation events (selection changes affect cell address display)
        let _ = state.events.navigation.get();
        model.with_value(|m| {
            let ac = m.active_cell();
            format!("{}{}", col_name(ac.column), ac.row)
        })
    };

    // While editing: live edit buffer (shared with CellEditor).
    // Otherwise: raw cell content (formula text or literal).
    let display_text = move || {
        if let Some(edit) = state.editing_cell.get() {
            return edit.text;
        }
        // Subscribe to content + navigation events (content changes and selection changes affect display)
        let _ = state.events.content.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| m.active_cell_content())
    };

    let is_editing = move || state.editing_cell.get().is_some();

    // Helper: collect (sheet_index, sheet_name) pairs for analyze_formula().
    // Called at the start of each on_input to get the current sheet list.
    let get_sheet_names =
        move || -> Vec<(u32, String)> { model.with_value(|m| m.get_sheet_names()) };

    let sheet_names = get_sheet_names();

    let (_validation_error, _set_validation_error) = signal(None::<String>);

    // Start an edit session with FormulaBar focus (so CellEditor doesn't
    // steal focus back), or switch focus if already editing.
    let on_focus = move |_: web_sys::FocusEvent| {
        if state.editing_cell.get_untracked().is_some() {
            state.editing_cell.update(|cell| {
                if let Some(c) = cell {
                    c.focus = EditFocus::FormulaBar;
                }
            });
            return;
        }
        model.with_value(|m| {
            let text = m.active_cell_content();
            let address = m.active_cell();

            // Fire editing started event
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::EditingStarted { address },
            ));

            state.editing_cell.set(Some(EditingCell {
                address,
                text: text.clone(),
                mode: EditMode::Edit,
                focus: EditFocus::FormulaBar,
                text_dirty: false,
                formula_analysis: analyze_formula(&text, address.sheet, &sheet_names), //FormulaAnalysis::default(),
                cursor: text.len(),
            }));
        });
    };

    // Update the shared edit buffer (syncs with CellEditor) + debounced validation.
    let on_input = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some((value, cursor)) = read_value_and_cursor(&target) else {
            return;
        };
        let sheet_names = model.with_value(|m| m.get_sheet_names());
        sync_edit(state.editing_cell, value, cursor, sheet_names);
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| suppress_navigation_defaults(&ev);

    let input_class = move || {
        let base = if is_editing() {
            "fb-input editing"
        } else {
            "fb-input"
        };
        let validation = state.editing_cell.get().map_or("", |edit| {
            if !edit.text.starts_with('=') {
                return "";
            }
            if edit.formula_analysis.validation_error.is_some() {
                " error"
            } else {
                " valid"
            }
        });
        format!("{base}{validation}")
    };

    view! {
        <div id="formula-bar" class="fb">
            <div class="fb-addr">
                {cell_address}
            </div>
            <div class="fb-fx">"fx"</div>
            <input
                node_ref=input_ref
                type="text"
                class=input_class
                prop:value=display_text
                on:focus=on_focus
                on:input=on_input
                on:keydown=on_keydown
            />

            // Validation status indicator — Stage 3 will populate this.
            <div class="fb-valid"></div>

        </div>
    }
}
