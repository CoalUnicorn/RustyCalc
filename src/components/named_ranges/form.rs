//! Edit form for a single named range.
//!
//! Reads / writes [`crate::state::WorkbookState::editing_named_range`]. The
//! form is hidden behind a `<Show>` that flips when a row is selected (or
//! `+ New` is clicked) — the empty state shows only the `+ New` button.
//!
//! Save / Delete dispatch through `create_defined_name` /
//! `create_defined_name` / `rename_defined_name` / `remove_defined_name`,
//! each wrapped in `try_mutate(EvaluationMode::Immediate, …)` so dependent
//! cells recompute before the next paint. Errors from ironcalc surface
//! directly in the status bar — they're already user-readable strings
//! ("name already exists", "invalid formula", etc.).
//!
//! Formula storage convention: during edit we keep the user-typed text
//! verbatim (with the leading `=` they see). The `=` is stripped exactly
//! once at the save boundary before being handed to ironcalc, which expects
//! the bare body.

use leptos::prelude::*;

use crate::coord::{CellAddress, TextRef};
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::input::formula_analysis::{FormulaAnalysis, analyze_formula};
use crate::model::frontend_model::DefinedNameManager;
use crate::model::{EvaluationMode, SheetQuery, try_mutate};
use crate::state::{EditingDefinedName, ModelStore, StatusMessage, WorkbookState};

use super::formula_input::FormulaInput;

