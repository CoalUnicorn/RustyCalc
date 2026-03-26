use ironcalc_base::expressions::types::Area;

use leptos::html;
use leptos::prelude::*;
use leptos_use::use_resize_observer;
use web_sys::HtmlCanvasElement;

use crate::canvas::*;
use crate::components::cell_editor::CellEditor;
use crate::input::formula_input::*;
use crate::model::{AppClipboard, ArrowKey, FrontendModel, PageDir};

use crate::state::{
    ContextMenuState, ContextMenuTarget, DragState, EditFocus, EditMode, EditingCell, ModelStore,
    WorkbookState,
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
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // ── ResizeObserver: re-render when the container changes size ────────────
    // Leptos signals don't fire on DOM resize, so we use a ResizeObserver
    // that bumps the redraw counter whenever the worksheet div is resized
    // (e.g. browser window resize, devtools open/close).
    // Cleanup is automatic when the component unmounts.
    let container_ref = NodeRef::<html::Div>::new();
    let _ = use_resize_observer(container_ref, move |_, _| {
        state.request_redraw();
    });

    // Re-render canvas every time the redraw counter increments.
    let clipboard_draw = expect_context::<StoredValue<Option<AppClipboard>, LocalStorage>>();
    Effect::new(move |_| {
        let _ = state.redraw.get();
        let Some(canvas) = canvas_ref.get() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas;
        let extend_to = if let DragState::Extending { to_row, to_col } = state.drag.get_untracked()
        {
            Some((to_row, to_col))
        } else {
            None
        };
        let point_range = state.point_range.get_untracked();
        let canvas_theme = state.theme.get_untracked().canvas_theme();
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
                state.drag.set(DragState::ResizingCol { col, x });
                ev.prevent_default();
                return;
            }
        }

        // Row resize: click inside the row header column near a bottom edge.
        if x < HEADER_COL_WIDTH && y > HEADER_ROW_HEIGHT {
            if let Some(row) =
                model.with_value(|m| crate::canvas::geometry::find_row_boundary_at(m, y, HIT_ZONE))
            {
                state.drag.set(DragState::ResizingRow { row, y });
                ev.prevent_default();
                return;
            }
        }

        // Corner cell (top-left of header area) -> select the entire sheet.
        if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
            model.update_value(|m| {
                m.nav_select_all();
            });
            state.editing_cell.set(None);
            state.request_redraw();
            return;
        }

        // Column header click -> select the entire column.
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
            state.editing_cell.set(None);
            state.request_redraw();
            return;
        }

        // Row header click -> select the entire row.
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
            state.editing_cell.set(None);
            state.request_redraw();
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
        if let Some(ref edit) = state.editing_cell.get_untracked() {
            if edit.mode == crate::state::EditMode::Accept {
                let cursor = get_formula_cursor();
                let already_pointing = state.point_range.get_untracked().is_some();
                if already_pointing || is_in_reference_mode(&edit.text, cursor) {
                    let sheet = model.with_value(|m| m.active_cell().sheet);
                    let ref_str = range_ref_str(row, col, row, col, sheet, sheet, "");
                    let prev_span = state.point_ref_span.get_untracked();
                    let text = edit.text.clone();
                    let (new_text, new_start, new_end) =
                        splice_ref(&text, cursor, &ref_str, prev_span);
                    state.editing_cell.update(|c| {
                        if let Some(e) = c {
                            e.text = new_text;
                        }
                    });
                    state.point_range.set(Some([row, col, row, col]));
                    state.drag.set(DragState::Pointing);
                    state.point_ref_span.set(Some((new_start, new_end)));
                    state.request_redraw();
                    return;
                }
            }
        }

        // Apply model mutations and signal writes after the read closure.
        if near_handle {
            // Begin autofill drag — don't change the selection.
            state.drag.set(DragState::Extending {
                to_row: row,
                to_col: col,
            });
        } else if ev.shift_key() {
            // Shift-click extends the range from the current anchor.
            model.update_value(|m| {
                m.nav_extend_selection(row, col);
            });
            state.drag.set(DragState::Selecting);
        } else {
            model.update_value(|m| {
                m.nav_set_cell(row, col);
            });
            state.drag.set(DragState::Selecting);
        }

        state.editing_cell.set(None);
        state.request_redraw();
    };

    // ── mousemove: expand selection or autofill preview ──────────────────────
    let on_mousemove = move |ev: web_sys::MouseEvent| {
        // If no button is held the drag ended outside the canvas (mouseup was
        // missed). Reset all drag state so the next interaction starts clean.
        if ev.buttons() == 0 {
            state.drag.set(DragState::Idle);
            return;
        }
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        // ── Resize drags ──────────────────────────────────────────────────────
        match state.drag.get_untracked() {
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
                state.drag.set(DragState::ResizingCol { col, x });
                state.request_redraw();
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
                state.drag.set(DragState::ResizingRow { row, y });
                state.request_redraw();
                ev.prevent_default();
                return;
            }
            DragState::Idle
            | DragState::Selecting
            | DragState::Extending { .. }
            | DragState::Pointing => {}
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

        match state.drag.get_untracked() {
            DragState::Extending { .. } => {
                // Update autofill preview target.
                state.drag.set(DragState::Extending {
                    to_row: row,
                    to_col: col,
                });
                state.request_redraw();
            }
            DragState::Pointing => {
                // Extend the point-mode range to the hovered cell.
                if let Some([r1, c1, _, _]) = state.point_range.get_untracked() {
                    let sheet = model.with_value(|m| m.active_cell().sheet);
                    let ref_str = range_ref_str(r1, c1, row, col, sheet, sheet, "");
                    let prev_span = state.point_ref_span.get_untracked();
                    let cursor = prev_span.map(|(_, end)| end).unwrap_or(0);
                    let new_state = state
                        .editing_cell
                        .get_untracked()
                        .map(|edit| splice_ref(&edit.text, cursor, &ref_str, prev_span));
                    if let Some((new_text, new_start, new_end)) = new_state {
                        state.editing_cell.update(|c| {
                            if let Some(e) = c {
                                e.text = new_text;
                            }
                        });
                        state.point_range.set(Some([r1, c1, row, col]));
                        state.point_ref_span.set(Some((new_start, new_end)));
                        state.request_redraw();
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
                state.request_redraw();
            }
            DragState::Idle | DragState::ResizingCol { .. } | DragState::ResizingRow { .. } => {}
        }
    };

    // ── mouseup: commit autofill or end selection drag ────────────────────────
    let on_mouseup = move |_ev: web_sys::MouseEvent| {
        // Commit autofill if active, then reset drag state unconditionally.
        if let DragState::Extending { to_row, to_col } = state.drag.get_untracked() {
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
            state.request_redraw();
        }
        state.drag.set(DragState::Idle);
    };

    // ── dblclick: enter edit mode with existing cell content ──────────────────
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
            state.editing_cell.set(Some(EditingCell {
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
            state.context_menu.set(Some(ContextMenuState {
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
                // Small vertical delta -> single-row scroll (trackpad).
                if dy > 0.0 {
                    m.nav_arrow(ArrowKey::Down);
                } else {
                    m.nav_arrow(ArrowKey::Up);
                }
            } else {
                // Large vertical delta -> page scroll (mouse wheel).
                if dy > 0.0 {
                    m.nav_page(PageDir::Down);
                } else {
                    m.nav_page(PageDir::Up);
                }
            }
        });
        state.request_redraw();
    };

    view! {
        <div node_ref=container_ref class="worksheet-container">
            <canvas
                node_ref=canvas_ref
                role="application"
                aria-label="Spreadsheet grid"
                class=move || {
                    match state.drag.get() {
                        DragState::ResizingCol { .. } => "worksheet-canvas resize-col",
                        DragState::ResizingRow { .. } => "worksheet-canvas resize-row",
                        DragState::Idle
                        | DragState::Selecting
                        | DragState::Extending { .. }
                        | DragState::Pointing => "worksheet-canvas",
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
