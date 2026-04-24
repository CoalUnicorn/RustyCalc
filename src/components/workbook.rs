use leptos::prelude::*;

use crate::components::{
    file_bar::FileBar, formula_bar::FormulaBar, header_context_menu::HeaderContextMenuOverlay,
    sheet_tab_bar::SheetTabBar, status_bar::StatusBar, toolbar::Toolbar, worksheet::Worksheet,
};
use crate::coord::{Cell, SheetRange};
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::input::error::EditError;
use crate::input::{
    edit::EditAction,
    formula_input::*,
    keyboard::{classify_key, execute, KeyMod, SpreadsheetAction},
};
use crate::model::{mutate, try_mutate, AppClipboard, EvaluationMode, PasteMode};
use crate::state::{DragState, EditMode, ModelStore, StatusMessage, WorkbookState};

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

        if let Some(ref edit) = state.editing_cell.get_untracked() {
            let already_pointing = matches!(state.drag.get_untracked(), DragState::Pointing { .. });
            let may_point = edit.mode == EditMode::Accept || edit.text_dirty || already_pointing;

            if may_point && !is_ctrl && !is_alt {
                let caret_hit = if !already_pointing {
                    edit.formula_analysis
                        .refs_at_cursor(edit.cursor)
                        .next()
                        .cloned()
                } else {
                    None
                };

                let (current_ref, prev_span) = match caret_hit {
                    Some(hit) => (hit.ref_node, Some(hit.span)),
                    None => (
                        state.effective_point_ref(model),
                        if let DragState::Pointing { ref_text, .. } = state.drag.get_untracked() {
                            Some(ref_text)
                        } else {
                            None
                        },
                    ),
                };

                let editing = model.with_value(Cell::from_view);
                let ctx = PointMoveCtx {
                    text: &edit.text,
                    cursor: edit.cursor,
                    already_pointing,
                    current_ref,
                    prev_span,
                    editing,
                };
                // web_sys::console::log_1(
                //     &format!(
                //         "key: {} ,already_pointing: {}, may_point: {}, edit.cursor: {}, edit.text: {}",
                //         &key, already_pointing, may_point, edit.cursor, edit.text,
                //     )
                //     .into(),
                // );
                match try_point_move(&ctx, &key, is_shift) {
                    PointMoveOutcome::NoAction => {}
                    PointMoveOutcome::ExitPointing => {
                        if already_pointing {
                            state.drag.set(DragState::Idle);
                        }
                    }
                    PointMoveOutcome::Move(result) => {
                        state.editing_cell.update(|c| {
                            if let Some(e) = c {
                                e.text = result.text;
                                e.text_dirty = false;
                            }
                        });
                        state.drag.set(DragState::Pointing {
                            ref_node: result.ref_node,
                            ref_text: result.span,
                        });
                        ev.prevent_default();
                        return;
                    }
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
                if let Err(e) = try_mutate(
                    model,
                    EvaluationMode::Immediate,
                    |m| -> Result<(), EditError> {
                        let sheet_area = SheetRange::from_view(m);
                        sheet_area.area.cells().try_for_each(|(row, col)| {
                            m.set_user_input(sheet_area.sheet, row, col, "")
                                .map_err(EditError::Engine)
                        })
                    },
                ) {
                    state.status.set(Some(StatusMessage::Error(e.to_string())));
                }
                let sheet_area = model.with_value(SheetRange::from_view);
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                    sheet_area,
                }));
                ev.prevent_default();
            }
            SpreadsheetAction::Paste => {
                if paste_from_clipboard(model, state, clipboard_store) {
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
            <HeaderContextMenuOverlay />
            <SheetTabBar />
            <StatusBar />
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
                let clip = leptos::prelude::window().navigator().clipboard();
                let _ = wasm_bindgen_futures::JsFuture::from(clip.write_text(&csv)).await;
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
            let clip = leptos::prelude::window().navigator().clipboard();
            let Ok(js_text) = wasm_bindgen_futures::JsFuture::from(clip.read_text()).await else {
                return;
            };
            let text = js_text.as_string().unwrap_or_default();
            if text.is_empty() {
                return;
            }
            mutate(model, EvaluationMode::Immediate, |m| {
                let area = SheetRange::from_view(m).to_ironcalc_area();
                if let Err(e) = m.paste_csv_string(&area, &text) {
                    web_sys::console::warn_1(
                        &format!("[ironcalc] paste_csv_string failed: {e}").into(),
                    );
                }
            });
            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
        });
    }

    if internal_pasted {
        state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
    }

    internal_pasted
}
