use leptos::prelude::*;

use crate::action::{classify_key, execute};
use crate::canvas::Clipboard;
use crate::components::worksheet::Worksheet;
use crate::formula_input;
use crate::state::ModelStore;
use crate::state::{EditMode, WorkbookState};

/// Top-level editor container.
///
/// Handles all keyboard events and delegates rendering to `Worksheet`.
/// Key dispatch logic lives in `crate::action` — here we only:
///   1. Pre-check for point-mode arrow navigation (requires DOM cursor state).
///   2. Call `classify_key` to map the event to a `SpreadsheetAction`.
///   3. Call `execute` to apply it.
#[component]
pub fn Workbook() -> impl IntoView {
    #[allow(clippy::expect_used)]
    let state = use_context::<WorkbookState>().expect("WorkbookState must be in context");
    #[allow(clippy::expect_used)]
    let model = use_context::<ModelStore>().expect("StoredValue<UserModel> must be in context");

    #[allow(clippy::expect_used)]
    let clipboard_store = use_context::<StoredValue<Option<Clipboard>, LocalStorage>>()
        .expect("StoredValue<Option<Clipboard>> must be in context");

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();
        let ctrl = ev.ctrl_key() || ev.meta_key();
        let shift = ev.shift_key();
        let alt = ev.alt_key();

        let edit = state.editing_cell.get_untracked();

        // ── Point-mode pre-check ────────────────────────────────────────────
        // Arrow keys while in Accept-mode editing may extend a formula
        // reference range rather than committing the edit.  This requires
        // reading the textarea cursor position from the DOM, so it cannot be
        // encoded as a pure `SpreadsheetAction` and must run before classify.
        if let Some(ref e) = edit {
            if e.mode == EditMode::Accept
                && matches!(
                    key.as_str(),
                    "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight"
                )
            {
                let cursor = formula_input::get_formula_cursor();
                let already_pointing = state.point_range.get_untracked().is_some();
                if already_pointing || formula_input::is_in_reference_mode(&e.text, cursor) {
                    handle_point_mode_arrow(&key, shift, model, &state);
                    ev.prevent_default();
                    return;
                }
            }
        }

        // ── Classify → execute ──────────────────────────────────────────────
        if let Some(action) = classify_key(&key, ctrl, shift, alt, edit.as_ref()) {
            execute(&action, model, &state);
            ev.prevent_default();
        }
    };

    view! {
        <div
            id="workbook"
            style="display:flex;flex-direction:column;flex:1;min-width:0;height:100%;outline:none;"
            tabindex="0"
            on:keydown=on_keydown
        >
            <Worksheet />
        </div>
    }
}

// ── Point-mode arrow navigation ───────────────────────────────────────────────
//
// When the user presses an arrow key while typing a formula in Accept mode and
// the cursor is inside a cell-reference token, splice an updated range
// reference into the formula text instead of committing the edit.
//
// Shift extends the range anchor; plain arrows move the whole range.

fn handle_point_mode_arrow(key: &str, shift: bool, model: ModelStore, state: &WorkbookState) {
    let [r1, c1, r2, c2] = state.point_range.get_untracked().unwrap_or_else(|| {
        model.with_value(|m| {
            let v = m.get_selected_view();
            [v.row, v.column, v.row, v.column]
        })
    });

    let (new_r2, new_c2) = match key {
        "ArrowDown" => (r2 + 1, c2),
        "ArrowUp" => ((r2 - 1).max(1), c2),
        "ArrowLeft" => (r2, (c2 - 1).max(1)),
        "ArrowRight" => (r2, c2 + 1),
        _ => (r2, c2),
    };
    // Shift extends from the original anchor; plain arrow moves the whole range.
    let (new_r1, new_c1) = if shift { (r1, c1) } else { (new_r2, new_c2) };

    let sheet = model.with_value(|m| m.get_selected_view().sheet);
    let ref_str = formula_input::range_ref_str(new_r1, new_c1, new_r2, new_c2, sheet, sheet, "");

    let prev_span = state.point_ref_span.get_untracked();
    let cursor = formula_input::get_formula_cursor();
    let splice_at = prev_span.map(|(_, end)| end).unwrap_or(cursor);
    let text = state
        .editing_cell
        .get_untracked()
        .map(|e| e.text)
        .unwrap_or_default();

    let (new_text, new_start, new_end) =
        formula_input::splice_ref(&text, splice_at, &ref_str, prev_span);

    state.editing_cell.update(|c| {
        if let Some(e) = c {
            e.text = new_text;
        }
    });
    state
        .point_range
        .set(Some([new_r1, new_c1, new_r2, new_c2]));
    state.point_ref_span.set(Some((new_start, new_end)));
    state.request_redraw();
}
