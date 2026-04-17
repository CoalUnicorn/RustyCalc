// See docs/leptos-patterns.md for component conventions.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::canvas::col_name;
use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::input::formula_analysis::analyze_formula;
use crate::model::FrontendModel;
use crate::state::{EditFocus, EditMode};
use crate::state::{EditingCell, ModelStore, WorkbookState};

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

    let (validation_error, _set_validation_error) = signal(None::<String>);

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
        let el = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());

        let (value, cursor) = match el {
            Some(el) => {
                let value = el.value();
                let cursor = el
                    .selection_end()
                    .ok()
                    .flatten()
                    .map(|n| n as usize)
                    // fallback: cursor at end of text
                    .unwrap_or_else(|| value.len());
                (value, cursor)
            }
            None => return,
        };

        let sheet_names = get_sheet_names();

        // Immediate UI update (no lag in typing experience)
        if state.editing_cell.get_untracked().is_some() {
            state.editing_cell.update(|cell| {
                if let Some(c) = cell {
                    let active_sheet = c.address.sheet;
                    c.text = value.clone();
                    c.text_dirty = true;
                    c.formula_analysis = analyze_formula(&value, active_sheet, &sheet_names);
                    c.cursor = cursor;
                }
            });
        } else {
            // First keystroke - Accept mode: arrows commit + navigate.
            model.with_value(|m| {
                let address = m.active_cell();
                let analysis = analyze_formula(&value, address.sheet, &sheet_names);
                state.editing_cell.set(Some(EditingCell {
                    address,
                    text: value.clone(),
                    mode: EditMode::Accept,
                    focus: EditFocus::FormulaBar,
                    text_dirty: true,
                    formula_analysis: analysis,
                    cursor,
                }));
                state.emit_event(SpreadsheetEvent::Navigation(
                    NavigationEvent::EditingStarted { address },
                ));
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
                placeholder="Enter value or formula"
            />
            /*
            <div class="fb-valid">
                {move || {
                    let Some(edit) = state.editing_cell.get() else {
                        return view! { <span class="fb-neutral"> title={"No validation needed".to_string()}>""</span> }
                    };
                    if !edit.text.starts_with('=') {
                        return view! { <span class="fb-neutral"></span> };
                    }
                    match edit.formula_analysis.validation_error {
                        Some(ref err) => {
                            let msg = err.clone();
                            view! { <span class="fb-error" title={msg}>"Error"</span> }
                        }
                        None => view! { <span class="fb-success" title="Formula syntax is valid">"Valid"</span> },
                    }
                }}
            </div>
            */
            // Validation status indicator
            <div class="fb-valid">
                {move || {
                    if state.editing_cell.get().is_some() {
                        view! { <span class="fb-pending" title={"Checking formula syntax...".to_string()}>"Validating..."</span> }
                    } else if let Some(error) = validation_error.get() {
                        view! { <span class="fb-error" title={error.clone()}>"Error"</span> }
                    } else if is_editing() && display_text().starts_with('=') {
                        view! { <span class="fb-success" title={"Formula syntax is valid".to_string()}>"Valid"</span> }
                    } else {
                        view! { <span class="fb-neutral" title={"No validation needed".to_string()}>""</span> }
                    }
                }}
            </div>

        </div>
    }
}
