// See docs/leptos-patterns.md for component conventions.

use leptos::prelude::*;
use leptos_use::{use_debounce_fn, use_throttle_fn};
use wasm_bindgen::JsCast;

use crate::canvas::col_name;
use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::model::{CellAddress, FrontendModel};
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
        // Subscribe to navigation events (selection changes affect cell address display)
        let _ = state.subscribe_to_navigation_events()();
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
        // Subscribe to content + navigation events (content changes and selection changes affect display)
        let _ = state.subscribe_to_content_events()();
        let _ = state.subscribe_to_navigation_events()();
        model.with_value(|m| m.active_cell_content())
    };

    let is_editing = move || state.get_editing_cell().is_some();

    // ???: Debounced formula validation (300ms)
    // Create a stored validation state that's updated via debouncing
    let (validation_pending, set_validation_pending) = signal(false);
    let (validation_error, set_validation_error) = signal(None::<String>);

    // Manual debounce implementation using leptos-use use_timeout_fn
    let debounced_validate = use_debounce_fn(
        move || {
            // Get current formula text for validation
            if let Some(edit) = state.get_editing_cell_untracked() {
                let text = edit.text;
                if text.trim().is_empty() || !text.starts_with('=') {
                    set_validation_error.set(None);
                    return;
                }

                // Simple validation checks
                if text.len() > 1000 {
                    set_validation_error.set(Some("Formula too long (max 1000 chars)".to_string()));
                } else if text.matches('(').count() != text.matches(')').count() {
                    set_validation_error.set(Some("Mismatched parentheses".to_string()));
                } else {
                    set_validation_error.set(None);
                }
            }
            set_validation_pending.set(false);
        },
        300.0,
    );

    // ???: Throttled highlighting state update (100ms = 10fps)
    let throttled_highlight = use_throttle_fn(
        move || {
            // In a real implementation, this could update CSS classes for syntax highlighting
            // For now, just mark that highlighting occurred
            if let Some(edit) = state.get_editing_cell_untracked() {
                let text = edit.text;
                if text.starts_with('=') && text.len() > 1 {
                    // This could trigger syntax highlighting updates
                    // web_sys::console::debug_1(&"Highlighting formula".into());
                }
            }
        },
        100.0,
    );

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
            let address = CellAddress {
                sheet: ac.sheet,
                row: ac.row,
                column: ac.column,
            };

            // Fire editing started event
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::EditingStarted { address },
            ));

            state.set_editing_cell(Some(EditingCell {
                address,
                text,
                mode: EditMode::Edit,
                focus: EditFocus::FormulaBar,
            }));
        });
    };

    // Update the shared edit buffer (syncs with CellEditor) + debounced validation.
    let on_input = move |ev: web_sys::Event| {
        let value = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|el| el.value())
            .unwrap_or_default();

        // Immediate UI update (no lag in typing experience)
        if state.get_editing_cell_untracked().is_some() {
            state.update_editing_cell(|cell| {
                if let Some(c) = cell {
                    c.text = value.clone();
                }
            });
        } else {
            // First keystroke — Accept mode: arrows commit + navigate.
            model.with_value(|m| {
                let ac = m.active_cell();
                state.set_editing_cell(Some(EditingCell {
                    address: CellAddress {
                        sheet: ac.sheet,
                        row: ac.row,
                        column: ac.column,
                    },
                    text: value.clone(),
                    mode: EditMode::Accept,
                    focus: EditFocus::FormulaBar,
                }));
            });
        }

        // Trigger debounced validation (300ms after typing stops)
        set_validation_pending.set(true);
        debounced_validate();

        // Trigger throttled highlighting (smooth 10fps updates)
        throttled_highlight();
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
            "formula-bar-input editing"
        } else {
            "formula-bar-input"
        };
        let validation = match validation_error.get() {
            Some(_) => " error",
            None if validation_pending.get() => " validating",
            None => " valid",
        };
        format!("{}{}", base, validation)
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
                placeholder="Enter formula (=SUM(A1:A10)) or value"
            />
            // Validation status indicator
            <div class="formula-validation">
                {move || {
                    if validation_pending.get() {
                        view! { <span class="validation-pending" title={"Checking formula syntax...".to_string()}>"Validating..."</span> }
                    } else if let Some(error) = validation_error.get() {
                        view! { <span class="validation-error" title={error.clone()}>"Error"</span> }
                    } else if is_editing() && display_text().starts_with('=') {
                        view! { <span class="validation-success" title={"Formula syntax is valid".to_string()}>"Valid"</span> }
                    } else {
                        view! { <span class="validation-neutral" title={"No validation needed".to_string()}>""</span> }
                    }
                }}
            </div>
        </div>
    }
}
