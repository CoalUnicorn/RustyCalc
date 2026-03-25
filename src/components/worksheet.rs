use ironcalc_base::expressions::types::Area;

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use crate::canvas::{
    autofill_handle_pos, frozen_geometry, pixel_to_col, pixel_to_row, CanvasRenderer,
    ClipboardRange, RenderOverlays, SheetRect, AUTOFILL_HANDLE_PX, DEFAULT_COL_WIDTH,
    DEFAULT_ROW_HEIGHT, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};
use crate::components::cell_editor::CellEditor;
use crate::model::{AppClipboard, ArrowKey, FrontendModel, PageDir};
use crate::state::ModelStore;
use crate::state::{
    ContextMenuState, ContextMenuTarget, DragState, EditFocus, EditMode, EditingCell, WorkbookState,
};
use crate::util::warn_if_err;

/// The spreadsheet canvas element.
///
/// Subscribes to `WorkbookState.redraw` so the canvas repaints whenever
/// any model mutation calls `WorkbookState::request_redraw()`.
/// Handles the full mouse interaction set: click-to-select, drag-to-select,
/// autofill handle drag, double-click-to-edit, and wheel scrolling.
#[component]
pub fn Worksheet() -> impl IntoView {
    let canvas_ref = NodeRef::<html::Canvas>::new();
    #[allow(clippy::expect_used)]
    let state = use_context::<WorkbookState>().expect("WorkbookState must be in context");
    #[allow(clippy::expect_used)]
    let model = use_context::<ModelStore>().expect("StoredValue<UserModel> must be in context");

    // ── ResizeObserver: re-render when the container changes size ────────────
    // Leptos signals don't fire on DOM resize, so we wire a ResizeObserver
    // that bumps the redraw counter whenever the worksheet div is resized
    // (e.g. browser window resize, devtools open/close).
    let state_ro = state.clone();
    let container_ref = NodeRef::<html::Div>::new();
    // Guard: only create the ResizeObserver on the first effect run.
    // Without this, any reactive re-run (e.g. hot-reload) would create a second
    // observer while the first remains connected — firing double redraws forever.
    let observer_created = StoredValue::new_local(false);
    Effect::new(move |_| {
        let Some(div) = container_ref.get() else {
            return;
        };
        if observer_created.get_value() {
            return;
        }
        observer_created.set_value(true);
        let div_el: web_sys::Element = div.into();
        let state_ro2 = state_ro.clone();
        let cb = Closure::<dyn Fn(web_sys::js_sys::Array, web_sys::ResizeObserver)>::new(
            move |_entries, _observer| {
                state_ro2.request_redraw();
            },
        );
        #[allow(clippy::expect_used)]
        let observer = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref())
            .expect("ResizeObserver::new must not fail");
        observer.observe(&div_el);
        cb.forget();
        std::mem::forget(observer);
    });

    // Re-render canvas every time the redraw counter increments.
    let state_draw = state.clone();
    #[allow(clippy::expect_used)]
    let clipboard_draw = use_context::<StoredValue<Option<AppClipboard>, LocalStorage>>()
        .expect("StoredValue<Option<AppClipboard>> must be in context");
    Effect::new(move |_| {
        let _ = state_draw.redraw.get();
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas;
        let extend_to = match state_draw.drag.get_untracked() {
            DragState::Extending { to_row, to_col } => Some((to_row, to_col)),
            _ => None,
        };
        let point_range = state_draw.point_range.get_untracked();
        let canvas_theme = state_draw.theme.get_untracked().canvas_theme();
        let clipboard = clipboard_draw.with_value(|opt| {
            opt.as_ref().map(|acb| {
                let (r1, c1, r2, c2) = acb.range;
                ClipboardRange {
                    sheet: acb.sheet,
                    r1,
                    c1,
                    r2,
                    c2,
                }
            })
        });
        let overlays = RenderOverlays {
            extend_to,
            clipboard,
            point_range: point_range.map(|[r1, c1, r2, c2]| SheetRect { r1, c1, r2, c2 }),
        };
        // Sync canvas dimensions into the model so that on_area_selecting
        // knows how wide/tall the visible area is and only scrolls when the
        // drag target is genuinely outside the viewport (not on every move).
        let canvas_w = canvas_el.client_width() as f64;
        let canvas_h = canvas_el.client_height() as f64;
        model.update_value(|m| {
            m.set_window_width(canvas_w);
            m.set_window_height(canvas_h);
        });
        model.with_value(|m| {
            let renderer = CanvasRenderer::new(&canvas_el, canvas_theme);
            renderer.render(m, &overlays);
        });
    });

    // ── mousedown: start selection or autofill drag ───────────────────────────
    let state_md = state.clone();
    let on_mousedown = move |ev: web_sys::MouseEvent| {
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        // ── Resize hit-tests (must be first: takes priority over cell click) ──
        // A 4-px zone around each column/row boundary in the header acts as a
        // resize handle; dragging it will change that column/row's size.
        const HIT_ZONE: f64 = 4.0;

        // Column resize: click inside the column header row near a right edge.
        if y < HEADER_ROW_HEIGHT && x > HEADER_COL_WIDTH {
            if let Some(col) =
                model.with_value(|m| crate::canvas::geometry::find_col_boundary_at(m, x, HIT_ZONE))
            {
                state_md.drag.set(DragState::ResizingCol { col, x });
                ev.prevent_default();
                return;
            }
        }

        // Row resize: click inside the row header column near a bottom edge.
        if x < HEADER_COL_WIDTH && y > HEADER_ROW_HEIGHT {
            if let Some(row) =
                model.with_value(|m| crate::canvas::geometry::find_row_boundary_at(m, y, HIT_ZONE))
            {
                state_md.drag.set(DragState::ResizingRow { row, y });
                ev.prevent_default();
                return;
            }
        }

        // Corner cell (top-left of header area) → select the entire sheet.
        if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
            model.update_value(|m| {
                m.nav_select_all();
            });
            state_md.editing_cell.set(None);
            state_md.request_redraw();
            return;
        }

        // Column header click → select the entire column.
        if y < HEADER_ROW_HEIGHT && x >= HEADER_COL_WIDTH {
            model.update_value(|m| {
                let view = m.get_selected_view();
                let sheet = view.sheet;
                let fg = frozen_geometry(m, sheet);
                let col = pixel_to_col(m, sheet, view.left_column, x, &fg);
                if ev.shift_key() {
                    // Extend the current selection to this column.
                    m.nav_extend_selection(1, col);
                } else {
                    m.nav_select_column(col);
                }
            });
            state_md.editing_cell.set(None);
            state_md.request_redraw();
            return;
        }

        // Row header click → select the entire row.
        if x < HEADER_COL_WIDTH && y >= HEADER_ROW_HEIGHT {
            model.update_value(|m| {
                let view = m.get_selected_view();
                let sheet = view.sheet;
                let fg = frozen_geometry(m, sheet);
                let row = pixel_to_row(m, sheet, view.top_row, y, &fg);
                if ev.shift_key() {
                    // Extend the current selection to this row.
                    m.nav_extend_selection(row, 1);
                } else {
                    m.nav_select_row(row);
                }
            });
            state_md.editing_cell.set(None);
            state_md.request_redraw();
            return;
        }

        if x < HEADER_COL_WIDTH || y < HEADER_ROW_HEIGHT {
            return;
        }

        // Read model state (row, col, autofill hit-test) without holding a
        // mutable borrow, so signal writes below don't interleave with the lock.
        let (row, col, near_handle) = model.with_value(|m| {
            let view = m.get_selected_view();
            let sheet = view.sheet;
            let fg = frozen_geometry(m, sheet);
            let col = pixel_to_col(m, sheet, view.left_column, x, &fg);
            let row = pixel_to_row(m, sheet, view.top_row, y, &fg);
            let handle = autofill_handle_pos(m);
            let near_handle = (x - handle.x).abs() <= AUTOFILL_HANDLE_PX
                && (y - handle.y).abs() <= AUTOFILL_HANDLE_PX;
            (row, col, near_handle)
        });

        // ── Point mode: intercept click during formula entry ─────────────────
        // When the cursor is at a syntactically valid reference position inside
        // a formula, clicking a cell inserts/replaces the reference rather than
        // committing the edit and navigating away.
        if let Some(ref edit) = state_md.editing_cell.get_untracked() {
            if edit.mode == crate::state::EditMode::Accept {
                let cursor = get_formula_cursor();
                let already_pointing = state_md.point_range.get_untracked().is_some();
                if already_pointing
                    || crate::formula_input::is_in_reference_mode(&edit.text, cursor)
                {
                    let sheet = model.with_value(|m| m.active_cell().sheet);
                    let ref_str =
                        crate::formula_input::range_ref_str(row, col, row, col, sheet, sheet, "");
                    let prev_span = state_md.point_ref_span.get_untracked();
                    let text = edit.text.clone();
                    let (new_text, new_start, new_end) =
                        crate::formula_input::splice_ref(&text, cursor, &ref_str, prev_span);
                    state_md.editing_cell.update(|c| {
                        if let Some(e) = c {
                            e.text = new_text;
                        }
                    });
                    state_md.point_range.set(Some([row, col, row, col]));
                    state_md.drag.set(DragState::Pointing);
                    state_md.point_ref_span.set(Some((new_start, new_end)));
                    state_md.request_redraw();
                    return;
                }
            }
        }

        // Apply model mutations and signal writes after the read closure.
        if near_handle {
            // Begin autofill drag — don't change the selection.
            state_md.drag.set(DragState::Extending {
                to_row: row,
                to_col: col,
            });
        } else if ev.shift_key() {
            // Shift-click extends the range from the current anchor.
            model.update_value(|m| {
                m.nav_extend_selection(row, col);
            });
            state_md.drag.set(DragState::Selecting);
        } else {
            model.update_value(|m| {
                m.nav_set_cell(row, col);
            });
            state_md.drag.set(DragState::Selecting);
        }

        state_md.editing_cell.set(None);
        state_md.request_redraw();
    };

    // ── mousemove: expand selection or autofill preview ──────────────────────
    let state_mm = state.clone();
    let on_mousemove = move |ev: web_sys::MouseEvent| {
        // If no button is held the drag ended outside the canvas (mouseup was
        // missed). Reset all drag state so the next interaction starts clean.
        if ev.buttons() == 0 {
            state_mm.drag.set(DragState::Idle);
            return;
        }
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        // ── Resize drags ──────────────────────────────────────────────────────
        match state_mm.drag.get_untracked() {
            DragState::ResizingCol { col, x: last_x } => {
                let delta = x - last_x;
                model.update_value(|m| {
                    let sheet = m.active_cell().sheet;
                    let current_w = m.get_column_width(sheet, col).unwrap_or(DEFAULT_COL_WIDTH);
                    let new_w = (current_w + delta).max(5.0);
                    warn_if_err(
                        m.set_columns_width(sheet, col, col, new_w),
                        "set_columns_width",
                    );
                });
                state_mm.drag.set(DragState::ResizingCol { col, x });
                state_mm.request_redraw();
                ev.prevent_default();
                return;
            }
            DragState::ResizingRow { row, y: last_y } => {
                let delta = y - last_y;
                model.update_value(|m| {
                    let sheet = m.active_cell().sheet;
                    let current_h = m.get_row_height(sheet, row).unwrap_or(DEFAULT_ROW_HEIGHT);
                    let new_h = (current_h + delta).max(3.0);
                    warn_if_err(m.set_rows_height(sheet, row, row, new_h), "set_rows_height");
                });
                state_mm.drag.set(DragState::ResizingRow { row, y });
                state_mm.request_redraw();
                ev.prevent_default();
                return;
            }
            _ => {}
        }

        if x < HEADER_COL_WIDTH || y < HEADER_ROW_HEIGHT {
            return;
        }

        let (row, col) = model.with_value(|m| {
            let view = m.get_selected_view();
            let sheet = view.sheet;
            let fg = frozen_geometry(m, sheet);
            (
                pixel_to_row(m, sheet, view.top_row, y, &fg),
                pixel_to_col(m, sheet, view.left_column, x, &fg),
            )
        });

        match state_mm.drag.get_untracked() {
            DragState::Extending { .. } => {
                // Update autofill preview target.
                state_mm.drag.set(DragState::Extending {
                    to_row: row,
                    to_col: col,
                });
                state_mm.request_redraw();
            }
            DragState::Pointing => {
                // Extend the point-mode range to the hovered cell.
                if let Some([r1, c1, _, _]) = state_mm.point_range.get_untracked() {
                    let sheet = model.with_value(|m| m.active_cell().sheet);
                    let ref_str =
                        crate::formula_input::range_ref_str(r1, c1, row, col, sheet, sheet, "");
                    let prev_span = state_mm.point_ref_span.get_untracked();
                    let cursor = prev_span.map(|(_, end)| end).unwrap_or(0);
                    let new_state = state_mm.editing_cell.get_untracked().map(|edit| {
                        crate::formula_input::splice_ref(&edit.text, cursor, &ref_str, prev_span)
                    });
                    if let Some((new_text, new_start, new_end)) = new_state {
                        state_mm.editing_cell.update(|c| {
                            if let Some(e) = c {
                                e.text = new_text;
                            }
                        });
                        state_mm.point_range.set(Some([r1, c1, row, col]));
                        state_mm.point_ref_span.set(Some((new_start, new_end)));
                        state_mm.request_redraw();
                    }
                }
            }
            DragState::Selecting => {
                // pixel_to_col/row start scanning from left_column/top_row, so
                // they can never return a value smaller than the viewport origin.
                // When the pointer is at the leftmost/topmost visible data cell
                // and the viewport is scrolled, nudge the target one step past
                // the edge so on_area_selecting scrolls the viewport left/up.
                let (eff_row, eff_col) = model.with_value(|m| {
                    let view = m.get_selected_view();
                    let ec = if col == view.left_column && view.left_column > 1 {
                        col - 1
                    } else {
                        col
                    };
                    let er = if row == view.top_row && view.top_row > 1 {
                        row - 1
                    } else {
                        row
                    };
                    (er, ec)
                });
                model.update_value(|m| {
                    m.nav_extend_selection(eff_row, eff_col);
                });
                state_mm.request_redraw();
            }
            _ => {}
        }
    };

    // ── mouseup: commit autofill or end selection drag ────────────────────────
    let state_mu = state.clone();
    let on_mouseup = move |_ev: web_sys::MouseEvent| {
        // Commit autofill if active, then reset drag state unconditionally.
        if let DragState::Extending { to_row, to_col } = state_mu.drag.get_untracked() {
            model.update_value(|m| {
                let view = m.get_selected_view();
                let sheet = view.sheet;
                let [r1, c1, r2, c2] = view.range;
                let (r_min, r_max) = (r1.min(r2), r1.max(r2));
                let (c_min, c_max) = (c1.min(c2), c1.max(c2));
                let area = Area {
                    sheet,
                    row: r_min,
                    column: c_min,
                    height: r_max - r_min + 1,
                    width: c_max - c_min + 1,
                };
                // Fill rows if the drag crossed a row boundary, else fill columns.
                if to_row < r_min || to_row > r_max {
                    warn_if_err(m.auto_fill_rows(&area, to_row), "auto_fill_rows");
                } else {
                    warn_if_err(m.auto_fill_columns(&area, to_col), "auto_fill_columns");
                }
                m.evaluate();
            });
            state_mu.request_redraw();
        }
        state_mu.drag.set(DragState::Idle);
    };

    // ── dblclick: enter edit mode with existing cell content ──────────────────
    let state_dc = state.clone();
    let on_dblclick = move |ev: web_sys::MouseEvent| {
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;
        if x < HEADER_COL_WIDTH || y < HEADER_ROW_HEIGHT {
            return;
        }
        // mousedown already navigated the active cell to the clicked position.
        model.with_value(|m| {
            let ac = m.active_cell();
            let text = m.active_cell_content();
            state_dc.editing_cell.set(Some(EditingCell {
                sheet: ac.sheet,
                row: ac.row,
                col: ac.column,
                text,
                mode: EditMode::Edit,
                focus: EditFocus::Cell,
            }));
        });
    };

    // ── contextmenu: right-click on column/row header ────────────────────────
    let state_cm = state.clone();
    let on_contextmenu = move |ev: web_sys::MouseEvent| {
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        let target = if y < HEADER_ROW_HEIGHT && x >= HEADER_COL_WIDTH {
            // Column header
            Some(ContextMenuTarget::Column(model.with_value(|m| {
                let v = m.get_selected_view();
                let fg = frozen_geometry(m, v.sheet);
                pixel_to_col(m, v.sheet, v.left_column, x, &fg)
            })))
        } else if x < HEADER_COL_WIDTH && y >= HEADER_ROW_HEIGHT {
            // Row header
            Some(ContextMenuTarget::Row(model.with_value(|m| {
                let v = m.get_selected_view();
                let fg = frozen_geometry(m, v.sheet);
                pixel_to_row(m, v.sheet, v.top_row, y, &fg)
            })))
        } else {
            None
        };

        if let Some(target) = target {
            ev.prevent_default();
            state_cm.context_menu.set(Some(ContextMenuState {
                x: ev.client_x(),
                y: ev.client_y(),
                target,
            }));
        }
    };

    // ── wheel: scroll with delta-magnitude awareness ──────────────────────────
    // Trackpads emit many small-delta events; physical wheels emit large ones.
    // Use arrow-style scroll for small deltas so trackpad users aren't thrown a
    // full page per gesture. Also handle delta_x for horizontal trackpad swipes.
    let state_wh = state.clone();
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let dy = ev.delta_y();
        let dx = ev.delta_x();
        model.update_value(|m| {
            if dx.abs() > dy.abs() {
                // Predominantly horizontal — trackpad swipe.
                if dx > 0.0 {
                    m.nav_arrow(ArrowKey::Right);
                } else {
                    m.nav_arrow(ArrowKey::Left);
                }
            } else if dy.abs() < 100.0 {
                // Small vertical delta → single-row scroll (trackpad).
                if dy > 0.0 {
                    m.nav_arrow(ArrowKey::Down);
                } else {
                    m.nav_arrow(ArrowKey::Up);
                }
            } else {
                // Large vertical delta → page scroll (mouse wheel).
                if dy > 0.0 {
                    m.nav_page(PageDir::Down);
                } else {
                    m.nav_page(PageDir::Up);
                }
            }
        });
        state_wh.request_redraw();
    };

    view! {
        <div node_ref=container_ref style="position:relative;flex:1;overflow:hidden;min-height:0;">
            <canvas
                node_ref=canvas_ref
                role="application"
                aria-label="Spreadsheet grid"
                style=move || {
                    match state.drag.get() {
                        DragState::ResizingCol { .. } =>
                            "width:100%;height:100%;display:block;cursor:col-resize;",
                        DragState::ResizingRow { .. } =>
                            "width:100%;height:100%;display:block;cursor:row-resize;",
                        _ =>
                            "width:100%;height:100%;display:block;cursor:cell;",
                    }
                }
                tabindex="-1"
                on:mousedown=on_mousedown
                on:mousemove=on_mousemove
                on:mouseup=on_mouseup
                on:dblclick=on_dblclick
                on:wheel=on_wheel
                on:contextmenu=on_contextmenu
            />
            <CellEditor />
        </div>
    }
}

// ── Module-level helpers ──────────────────────────────────────────────────────

fn get_formula_cursor() -> usize {
    crate::formula_input::get_formula_cursor()
}
