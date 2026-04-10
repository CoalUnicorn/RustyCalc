use leptos::prelude::*;

use crate::components::{
    file_bar::FileBar, formula_bar::FormulaBar, perf_panel::PerfPanel, sheet_tab_bar::SheetTabBar,
    toolbar::Toolbar, worksheet::Worksheet,
};
use crate::coord::{CellArea, SheetArea};
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::input::{
    action::{classify_key, execute, KeyMod, SpreadsheetAction},
    edit::EditAction,
    formula_input::*,
};
use crate::model::{mutate, AppClipboard, EvaluationMode, PasteMode};
use crate::state::{DragState, EditMode, ModelStore, WorkbookState};
use crate::storage;
use crate::util::warn_if_err;

/// Top-level keyboard router. Clipboard ops and point-mode arrow handling
/// live here (need async OS APIs / DOM cursor position); everything else
/// delegates to `classify_key` + `execute`.
#[component]
pub fn Workbook() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let clipboard_store = expect_context::<StoredValue<Option<AppClipboard>, LocalStorage>>();

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        // Don't intercept keystrokes from panel form elements (Named Ranges, etc.).
        // Exception: the cell-editor <textarea> formula-bar must bubble Enter/Escape/Tab/Arrow
        // up to this handler, so we only block textarea when not in editing mode.
        if let Some(target) = ev.target() {
            use wasm_bindgen::JsCast;
            if let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                let tag = el.tag_name().to_ascii_lowercase();
                let is_editing = state.editing_cell.get_untracked().is_some();
                if tag == "select" || ((tag == "input" || tag == "textarea") && !is_editing) {
                    return;
                }
            }
        }

        let key = ev.key();
        let is_ctrl = ev.ctrl_key() || ev.meta_key();
        let is_shift = ev.shift_key();
        let is_alt = ev.alt_key();

        // Point-mode pre-check
        // Arrow keys in Accept mode may enter/extend a cell-reference range
        // inside a formula, rather than committing the edit.  This requires
        // reading the textarea cursor position from the DOM, so it must run
        // here before classify_key (which is pure).
        if let Some(ref edit) = state.editing_cell.get_untracked() {
            // Exit pointing when user types a non-arrow key (e.g. operator,
            // digit, backspace). This lets the next arrow press start a fresh
            // cell reference via is_in_reference_mode.
            if !matches!(
                key.as_str(),
                "ArrowDown"
                    | "ArrowUp"
                    | "ArrowLeft"
                    | "ArrowRight"
                    | "Shift"
                    | "Control"
                    | "Alt"
                    | "Meta"
            ) && matches!(state.drag.get_untracked(), DragState::Pointing { .. })
            {
                state.drag.set(DragState::Idle);
            }

            if !is_ctrl
                && !is_alt
                && matches!(
                    key.as_str(),
                    "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight"
                )
            {
                let already_pointing =
                    matches!(state.drag.get_untracked(), DragState::Pointing { .. });
                // Accept mode always checks; Edit mode only when text was just
                // modified (typed operator/paren) or already in pointing mode.
                let may_point =
                    edit.mode == EditMode::Accept || edit.text_dirty || already_pointing;

                if may_point {
                    // Clear the dirty flag — this arrow key consumed it.
                    state.editing_cell.update(|c| {
                        if let Some(e) = c {
                            e.text_dirty = false;
                        }
                    });
                }

                let cursor = get_formula_cursor();
                if may_point && (already_pointing || is_in_reference_mode(&edit.text, cursor)) {
                    // Move or extend the point-mode range by one cell.
                    let pr = state.effective_point_range(model);
                    let trailing = pr.extend_trailing(&key);
                    // Shift extends the selection (anchor stays); plain arrow moves the whole range.
                    let new_pr = if is_shift {
                        CellArea {
                            r1: pr.r1,
                            c1: pr.c1,
                            r2: trailing.r2,
                            c2: trailing.c2,
                        }
                    } else {
                        CellArea::from_cell(trailing.r2, trailing.c2)
                    };
                    let sheet = model.with_value(|m| m.get_selected_sheet());
                    let ref_str =
                        range_ref_str(new_pr.r1, new_pr.c1, new_pr.r2, new_pr.c2, sheet, sheet, "");
                    let prev_span = if let DragState::Pointing { ref_span, .. } = state.drag.get() {
                        Some(ref_span)
                    } else {
                        None
                    };
                    let splice_at = prev_span.map(|(_, end)| end).unwrap_or(cursor);
                    let text = edit.text.clone();
                    let (new_text, new_start, new_end) =
                        splice_ref(&text, splice_at, &ref_str, prev_span);
                    state.editing_cell.update(|c| {
                        if let Some(e) = c {
                            e.text = new_text;
                        }
                    });
                    state.drag.set(DragState::Pointing {
                        range: new_pr,
                        ref_span: (new_start, new_end),
                    });
                    ev.prevent_default();
                    return;
                }
            }
        }

        // Classify key -> action
        let edit_ref = state.editing_cell.get_untracked();
        let Some(action) = classify_key(
            &key,
            KeyMod {
                ctrl: is_ctrl,
                shift: is_shift,
                alt: is_alt,
            },
            edit_ref.as_ref(),
        ) else {
            return;
        };

        // Dispatch
        match &action {
            // Clipboard: needs AppClipboard store + async OS clipboard APIs.
            SpreadsheetAction::Copy => {
                copy_to_app_clipboard(model, state, clipboard_store);
                ev.prevent_default();
            }
            SpreadsheetAction::Cut => {
                copy_to_app_clipboard(model, state, clipboard_store);
                // Clear the selected range.
                // Pause evaluation so each set_user_input doesn't trigger a
                // full recalc; evaluate once at the end.
                mutate(model, EvaluationMode::Immediate, |m| {
                    let sheet_area = SheetArea::from_view(m);
                    sheet_area.area.cells().for_each(|(row, col)| {
                        warn_if_err(
                            m.set_user_input(sheet_area.sheet, row, col, ""),
                            "set_user_input (cut)",
                        );
                    });
                });
                let sheet_area = model.with_value(SheetArea::from_view);
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                    sheet_area,
                }));
                ev.prevent_default();
            }
            SpreadsheetAction::Paste => {
                if paste_from_clipboard(model, state, clipboard_store) {
                    if let Some(uuid) = state.current_uuid.get_untracked() {
                        model.with_value(|m| storage::save(&uuid, m));
                    }
                    ev.prevent_default();
                }
            }

            // Escape cancels the marching-ants clipboard selection before
            // delegating the cancel action itself to execute().
            SpreadsheetAction::Edit(EditAction::Cancel) => {
                clipboard_store.update_value(|c| *c = None);
                execute(&action, model, &state);
                ev.prevent_default();
            }

            // Everything else is handled by the centralised execute().
            SpreadsheetAction::Nav(_)
            | SpreadsheetAction::Edit(_)
            | SpreadsheetAction::Format(_)
            | SpreadsheetAction::Structure(_) => {
                execute(&action, model, &state);
                ev.prevent_default();
            }
        }
    };

    view! {
        <div
            id="workbook"
            class="workbook"
            tabindex="0"
            on:keydown=on_keydown
        >
            <FileBar />
            <Toolbar />
            <FormulaBar />
            <Worksheet />
            <Show when=move || state.show_perf_panel.get()>
                <PerfPanel />
            </Show>
            <SheetTabBar />
        </div>
    }
}

