use ironcalc_base::expressions::types::Area;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::canvas::{AppClipboard, ArrowKey, FrontendModel, PageDir};
use crate::components::worksheet::Worksheet;
use crate::state::ModelStore;
use crate::state::{EditFocus, EditMode, EditingCell, WorkbookState};
use crate::storage;

/// Top-level editor container.
///
/// Handles all keyboard events when no overlay input is focused, then
/// delegates rendering to FormulaBar, Worksheet (canvas), and SheetTabBar.
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
    let state_upload = state.clone();
    let state_share = state.clone();
    let state_regional = state.clone();
    let state_named = state.clone();
    let state_chart = state.clone();
    let state_fn_browser = state.clone();

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

        // ── While editing: handle commit / cancel / escape ────────────────
        if let Some(edit) = state.editing_cell.get_untracked() {
            match key.as_str() {
                "Enter" => {
                    commit_edit(model, &state, &edit);
                    model.update_value(|m| m.nav_arrow(ArrowKey::Down));
                    state.request_redraw();
                    refocus_workbook();
                    ev.prevent_default();
                }
                "Tab" => {
                    commit_edit(model, &state, &edit);
                    model.update_value(|m| {
                        m.nav_arrow(if is_shift {
                            ArrowKey::Left
                        } else {
                            ArrowKey::Right
                        })
                    });
                    state.request_redraw();
                    refocus_workbook();
                    ev.prevent_default();
                }
                "Escape" => {
                    state.editing_cell.set(None);
                    state.point_range.set(None);
                    state.point_ref_span.set(None);
                    state.request_redraw();
                    refocus_workbook();
                    ev.prevent_default();
                }
                // Arrow keys in Accept mode: enter point mode when the cursor is
                // at a syntactically valid reference position inside a formula;
                // otherwise commit the edit and navigate.
                // In Edit mode they move the text cursor (handled by textarea).
                "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight"
                    if edit.mode == EditMode::Accept =>
                {
                    let cursor = crate::formula_input::get_formula_cursor();
                    // Stay in point mode once a range is active, even though the
                    // cursor now sits after a reference token (not an operator).
                    let already_pointing = state.point_range.get_untracked().is_some();
                    if already_pointing
                        || crate::formula_input::is_in_reference_mode(&edit.text, cursor)
                    {
                        // Move or extend the point-mode range by one cell.
                        let [r1, c1, r2, c2] =
                            state.point_range.get_untracked().unwrap_or_else(|| {
                                model.with_value(|m| {
                                    let ac = m.active_cell();
                                    [ac.row, ac.column, ac.row, ac.column]
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
                        let (new_r1, new_c1) = if is_shift { (r1, c1) } else { (new_r2, new_c2) };
                        let sheet = model.with_value(|m| m.active_cell().sheet);
                        let ref_str = crate::formula_input::range_ref_str(
                            new_r1, new_c1, new_r2, new_c2, sheet, sheet, "",
                        );
                        let prev_span = state.point_ref_span.get_untracked();
                        let splice_at = prev_span.map(|(_, end)| end).unwrap_or(cursor);
                        let text = edit.text.clone();
                        let (new_text, new_start, new_end) =
                            crate::formula_input::splice_ref(&text, splice_at, &ref_str, prev_span);
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
                    } else {
                        commit_edit(model, &state, &edit);
                        if let Some(dir) = arrow_key_from_str(&key) {
                            model.update_value(|m| m.nav_arrow(dir));
                        }
                        state.request_redraw();
                        refocus_workbook();
                        ev.prevent_default();
                    }
                }
                _ => {} // other keys handled by the focused input element
            }
            return;
        }

        // ── Not editing — full keyboard navigation ────────────────────────

        // Ctrl+... shortcuts
        if is_ctrl && !is_shift && !is_alt {
            let mutated = match key.to_lowercase().as_str() {
                "z" => {
                    model.update_value(|m| {
                        if let Err(e) = m.undo() {
                            web_sys::console::warn_1(
                                &format!("[ironcalc] undo failed: {e}").into(),
                            );
                        }
                    });
                    true
                }
                "y" => {
                    model.update_value(|m| {
                        if let Err(e) = m.redo() {
                            web_sys::console::warn_1(
                                &format!("[ironcalc] redo failed: {e}").into(),
                            );
                        }
                    });
                    true
                }
                "a" => {
                    // Select the used data range (like Excel's Ctrl+A).
                    model.update_value(|m| {
                        let d = m.sheet_dimension();
                        m.nav_select_range(d.min_row, d.min_column, d.max_row, d.max_column);
                    });
                    true
                }
                // TODO: Toolbar
                /// Apply a style property to the full selection range of the current view.
                // pub fn apply_to_selection(m: &mut UserModel, path: &str, value: &str) {
                //     let v = m.get_selected_view();
                //     let [r1, c1, r2, c2] = v.range;
                //     let area = Area {
                //         sheet: v.sheet,
                //         row: r1.min(r2),
                //         column: c1.min(c2),
                //         height: (r1 - r2).abs() + 1,
                //         width: (c1 - c2).abs() + 1,
                //     };
                //     m.update_range_style(&area, path, value).ok();
                // }
                // "b" => {
                //     model.update_value(|m| {
                //         let on = m.toolbar_state().bold;
                //         apply_to_selection(m, "font.b", if on { "false" } else { "true" });
                //     });
                //     true
                // }
                // "i" => {
                //     model.update_value(|m| {
                //         let on = m.toolbar_state().italic;
                //         apply_to_selection(m, "font.i", if on { "false" } else { "true" });
                //     });
                //     true
                // }
                // "u" => {
                //     model.update_value(|m| {
                //         let on = m.toolbar_state().underline;
                //         apply_to_selection(m, "font.u", if on { "false" } else { "true" });
                //     });
                //     true
                // }
                "c" | "x" => {
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
                    // Cut: clear all cells in the selected range after copying.
                    if key == "x" {
                        model.update_value(|m| {
                            let v = m.get_selected_view();
                            let [row_start, col_start, row_end, col_end] = v.range;
                            for row in row_start..=row_end {
                                for col in col_start..=col_end {
                                    if let Err(e) = m.set_user_input(v.sheet, row, col, "") {
                                        web_sys::console::warn_1(
                                            &format!("[ironcalc] set_user_input failed: {e}")
                                                .into(),
                                        );
                                    }
                                }
                            }
                            m.evaluate();
                        });
                        state.request_redraw();
                    }
                    ev.prevent_default();
                    false
                }
                "v" => {
                    // Internal paste (synchronous) — from within-app Ctrl+C
                    let internal_pasted = {
                        let mut pasted = false;
                        clipboard_store.with_value(|opt| {
                            if let Some(acb) = opt {
                                model.update_value(|m| {
                                    if let Err(e) = acb.paste(m, false) {
                                        web_sys::console::warn_1(
                                            &format!("[ironcalc] paste failed: {e}")
                                                .into(),
                                        );
                                    }
                                    m.evaluate();
                                });
                                pasted = true;
                            }
                        });
                        pasted
                    };
                    // OS clipboard paste (async, fire-and-forget) — from Excel / Google Sheets.
                    // Only attempted when no internal clipboard data was available; otherwise
                    // the async path would race and overwrite the already-completed paste.
                    // paste_csv_string expects tab-delimited text, which is exactly what
                    // Excel and Sheets write to the OS clipboard.
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
                                let ac = m.active_cell();
                                let area = Area {
                                    sheet: ac.sheet,
                                    row: ac.row,
                                    column: ac.column,
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
                        ev.prevent_default();
                    }
                    internal_pasted
                }
                _ => false,
            };
            // Ctrl+Home: jump to A1
            if key == "Home" {
                model.update_value(|m| m.nav_set_cell(1, 1));
                state.request_redraw();
                ev.prevent_default();
                return;
            }
            // Ctrl+End: jump to the last used cell (navigate to bottom-right edge)
            if key == "End" {
                model.update_value(|m| {
                    m.nav_to_edge(ArrowKey::Down);
                    m.nav_to_edge(ArrowKey::Right);
                });
                state.request_redraw();
                ev.prevent_default();
                return;
            }
            // Ctrl+Arrow: navigate to edge
            if let Some(dir) = arrow_key_from_str(&key) {
                model.update_value(|m| m.nav_to_edge(dir));
                state.request_redraw();
                ev.prevent_default();
                return;
            }
            if mutated {
                state.request_redraw();
                ev.prevent_default();
            }
            return;
        }

        // Alt+Arrow: switch sheets
        if is_alt && !is_ctrl && !is_shift {
            match key.as_str() {
                "ArrowDown" => {
                    navigate_sheet(model, &state, 1);
                    ev.prevent_default();
                }
                "ArrowUp" => {
                    navigate_sheet(model, &state, -1);
                    ev.prevent_default();
                }
                _ => {}
            }
            return;
        }

        // Shift+Arrow: expand selection
        if is_shift && !is_ctrl && !is_alt {
            let expanded = match key.as_str() {
                "ArrowRight" | "ArrowLeft" | "ArrowUp" | "ArrowDown" => {
                    if let Some(dir) = arrow_key_from_str(&key) {
                        model.update_value(|m| m.nav_expand_selection(dir));
                    }
                    true
                }
                "Tab" => {
                    model.update_value(|m| m.nav_arrow(ArrowKey::Left));
                    true
                }
                _ => false,
            };
            if expanded {
                state.request_redraw();
                ev.prevent_default();
                return;
            }
        }

        // Ctrl+Shift+= : insert row; Ctrl+Shift+Alt+= : insert column
        // Ctrl+Shift+Delete : clear contents AND formatting in the selected range
        #[allow(clippy::collapsible_if)]
        if is_ctrl && is_shift && !is_alt {
            if key == "Delete" {
                model.update_value(|m| {
                    let v = m.get_selected_view();
                    let area = SelectionBounds::from_range(v.range).area(v.sheet);
                    m.range_clear_all(&area).ok();
                    m.evaluate();
                });
                state.request_redraw();
                ev.prevent_default();
                return;
            }
        }
        #[allow(clippy::collapsible_if)]
        if is_ctrl && is_shift {
            if key == "=" || key == "+" {
                if is_alt {
                    // Insert column left of selection
                    model.update_value(|m| {
                        let v = m.get_selected_view();
                        let sb = SelectionBounds::from_range(v.range);
                        m.insert_columns(v.sheet, sb.col_min, sb.col_max - sb.col_min + 1)
                            .ok();
                        m.evaluate();
                    });
                } else {
                    // Insert row above selection
                    model.update_value(|m| {
                        let v = m.get_selected_view();
                        let sb = SelectionBounds::from_range(v.range);
                        m.insert_rows(v.sheet, sb.row_min, sb.row_max - sb.row_min + 1)
                            .ok();
                        m.evaluate();
                    });
                }
                state.request_redraw();
                ev.prevent_default();
                return;
            }
        }

        // Ctrl+- : delete rows; Ctrl+Alt+- : delete columns
        if is_ctrl && !is_shift && key == "-" {
            if is_alt {
                // Delete selected columns
                model.update_value(|m| {
                    let v = m.get_selected_view();
                    let sb = SelectionBounds::from_range(v.range);
                    m.delete_columns(v.sheet, sb.col_min, sb.col_max - sb.col_min + 1)
                        .ok();
                    m.evaluate();
                });
            } else {
                // Delete selected rows
                model.update_value(|m| {
                    let v = m.get_selected_view();
                    let sb = SelectionBounds::from_range(v.range);
                    m.delete_rows(v.sheet, sb.row_min, sb.row_max - sb.row_min + 1)
                        .ok();
                    m.evaluate();
                });
            }
            state.request_redraw();
            ev.prevent_default();
            return;
        }

        if is_ctrl || is_alt {
            return; // unhandled modifier combinations
        }

        // Plain keys
        match key.as_str() {
            "ArrowRight" | "Tab" => {
                model.update_value(|m| m.nav_arrow(ArrowKey::Right));
                state.request_redraw();
                ev.prevent_default();
            }
            "ArrowLeft" => {
                model.update_value(|m| m.nav_arrow(ArrowKey::Left));
                state.request_redraw();
                ev.prevent_default();
            }
            "ArrowDown" | "Enter" => {
                model.update_value(|m| m.nav_arrow(ArrowKey::Down));
                state.request_redraw();
                ev.prevent_default();
            }
            "ArrowUp" => {
                model.update_value(|m| m.nav_arrow(ArrowKey::Up));
                state.request_redraw();
                ev.prevent_default();
            }
            "PageDown" => {
                model.update_value(|m| m.nav_page(PageDir::Down));
                state.request_redraw();
                ev.prevent_default();
            }
            "PageUp" => {
                model.update_value(|m| m.nav_page(PageDir::Up));
                state.request_redraw();
                ev.prevent_default();
            }
            // Home: move to column A of the current row.
            "Home" => {
                model.update_value(|m| m.nav_home_row());
                state.request_redraw();
                ev.prevent_default();
            }
            // End: move to the last used cell in the current row.
            "End" => {
                model.update_value(|m| m.nav_to_edge(ArrowKey::Right));
                state.request_redraw();
                ev.prevent_default();
            }
            "Delete" => {
                // Clear cell contents (preserves formatting).
                model.update_value(|m| {
                    let v = m.get_selected_view();
                    let area = SelectionBounds::from_range(v.range).area(v.sheet);
                    m.range_clear_contents(&area).ok();
                    m.evaluate();
                });
                state.request_redraw();
                ev.prevent_default();
            }
            "Escape" => {
                state.editing_cell.set(None);
                state.request_redraw();
                ev.prevent_default();
            }
            "F2" => {
                // Enter edit mode with existing cell content
                model.with_value(|m| {
                    let ac = m.active_cell();
                    let text = m.active_cell_content();
                    state.editing_cell.set(Some(EditingCell {
                        sheet: ac.sheet,
                        row: ac.row,
                        col: ac.column,
                        text,
                        mode: EditMode::Edit,
                        focus: EditFocus::Cell,
                    }));
                });
                ev.prevent_default();
            }
            k if is_printable(k) => {
                // Start a fresh edit with the pressed character.
                // Accept mode: arrows commit and navigate (like Excel).
                model.with_value(|m| {
                    let ac = m.active_cell();
                    state.editing_cell.set(Some(EditingCell {
                        sheet: ac.sheet,
                        row: ac.row,
                        col: ac.column,
                        text: k.to_owned(),
                        mode: EditMode::Accept,
                        focus: EditFocus::Cell,
                    }));
                });
                ev.prevent_default();
            }
            _ => {}
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

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Write the edit buffer to the model and clear the editing state.
/// Also persists the updated workbook to localStorage.
fn commit_edit(
    model: StoredValue<UserModel<'static>, LocalStorage>,
    state: &WorkbookState,
    edit: &EditingCell,
) {
    model.update_value(|m| {
        m.set_user_input(edit.sheet, edit.row, edit.col, &edit.text)
            .ok();
        m.evaluate();
    });
    state.editing_cell.set(None);
    state.point_range.set(None);
    state.point_ref_span.set(None);

    // Persist after every committed edit.
    let uuid = state.current_uuid.get_untracked();
    if !uuid.is_empty() {
        model.with_value(|m| storage::save(&uuid, m));
    }
}

/// Switch to the next (+1) or previous (-1) visible sheet.
fn navigate_sheet(
    model: StoredValue<UserModel<'static>, LocalStorage>,
    state: &WorkbookState,
    delta: i32,
) {
    model.update_value(|m| {
        let current = m.get_selected_view().sheet;
        let sheets = m.get_worksheets_properties();
        // Find the index of the current sheet in the visible list.
        let visible: Vec<u32> = sheets
            .iter()
            .filter(|s| s.state == "visible")
            .map(|s| s.sheet_id)
            .collect();
        if visible.is_empty() {
            return;
        }
        if let Some(pos) = visible.iter().position(|&id| id == current) {
            let next_pos = (pos as i32 + delta).rem_euclid(visible.len() as i32) as usize;
            m.set_selected_sheet(visible[next_pos]).ok();
        }
    });
    state.request_redraw();
}

/// Normalised row/column bounds of a selection range.
struct SelectionBounds {
    row_min: i32,
    row_max: i32,
    col_min: i32,
    col_max: i32,
}

impl SelectionBounds {
    fn from_range(range: [i32; 4]) -> Self {
        let [r1, c1, r2, c2] = range;
        Self {
            row_min: r1.min(r2),
            row_max: r1.max(r2),
            col_min: c1.min(c2),
            col_max: c1.max(c2),
        }
    }

    /// Build an `Area` covering this selection on `sheet`.
    fn area(&self, sheet: u32) -> Area {
        Area {
            sheet,
            row: self.row_min,
            column: self.col_min,
            height: self.row_max - self.row_min + 1,
            width: self.col_max - self.col_min + 1,
        }
    }
}

/// Map a JS `KeyboardEvent.key` string to an `ArrowKey` variant.
fn arrow_key_from_str(key: &str) -> Option<ArrowKey> {
    match key {
        "ArrowUp" => Some(ArrowKey::Up),
        "ArrowDown" => Some(ArrowKey::Down),
        "ArrowLeft" => Some(ArrowKey::Left),
        "ArrowRight" => Some(ArrowKey::Right),
        _ => None,
    }
}

/// True for single printable characters that should start a cell edit.
fn is_printable(key: &str) -> bool {
    let bytes = key.as_bytes();
    key.chars().count() == 1 && bytes[0] >= 0x20
}

/// Move keyboard focus back to the `#workbook` container after an edit
/// commit or cancel, so subsequent keystrokes reach the keydown handler
/// without the user needing to click again.
fn refocus_workbook() {
    use wasm_bindgen::JsCast;
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("workbook"))
    {
        el.unchecked_into::<web_sys::HtmlElement>().focus().ok();
    }
}
