use ironcalc_base::expressions::types::Area;
use leptos::prelude::*;

use crate::action::{classify_key, execute, SpreadsheetAction};
use crate::canvas::AppClipboard;
use crate::util::warn_if_err;
use crate::components::worksheet::Worksheet;
use crate::state::{EditMode, ModelStore, WorkbookState};

/// Top-level editor container.
///
/// Handles all keyboard events when no overlay input is focused, then
/// delegates rendering to FormulaBar, Worksheet (canvas), and SheetTabBar.
///
/// Key classification and action execution are delegated to [`crate::action`].
/// Only clipboard operations (which need async OS APIs and the `AppClipboard`
/// store) and point-mode arrow handling (which needs DOM cursor position) are
/// handled inline here.
#[component]
pub fn Workbook() -> impl IntoView {
    #[allow(clippy::expect_used)]
    let state = use_context::<WorkbookState>().expect("WorkbookState must be in context");
    #[allow(clippy::expect_used)]
    let model = use_context::<ModelStore>().expect("StoredValue<UserModel> must be in context");
    #[allow(clippy::expect_used)]
    let clipboard_store = use_context::<StoredValue<Option<AppClipboard>, LocalStorage>>()
        .expect("StoredValue<Option<AppClipboard>> must be in context");

    // Pre-clone for the Show closures in view! (on_keydown moves `state` below).
    let _state_upload = state.clone();
    let _state_share = state.clone();
    let _state_regional = state.clone();
    let _state_named = state.clone();
    let _state_chart = state.clone();
    let _state_fn_browser = state.clone();

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        // Don't intercept keyboard events while the function browser modal is open;
        // it handles Escape / Enter / Arrow keys itself via window_event_listener.
        if state.show_function_browser.get_untracked() {
            return;
        }

        // Don't intercept keystrokes from panel form elements (Named Ranges, etc.).
        // Exception: the cell-editor <textarea> must bubble Enter/Escape/Tab/Arrow
        // up to this handler, so we only block textarea when not in editing mode.
        if let Some(target) = ev.target() {
            use wasm_bindgen::JsCast;
            if let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                let tag = el.tag_name().to_ascii_lowercase();
                let is_editing = state.editing_cell.get_untracked().is_some();
                if tag == "input" || tag == "select" || (tag == "textarea" && !is_editing) {
                    return;
                }
            }
        }

        let key = ev.key();
        let is_ctrl = ev.ctrl_key() || ev.meta_key();
        let is_shift = ev.shift_key();
        let is_alt = ev.alt_key();

        // ── Point-mode pre-check ──────────────────────────────────────────
        // Arrow keys in Accept mode may enter/extend a cell-reference range
        // inside a formula, rather than committing the edit.  This requires
        // reading the textarea cursor position from the DOM, so it must run
        // here before classify_key (which is pure).
        if let Some(ref edit) = state.editing_cell.get_untracked() {
            if edit.mode == EditMode::Accept
                && !is_ctrl
                && !is_alt
                && matches!(
                    key.as_str(),
                    "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight"
                )
            {
                let cursor = crate::formula_input::get_formula_cursor();
                let already_pointing = state.point_range.get_untracked().is_some();
                if already_pointing
                    || crate::formula_input::is_in_reference_mode(&edit.text, cursor)
                {
                    // Move or extend the point-mode range by one cell.
                    let [r1, c1, r2, c2] =
                        state.point_range.get_untracked().unwrap_or_else(|| {
                            model.with_value(|m| {
                                let v = m.get_selected_view();
                                [v.row, v.column, v.row, v.column]
                            })
                        });
                    let (new_r2, new_c2) = match key.as_str() {
                        "ArrowDown" => (r2 + 1, c2),
                        "ArrowUp" => ((r2 - 1).max(1), c2),
                        "ArrowLeft" => (r2, (c2 - 1).max(1)),
                        "ArrowRight" => (r2, c2 + 1),
                        _ => (r2, c2),
                    };
                    // Shift extends the range; plain arrow moves the whole range.
                    let (new_r1, new_c1) =
                        if is_shift { (r1, c1) } else { (new_r2, new_c2) };
                    let sheet = model.with_value(|m| m.get_selected_view().sheet);
                    let ref_str = crate::formula_input::range_ref_str(
                        new_r1, new_c1, new_r2, new_c2, sheet, sheet, "",
                    );
                    let prev_span = state.point_ref_span.get_untracked();
                    let splice_at = prev_span.map(|(_, end)| end).unwrap_or(cursor);
                    let text = edit.text.clone();
                    let (new_text, new_start, new_end) =
                        crate::formula_input::splice_ref(
                            &text, splice_at, &ref_str, prev_span,
                        );
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
                    ev.prevent_default();
                    return;
                }
            }
        }

        // ── Classify key → action ─────────────────────────────────────────
        let edit_ref = state.editing_cell.get_untracked();
        let Some(action) =
            classify_key(&key, is_ctrl, is_shift, is_alt, edit_ref.as_ref())
        else {
            return;
        };

        // ── Dispatch ──────────────────────────────────────────────────────
        match &action {
            // Clipboard: needs AppClipboard store + async OS clipboard APIs.
            SpreadsheetAction::Copy => {
                copy_to_app_clipboard(model, clipboard_store);
                ev.prevent_default();
            }
            SpreadsheetAction::Cut => {
                copy_to_app_clipboard(model, clipboard_store);
                // Clear the selected range.
                model.update_value(|m| {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    for row in r1..=r2 {
                        for col in c1..=c2 {
                            warn_if_err(
                                m.set_user_input(v.sheet, row, col, ""),
                                "set_user_input (cut)",
                            );
                        }
                    }
                    m.evaluate();
                });
                state.request_redraw();
                ev.prevent_default();
            }
            SpreadsheetAction::Paste => {
                if paste_from_clipboard(model, &state, clipboard_store) {
                    ev.prevent_default();
                }
            }

            // Everything else is handled by the centralised execute().
            _ => {
                execute(&action, model, &state);
                ev.prevent_default();
            }
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

// ── Clipboard helpers ─────────────────────────────────────────────────────────

/// Copy the selected range to the internal `AppClipboard` and write
/// tab-separated text to the OS clipboard (fire-and-forget async).
fn copy_to_app_clipboard(
    model: ModelStore,
    clipboard_store: StoredValue<Option<AppClipboard>, LocalStorage>,
) {
    model.with_value(|m| {
        if let Ok(cb) = m.copy_to_clipboard() {
            let app_cb = AppClipboard::capture(&cb);
            let csv = app_cb.csv.clone();
            clipboard_store.update_value(|c| *c = Some(app_cb));
            // Fire-and-forget: write tab-separated text to the OS clipboard.
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(window) = web_sys::window() {
                    let clip = window.navigator().clipboard();
                    let _ =
                        wasm_bindgen_futures::JsFuture::from(clip.write_text(&csv))
                            .await;
                }
            });
        }
    });
}