fn copy_to_app_clipboard(
    model: ModelStore,
    state: WorkbookState,
    clipboard_store: StoredValue<Option<AppClipboard>, LocalStorage>,
) {
    model.with_value(|m| {
        if let Ok(cb) = m.copy_to_clipboard() {
            let app_cb = AppClipboard::capture(&cb);
            let csv = app_cb.csv.clone();
            clipboard_store.update_value(|c| *c = Some(app_cb));
            // Repaint so the marching-ants border appears immediately.
            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
            // Fire-and-forget: write tab-separated text to the OS clipboard.
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(window) = web_sys::window() {
                    let clip = window.navigator().clipboard();
                    let _ = wasm_bindgen_futures::JsFuture::from(clip.write_text(&csv)).await;
                }
            });
        }
    });
}

/// Returns `true` if internal paste succeeded (caller should `prevent_default`).
/// Falls back to OS clipboard async read when no internal data is available.
fn paste_from_clipboard(
    model: ModelStore,
    state: WorkbookState,
    clipboard_store: StoredValue<Option<AppClipboard>, LocalStorage>,
) -> bool {
    // Internal paste (synchronous) - from within-app Ctrl+C.
    let internal_pasted = {
        let mut pasted = false;
        clipboard_store.with_value(|opt| {
            if let Some(acb) = opt {
                mutate(model, EvaluationMode::Immediate, |m| {
                    if let Err(e) = acb.paste(m, PasteMode::Copy) {
                        web_sys::console::warn_1(&format!("[ironcalc] paste failed: {e}").into());
                    }
                });
                pasted = true;
            }
        });
        pasted
    };

    // OS clipboard paste (async, fire-and-forget) - from Excel / Google Sheets.
    // Only attempted when no internal clipboard data was available; otherwise
    // the async path would race and overwrite the already-completed paste.
    if !internal_pasted {
        wasm_bindgen_futures::spawn_local(async move {
            let Some(window) = web_sys::window() else {
                return;
            };
            let clip = window.navigator().clipboard();
            let Ok(js_text) = wasm_bindgen_futures::JsFuture::from(clip.read_text()).await else {
                return;
            };
            let text = js_text.as_string().unwrap_or_default();
            if text.is_empty() {
                return;
            }
            mutate(model, EvaluationMode::Immediate, |m| {
                let area = SheetArea::from_view(m).to_ironcalc_area();
                if let Err(e) = m.paste_csv_string(&area, &text) {
                    web_sys::console::warn_1(
                        &format!("[ironcalc] paste_csv_string failed: {e}").into(),
                    );
                }
            });
            if let Some(uuid) = state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
        });
    }

    if internal_pasted {
        state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
    }

    internal_pasted
}