#[component]
pub fn NamedRangeForm() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Sheet list for the scope <select>. Re-runs only on content changes,
    // not on every selection move.
    let sheets = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| m.get_sheet_names())
    });

    let on_new = move |_| {
        let context_cell = model.with_value(CellAddress::from_view);
        state.editing_named_range.set(Some(EditingDefinedName {
            original: None,
            name: String::new(),
            scope: None,
            formula: String::from("="),
            cursor: 1,
            formula_analysis: FormulaAnalysis::default(),
            context_cell,
        }));
    };

    let on_cancel = move |_| state.editing_named_range.set(None);

    let on_name_change = move |ev: web_sys::Event| {
        let v = event_target_value(&ev);
        state.editing_named_range.update(|opt| {
            if let Some(c) = opt {
                c.name = v;
            }
        });
    };

    // "workbook" sentinel ↔ None scope; numeric ↔ Some(sheet_index).
    //
    // Maintains the invariant `context_cell.sheet == scope.unwrap_or(view_sheet)`
    // so unqualified refs in the formula body resolve against the scope sheet
    // for sheet-scoped names, and against the user's current view sheet for
    // workbook-scoped names. Re-runs analysis on the existing formula text
    // because the same body classifies differently under a new scope (a bare
    // `A1` flips between Valid-on-Sheet2 and bare-ref-error-on-Workbook).
    let on_scope_change = move |ev: web_sys::Event| {
        let raw = event_target_value(&ev);
        let scope = if raw == "workbook" {
            None
        } else {
            raw.parse::<u32>().ok()
        };
        let view_sheet = model.with_value(CellAddress::from_view).sheet;
        let (sheet_names, defined_names) =
            model.with_value(|m| (m.get_sheet_names(), m.get_defined_names()));
        state.editing_named_range.update(|opt| {
            if let Some(c) = opt {
                c.scope = scope;
                c.context_cell.sheet = scope.unwrap_or(view_sheet);
                c.formula_analysis =
                    analyze_formula(&c.formula, c.context_cell, &sheet_names, &defined_names);
            }
        });
    };

    let on_save = move |_| {
        let Some(edit) = state.editing_named_range.get_untracked() else {
            return;
        };
        if edit.save_blockers() {
            return;
        }
        // Strip the leading `=` once, at the save boundary.
        let formula_body = edit.formula.strip_prefix('=').unwrap_or(&edit.formula);

        // ironcalc validates defined-name formulas with `parse_reference_formula(None, ...)`
        // — no context sheet — so bare refs (`B1`) are rejected even for sheet-scoped
        // names. Under Workbook scope the user must qualify explicitly (save is gated
        // above). Under sheet scope we qualify on their behalf at the save boundary,
        // never touching the textarea text. The list re-displays whatever ironcalc
        // round-trips back, which is the canonical qualified form.
        let qualified;
        let body_for_save: &str = match edit.scope {
            Some(idx) if edit.formula_analysis.has_bare_refs() => {
                let sheet_name = model.with_value(|m| m.get_sheet_name(idx as usize));
                qualified = qualify_bare_refs(
                    formula_body,
                    &sheet_name,
                    &edit.formula_analysis.bare_ref_spans,
                );
                &qualified
            }
            _ => formula_body,
        };

        let result = match &edit.original {
            None => try_mutate(model, EvaluationMode::Immediate, |m| {
                m.create_defined_name(edit.name.trim(), edit.scope, body_for_save)
            }),
            Some((old_name, old_scope)) => try_mutate(model, EvaluationMode::Immediate, |m| {
                m.rename_defined_name(
                    old_name,
                    *old_scope,
                    edit.name.trim(),
                    edit.scope,
                    body_for_save,
                )
            }),
        };

        match result {
            Ok(()) => {
                state.editing_named_range.set(None);
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
            }
            Err(msg) => state.status.set(Some(StatusMessage::Error(msg))),
        }
    };

    let on_delete = move |_| {
        let Some(edit) = state.editing_named_range.get_untracked() else {
            return;
        };
        let Some((name, scope)) = edit.original else {
            return;
        };
        let result = try_mutate(model, EvaluationMode::Immediate, |m| {
            m.remove_defined_name(&name, scope)
        });
        match result {
            Ok(()) => {
                state.editing_named_range.set(None);
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
            }
            Err(msg) => state.status.set(Some(StatusMessage::Error(msg))),
        }
    };

    let save_disabled = move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.save_blockers())
            .unwrap_or(true)
    };

    let is_existing_row = move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.original.is_some())
            .unwrap_or(false)
    };

    let name_value = move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.name)
            .unwrap_or_default()
    };

    let scope_value = move || match state.editing_named_range.get().and_then(|e| e.scope) {
        None => "workbook".to_string(),
        Some(idx) => idx.to_string(),
    };

    view! {
        <div class="nrm-form">
            <Show
                when=move || state.editing_named_range.get().is_some()
                fallback=move || view! {
                    <p class="nrm-hint">
                        "Select a row to edit, or create a new defined name."
                    </p>
                    <div class="nrm-btns">
                        <button class="nrm-btn-primary" on:click=on_new>
                            "+ New"
                        </button>
                    </div>
                }
            >
                <div class="nrm-form-row">
                    <label>"Name"</label>
                    <input
                        type="text"
                        prop:value=name_value
                        on:input=on_name_change
                    />
                </div>
                <div class="nrm-form-row">
                    <label>"Scope"</label>
                    <select on:change=on_scope_change prop:value=scope_value>
                        <option value="workbook">"Workbook"</option>
                        <For
                            each=move || sheets.get()
                            key=|(idx, _)| *idx
                            children=move |(idx, name)| view! {
                                <option value=idx.to_string()>{name}</option>
                            }
                        />
                    </select>
                </div>
                <div class="nrm-form-row">
                    <label>"Refers to"</label>
                    <FormulaInput />
                </div>
                <div class="nrm-btns">
                    <button
                        class="nrm-btn-danger"
                        on:click=on_delete
                        disabled=move || !is_existing_row()
                    >
                        "Delete"
                    </button>
                    <button on:click=on_cancel>"Cancel"</button>
                    <button
                        class="nrm-btn-primary"
                        on:click=on_save
                        disabled=save_disabled
                    >
                        "Save"
                    </button>
                </div>
            </Show>
        </div>
    }
}

/// Prefix every bare-ref token in `body` with `<sheet_name>!`.
///
/// `bare_spans` are byte offsets into the **`=`-prefixed** formula string the
/// analyzer was fed (because `analyze_formula` runs on the textarea's full
/// content). `body` is the same string with the leading `=` already stripped,
/// so each insertion lands at `span.start - 1`.
///
/// Spans are walked in reverse so earlier offsets remain valid as later ones
/// are mutated. Sheet name is quoted via ironcalc's own `quote_name` so the
/// output round-trips through the same lexer that will validate it next.
fn qualify_bare_refs(body: &str, sheet_name: &str, bare_spans: &[TextRef]) -> String {
    let qualifier = format!(
        "{}!",
        ironcalc_base::expressions::utils::quote_name(sheet_name)
    );
    let mut result = body.to_string();
    for span in bare_spans.iter().rev() {
        let pos = span.start.saturating_sub(1).min(result.len());
        result.insert_str(pos, &qualifier);
    }
    result
}
