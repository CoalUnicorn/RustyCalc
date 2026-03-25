// ──────────────────────────────────────────────────────────────────────────────
// LEPTOS COMPONENT ANATOMY — FormulaBar
//
// A Leptos component is a plain Rust function annotated with #[component].
// It runs ONCE at mount time to set up reactive subscriptions, then Leptos
// re-runs only the individual closures whose signals changed — NOT the whole
// function (unlike React, which re-runs the entire component on every render).
//
// Key patterns used here:
//   1. expect_context::<T>()  — pull shared state from ancestor components
//   2. move || { ... }        — reactive closure: Leptos tracks which signals
//                               it reads and re-runs it when they change
//   3. Memo::new(move |_| ..) — cached derived value, only recomputes when
//                               its input signals change (like useMemo)
//   4. on:event=closure       — DOM event handler (like onClick)
//   5. prop:value=signal      — one-way bind a DOM property to a reactive value
//   6. node_ref=ref           — capture a reference to the DOM element
//   7. view! { <tag /> }      — JSX-like HTML template with reactive bindings
// ──────────────────────────────────────────────────────────────────────────────

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
    // ── 1. Pull shared state from context ────────────────────────────────────
    //
    // These were `provide_context()`-ed by App. Every component in the tree
    // can access them. Because WorkbookState is Copy (all fields are arena
    // handles), there's no cloning overhead.
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // This NodeRef is stored in WorkbookState so other components
    // (like FunctionBrowserModal) can read/write the input's cursor position.
    let input_ref = state.formula_input_ref;

    // ── 2. Derived reactive values ───────────────────────────────────────────
    //
    // These closures are NOT called eagerly — they're passed to the view!
    // macro and Leptos calls them whenever their subscribed signals change.
    //
    // `state.redraw.get()` subscribes to the redraw counter. Any model
    // mutation calls `state.request_redraw()` which increments it, causing
    // all closures that read it to re-run. This is how the address label
    // and display value stay current after navigation/edits.

    // Cell address: "A1", "B7", etc. — updates when selection moves.
    let cell_address = move || {
        // Reading redraw.get() subscribes this closure to selection changes.
        let _ = state.redraw.get();
        model.with_value(|m| {
            let ac = m.active_cell();
            format!("{}{}", col_name(ac.column), ac.row)
        })
    };

    // The text shown in the input:
    // - While editing: the live edit buffer (shared with CellEditor)
    // - While not editing: the raw cell content (formula text or literal)
    let display_text = move || {
        // First check if we're in an edit session.
        if let Some(edit) = state.editing_cell.get() {
            return edit.text;
        }
        // Not editing — show the stored cell content.
        let _ = state.redraw.get();
        model.with_value(|m| m.active_cell_content())
    };

    // Is the formula bar input currently the active editor?
    // Used to style the input differently during editing.
    let is_editing = move || state.editing_cell.get().is_some();

    // ── 3. Event handlers ────────────────────────────────────────────────────
    //
    // Each handler is a `move` closure that captures `state` and `model` by
    // copy (they're Copy types — just arena indices). The closures are passed
    // to `on:event` in the view.

    // Focus/click on the input: start an edit session (if not already editing)
    // with FormulaBar focus, so CellEditor doesn't steal focus back.
    let on_focus = move |_: web_sys::FocusEvent| {
        if state.editing_cell.get_untracked().is_some() {
            // Already editing — just switch focus to formula bar.
            state.editing_cell.update(|cell| {
                if let Some(c) = cell {
                    c.focus = EditFocus::FormulaBar;
                }
            });
            return;
        }
        // Start a new edit session with the cell's current content.
        model.with_value(|m| {
            let ac = m.active_cell();
            let text = m.active_cell_content();
            state.editing_cell.set(Some(EditingCell {
                sheet: ac.sheet,
                row: ac.row,
                col: ac.column,
                text,
                mode: EditMode::Edit,
                focus: EditFocus::FormulaBar,
            }));
        });
    };

    // Typing in the input: update the shared edit buffer.
    // CellEditor also reads `editing_cell.text`, so it updates live.
    let on_input = move |ev: web_sys::Event| {
        let value = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|el| el.value())
            .unwrap_or_default();

        if state.editing_cell.get_untracked().is_some() {
            // Update existing edit session.
            state.editing_cell.update(|cell| {
                if let Some(c) = cell {
                    c.text = value;
                }
            });
        } else {
            // First keystroke — start a new edit session in Accept mode.
            // Accept mode means arrow keys will commit + navigate (like Excel).
            model.with_value(|m| {
                let ac = m.active_cell();
                state.editing_cell.set(Some(EditingCell {
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

    // Keyboard shortcuts specific to the formula bar input.
    // Enter/Tab/Escape commit or cancel — same as CellEditor.
    // We prevent_default to stop the browser's native behavior, then
    // let the event bubble up to Workbook's keydown handler which
    // calls execute(CommitAndNavigate/CancelEdit).
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        match ev.key().as_str() {
            "Enter" | "Tab" | "Escape" => {
                ev.prevent_default();
                // Bubble up to Workbook — it handles commit/cancel/navigate.
            }
            _ => {}
        }
    };

    // ── 4. The view ──────────────────────────────────────────────────────────
    //
    // view! { } is a macro that produces real DOM nodes (not a virtual DOM).
    // Reactive closures (move || ...) are registered as fine-grained
    // subscriptions — only the specific text node or attribute updates,
    // not the whole component tree.
    //
    // Key bindings:
    //   {cell_address}     — text content, re-evaluated when redraw fires
    //   prop:value=...     — sets the DOM .value property reactively
    //   on:input=handler   — DOM event listener
    //   node_ref=input_ref — stores the HtmlInputElement in the NodeRef
    //   class:editing=...  — toggles CSS class based on a reactive bool

    view! {
        <div id="formula-bar" style="
            display: flex;
            align-items: center;
            height: 38px;
            border-bottom: 2px solid var(--border);
            background: var(--surface);
            font-family: Inter, Arial, sans-serif;
            font-size: 15px;
            flex-shrink: 0;
        ">
            // ── Cell address (read-only) ─────────────────────────────────
            <div style="
                min-width: 60px;
                padding: 0 8px;
                font-weight: 600;
                color: var(--text-strong);
                border-right: 1px solid var(--border);
                text-align: center;
                user-select: none;
            ">
                {cell_address}
            </div>

            // ── "fx" label ───────────────────────────────────────────────
            <div style="
                padding: 0 6px;
                color: var(--text-muted);
                font-style: italic;
                user-select: none;
            ">
                "fx"
            </div>

            // ── Formula/content input ────────────────────────────────────
            <input
                node_ref=input_ref
                type="text"
                prop:value=display_text
                on:focus=on_focus
                on:input=on_input
                on:keydown=on_keydown
                style=move || {
                    let base = "flex:1;\
                        border:none;\
                        outline:none;\
                        padding:0 8px;\
                        font-family:Inter,Arial,sans-serif;\
                        font-size:13px;\
                        background:transparent;\
                        color:var(--text-strong);\
                        height:100%;";
                    if is_editing() {
                        // Subtle highlight when actively editing
                        format!("{base}background:var(--cell-editor-bg);")
                    } else {
                        base.to_owned()
                    }
                }
            />
        </div>
    }
}