/// Attempt to paste from the internal clipboard (synchronous).
///
/// If no internal clipboard data is available, falls back to reading the
/// OS clipboard asynchronously.  Returns `true` if the internal paste
/// succeeded (caller should call `ev.prevent_default()`).
fn paste_from_clipboard(
    model: ModelStore,
    state: &WorkbookState,
    clipboard_store: StoredValue<Option<AppClipboard>, LocalStorage>,
) -> bool {
    // Internal paste (synchronous) — from within-app Ctrl+C.
    let internal_pasted = {
        let mut pasted = false;
        clipboard_store.with_value(|opt| {
            if let Some(acb) = opt {
                model.update_value(|m| {
                    if let Err(e) = acb.paste(m, false) {
                        web_sys::console::warn_1(
                            &format!("[ironcalc] paste failed: {e}").into(),
                        );
                    }
                    m.evaluate();
                });
                pasted = true;
            }
        });
        pasted
    };

    // Clear the dashed "marching ants" border around the copied range.
    if internal_pasted {
        clipboard_store.update_value(|c| *c = None);
    }

    // OS clipboard paste (async, fire-and-forget) — from Excel / Google Sheets.
    // Only attempted when no internal clipboard data was available; otherwise
    // the async path would race and overwrite the already-completed paste.
    if !internal_pasted {
        let model2 = model;
        let state2 = state.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Some(window) = web_sys::window() else {
                return;
            };
            let clip = window.navigator().clipboard();
            let Ok(js_text) =
                wasm_bindgen_futures::JsFuture::from(clip.read_text()).await
            else {
                return;
            };
            let text = js_text.as_string().unwrap_or_default();
            if text.is_empty() {
                return;
            }
            model2.update_value(|m| {
                let v = m.get_selected_view();
                let area = Area {
                    sheet: v.sheet,
                    row: v.row,
                    column: v.column,
                    width: 1,
                    height: 1,
                };
                if let Err(e) = m.paste_csv_string(&area, &text) {
                    web_sys::console::warn_1(
                        &format!("[ironcalc] paste_csv_string failed: {e}").into(),
                    );
                }
                m.evaluate();
            });
            state2.request_redraw();
        });
    }

    if internal_pasted {
        state.request_redraw();
    }

    internal_pasted
}
