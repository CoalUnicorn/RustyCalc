// See docs/leptos-patterns.md for component conventions.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::formula_overlay::FormulaOverlay;
use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::input::edit_sync::{read_value_and_cursor, suppress_navigation_defaults, sync_edit};
use crate::input::formula_analysis::{FormulaStatus, analyze_formula};
use crate::model::SheetQuery;
use crate::model::frontend_model::DefinedNameManager;
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use iron_canvas_core::col_name;

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
    // Cache the overlay element so on_scroll doesn't query_selector at 60 Hz.
    let overlay_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    let cell_address = move || {
        // While editing, pin to the editing cell's address. The live cursor
        // moves during point-mode reference selection, but the label must show
        // where the edit will be committed.
        if let Some(edit) = state.editing_cell.get() {
            return format!("{}{}", col_name(edit.address.column), edit.address.row);
        }
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
            let sheet_names = model.with_value(|m| m.get_sheet_names());
            let defined_names = model.with_value(|m| m.get_defined_names());

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
                formula_analysis: analyze_formula(&text, address, &sheet_names, &defined_names),
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

    // Overlay text mirrors `display_text` exactly so colored spans align
    // with the input characters regardless of edit state.
    let overlay_text = Signal::derive(move || {
        if let Some(edit) = state.editing_cell.get() {
            return edit.text;
        }
        let _ = state.events.content.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| m.active_cell_content())
    });
    // Colored spans only exist while editing — the non-editing display is
    // the cell's raw content, which isn't run through the formula analyzer.
    let overlay_refs = Signal::derive(move || {
        state
            .editing_cell
            .get()
            .map(|e| e.formula_analysis.refs().to_vec())
            .unwrap_or_default()
    });

    let on_scroll = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some(inp) = target.dyn_ref::<web_sys::HtmlInputElement>() else {
            return;
        };
        if let Some(overlay) = overlay_ref.get() {
            overlay.set_scroll_left(inp.scroll_left());
        }
    };

    // Ref-under-caret tooltip — first visible consumer of the ref_node
    // identity preserved by analyze_formula. While editing, if the caret
    // sits on (inclusive right edge) a resolved ref, render its localized
    // form — `$A$1` stays `$A$1`, `Sheet2!B2` keeps its qualifier — proving
    // absolute flags and sheet_name round-trip through the pipeline.
    //
    // The three primitives this closure composes are:
    //   - FormulaAnalysis::refs_at_cursor(cursor) -> Iterator<&FormulaRef>
    //   - RefNode::to_localized(&CellReferenceRC) -> String
    //   - CellAddress::as_stringify_ctx() -> CellReferenceRC
    let ref_under_caret = move || -> String {
        state
            .editing_cell
            .get()
            .and_then(|edit| {
                let ctx = edit.address.as_stringify_ctx();
                edit.formula_analysis
                    .refs_at_cursor(edit.cursor)
                    .next()
                    .map(|r| r.ref_node.to_localized(&ctx))
            })
            .unwrap_or_default()
    };

    let input_class = move || {
        let base = if is_editing() {
            "fb-input editing"
        } else {
            "fb-input"
        };
        let validation =
            state
                .editing_cell
                .get()
                .map_or("", |edit| match edit.formula_analysis.status {
                    FormulaStatus::NotFormula => "",
                    FormulaStatus::Valid { .. } => " valid",
                    FormulaStatus::ParseError(_)
                    | FormulaStatus::LexerError(_)
                    | FormulaStatus::Unresolved { .. } => " error",
                });
        format!("{base}{validation}")
    };

    view! {
        <div id="formula-bar" class="fb">
            <div class="fb-addr">
                {cell_address}
            </div>
            <div class="fb-fx">"fx"</div>
            <div class="fe-host fb-input-host">
                <FormulaOverlay node_ref=overlay_ref text=overlay_text refs=overlay_refs />
                <input
                    node_ref=input_ref
                    type="text"
                    class=input_class
                    prop:value=display_text
                    on:focus=on_focus
                    on:input=on_input
                    on:keydown=on_keydown
                    on:scroll=on_scroll
                />
            </div>

            // Ref-under-caret indicator. Populated by `ref_under_caret` when
            // editing and the cursor sits on a resolved ref.
            <div class="fb-valid">{ref_under_caret}</div>

        </div>
    }
}
