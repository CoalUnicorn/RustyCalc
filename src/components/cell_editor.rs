use leptos::html;
use leptos::prelude::*;

use crate::canvas::selected_cell_rect;
use crate::state::ModelStore;
use crate::state::WorkbookState;

/// In-cell `<textarea>` overlay positioned over the active cell while editing.
///
/// Mounts only when `WorkbookState.editing_cell` is `Some`.
/// Auto-focused on mount so subsequent keystrokes go here, not to the canvas.
#[component]
pub fn CellEditor() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let textarea_ref = NodeRef::<html::Textarea>::new();

    // Derive a memo that tracks only the EditFocus variant (not the text content).
    // This prevents the Effect below from re-running on every keystroke — text
    // updates mutate `editing_cell.text` but leave the focus variant unchanged,
    // so `focus_state` stays stable while the user types.
    let focus_state = Memo::new(move |_| state.editing_cell.get().map(|e| e.focus));

    // Auto-focus the textarea only when the edit session *starts* with Cell focus
    // (click or printable key), NOT when the formula bar triggered the edit —
    // in that case the formula bar input already holds focus and must keep it.
    // Tracking `focus_state` (not `editing_cell`) ensures this Effect fires only
    // when the focus variant transitions, not on every character typed.
    Effect::new(move |_| {
        let Some(focus) = focus_state.get() else {
            return;
        };
        if focus != crate::state::EditFocus::Cell {
            return;
        }
        let Some(ta) = textarea_ref.get() else { return };
        ta.focus().ok();
        // Move cursor to end of pre-filled text.
        let len = ta.value().len() as u32;
        ta.set_selection_range(len, len).ok();
    });

    // Only the pixel position is dynamic — static styles live in style.css (.cell-editor).
    let cell_style = move || {
        let _ = state.redraw.get();
        let r = model.with_value(|m| selected_cell_rect(m));
        format!(
            "left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px;",
            r.x, r.y, r.width, r.height,
        )
    };

    // Mirror formula bar: the live text buffer.
    let text_value = move || state.editing_cell.get().map(|e| e.text).unwrap_or_default();

    // Keep editing_cell.text in sync as the user types.
    let on_input = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let value = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
            .map(|el| el.value())
            .unwrap_or_default();
        state.editing_cell.update(|cell| {
            if let Some(c) = cell {
                c.text = value;
            }
        });
    };

    // Intercept Enter / Tab / Escape to stop default textarea behavior
    // (newline insertion, browser focus cycling) and let them bubble up
    // to the Workbook container which commits or cancels the edit.
    // Suppress browser defaults; let the event bubble to Workbook
    // which commits or cancels via classify_key -> execute.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if matches!(ev.key().as_str(), "Enter" | "Tab" | "Escape") {
            ev.prevent_default();
        }
    };

    view! {
        <Show when=move || state.editing_cell.get().is_some()>
            <textarea
                node_ref=textarea_ref
                class="cell-editor"
                style=cell_style
                prop:value=text_value
                on:input=on_input
                on:keydown=on_keydown
            />
        </Show>
    }
}
