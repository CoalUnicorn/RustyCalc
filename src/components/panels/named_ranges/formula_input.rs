//! Formula field for the Manage Named Ranges dialog.
//!
//! A thin adapter over the shared
//! [`crate::components::ui::formula_field::FormulaField`]. The dialog keeps its
//! own source of truth — [`crate::state::EditingDefinedName`], updated through
//! [`crate::input::formula::sync_edit`] — and merely projects it into the
//! component's read signals (`value`, `refs`, `is_error`) plus a write callback.
//! Switching to the shared component is what gives this dialog its colored
//! ref-token overlay; validation behaviour is unchanged (the error class still
//! reads `EditingDefinedName::formula_invalid`, the same predicate Save uses
//! minus the name-empty check).

use leptos::prelude::*;

use crate::components::ui::formula_field::FormulaField;
use crate::input::formula::sync_edit;
use crate::model::SheetQuery;
use crate::model::frontend_model::DefinedNameManager;
use crate::state::{ModelStore, WorkbookState};

#[component]
pub fn FormulaInput() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Sheet + defined-name lists fed to `analyze_formula` (inside `sync_edit`).
    // Memoised against `events.content` so a keystroke doesn't re-walk the
    // workbook.
    let analyzer_inputs = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| (m.get_sheet_names(), m.get_defined_names()))
    });

    let value = Signal::derive(move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.formula)
            .unwrap_or_default()
    });
    let refs = Signal::derive(move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.formula_analysis.refs().to_vec())
            .unwrap_or_default()
    });
    let is_error = Signal::derive(move || {
        state
            .editing_named_range
            .get()
            .map(|e| e.formula_invalid())
            .unwrap_or(false)
    });

    // Every keystroke flows back through `sync_edit`, which re-runs analysis and
    // updates text + cursor + analysis on `editing_named_range` in lockstep.
    let on_input = Callback::new(move |(value, cursor): (String, usize)| {
        analyzer_inputs.with_untracked(|(sheet_names, defined_names)| {
            sync_edit(
                state.editing_named_range,
                value,
                cursor,
                sheet_names,
                defined_names,
            );
        });
    });

    view! {
        <FormulaField value=value refs=refs is_error=is_error on_input=on_input rows=2 />
    }
}
