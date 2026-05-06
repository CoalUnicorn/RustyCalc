//! Read-only table of every defined name in the workbook.
//!
//! Rows render `(name, scope, formula)` straight off
//! `FrontendModel::get_defined_names()`. The list re-runs whenever
//! `state.events.content` fires — that's the channel the CRUD wrappers
//! emit on, so a Save / Delete / New refreshes the rows automatically.
//!
//! Clicking a row populates [`crate::state::WorkbookState::editing_named_range`]
//! with `original = Some((name, scope))`. The form below switches into
//! "edit existing" mode (Save calls `rename_defined_name`, Delete is enabled).

use leptos::prelude::*;

use crate::coord::{CellAddress, DefinedName};
use crate::input::formula_analysis::analyze_formula;
use crate::model::FrontendModel;
use crate::state::{EditingDefinedName, ModelStore, WorkbookState};

#[component]
pub fn NamedRangesList() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let names = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| m.get_defined_names())
    });

    // Identity of the currently-edited row, for `.active` highlighting.
    // Using `original` (not `name`/`scope`) keeps the highlight stable while
    // the user is renaming — the row visually anchored to the *source* row
    // rather than jumping around as they type.
    let active_key = move || state.editing_named_range.get().and_then(|e| e.original);

    view! {
        <div class="nrm-list-wrap">
            <table class="nrm-list">
                <thead>
                    <tr>
                        <th>"Name"</th>
                        <th>"Scope"</th>
                        <th>"Refers to"</th>
                    </tr>
                </thead>
                <tbody>
                    <Show
                        when=move || !names.get().is_empty()
                        fallback=|| view! {
                            <tr><td colspan="3" class="nrm-empty">
                                "No named ranges yet. Use the form below to create one."
                            </td></tr>
                        }
                    >
                        <For
                            each=move || names.get()
                            key=|d: &DefinedName| (d.name.clone(), d.scope)
                            children=move |d| {
                                let key = (d.name.clone(), d.scope);
                                let row_class = {
                                    let key = key.clone();
                                    move || if active_key() == Some(key.clone()) {
                                        "nrm-row active"
                                    } else {
                                        "nrm-row"
                                    }
                                };
                                let on_click = {
                                    let d = d.clone();
                                    move |_| select_row(state, model, &d)
                                };
                                view! {
                                    <tr class=row_class on:click=on_click>
                                        <td class="nrm-name">{d.name.clone()}</td>
                                        <td class="nrm-scope">{scope_label(d.scope, model)}</td>
                                        <td class="nrm-formula">
                                            <code>{format!("={}", d.formula)}</code>
                                        </td>
                                    </tr>
                                }
                            }
                        />
                    </Show>
                </tbody>
            </table>
        </div>
    }
}

/// Populate `editing_named_range` from a row in the list.
///
/// Snapshots the active cell at click time as `context_cell` — relative refs
/// inside the formula are interpreted from there, matching Excel's "active
/// cell at dialog open" convention. Re-runs `analyze_formula` so the form's
/// error class lights up immediately if the stored formula is broken.
fn select_row(state: WorkbookState, model: ModelStore, d: &DefinedName) {
    // Invariant: `context_cell.sheet == scope.unwrap_or(view_sheet_at_open)`.
    // For a sheet-scoped row, point resolution at the scope sheet so unqualified
    // refs in `d.formula` resolve there (matching ironcalc's runtime resolution
    // and Excel semantics for sheet-scoped names).
    let mut context_cell = model.with_value(CellAddress::from_view);
    if let Some(scope_idx) = d.scope {
        context_cell.sheet = scope_idx;
    }
    let formula_text = format!("={}", d.formula);
    let cursor = formula_text.len();
    let (sheet_names, defined_names) =
        model.with_value(|m| (m.get_sheet_names(), m.get_defined_names()));
    let analysis = analyze_formula(&formula_text, context_cell, &sheet_names, &defined_names);
    state.editing_named_range.set(Some(EditingDefinedName {
        original: Some((d.name.clone(), d.scope)),
        name: d.name.clone(),
        scope: d.scope,
        formula: formula_text,
        cursor,
        formula_analysis: analysis,
        context_cell,
    }));
}

/// Render a scope `Option<u32>` as a human-readable label.
/// `None` -> "Workbook"; `Some(idx)` -> that sheet's name (or fallback).
fn scope_label(scope: Option<u32>, model: ModelStore) -> String {
    match scope {
        None => "Workbook".to_string(),
        Some(idx) => model.with_value(|m| m.get_sheet_name(idx as usize)),
    }
}
