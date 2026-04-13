//! Mouse event handlers for the worksheet canvas.
//!
//! Most public functions follow the pattern used throughout `src/input/`:
//! pure logic that takes `(model: ModelStore, state: WorkbookState)` and
//! returns `()`. The two resize-begin helpers return `bool` to signal
//! whether a resize was started. The worksheet component holds thin
//! closures that delegate here.

use leptos::prelude::*;

use crate::canvas::geometry::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use crate::canvas::{
    autofill_handle_pos, frozen_geometry, pixel_to_col, pixel_to_row, AUTOFILL_HANDLE_PX,
    HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};
use crate::coord::{CellAddress, CellArea, SheetArea};
use crate::events::{ContentEvent, FormatEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::formula_input::{
    get_formula_cursor, is_in_reference_mode, range_ref_str, splice_ref,
};
use crate::model::{mutate, ArrowKey, EvaluationMode, FrontendModel, PageDir};
use crate::state::{
    ContextMenuState, DragState, EditFocus, EditMode, EditingCell, HeaderContextMenu, ModelStore,
    WorkbookState,
};
use crate::util::warn_if_err;
use ironcalc_base::UserModel;

/// Pixel tolerance for column/row resize hit-test in the header area.
const HIT_ZONE: f64 = 4.0;

/// Start a column resize if the click lands within `HIT_ZONE` of a column
/// boundary in the header row. Returns `true` if a resize was started.
pub fn try_begin_col_resize(
    ev: &web_sys::MouseEvent,
    x: f64,
    model: ModelStore,
    state: WorkbookState,
) -> bool {
    if let Some(col) =
        model.with_value(|m| crate::canvas::geometry::find_col_boundary_at(m, x, HIT_ZONE))
    {
        state.drag.set(DragState::ResizingCol { col, x });
        ev.prevent_default();
        true
    } else {
        false
    }
}

/// Start a row resize if the click lands within `HIT_ZONE` of a row
/// boundary in the header column. Returns `true` if a resize was started.
pub fn try_begin_row_resize(
    ev: &web_sys::MouseEvent,
    y: f64,
    model: ModelStore,
    state: WorkbookState,
) -> bool {
    if let Some(row) =
        model.with_value(|m| crate::canvas::geometry::find_row_boundary_at(m, y, HIT_ZONE))
    {
        state.drag.set(DragState::ResizingRow { row, y });
        ev.prevent_default();
        true
    } else {
        false
    }
}

/// Click on the top-left corner cell: select the entire sheet.
pub fn handle_corner_click(model: ModelStore, state: WorkbookState) {
    model.update_value(|m| {
        m.nav_select_all();
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetArea::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click on a column header: select the entire column, or extend the current
/// selection if Shift is held.
pub fn handle_col_header_click(
    ev: &web_sys::MouseEvent,
    x: f64,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        let view = m.get_selected_view();
        let sheet = view.sheet;
        let fg = frozen_geometry(m, sheet);
        let col = pixel_to_col(m, sheet, view.left_column, x, &fg);
        if ev.shift_key() {
            m.nav_extend_selection(1, col);
        } else {
            m.nav_select_column(col);
        }
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetArea::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click on a row header: select the entire row, or extend the current
/// selection if Shift is held.
pub fn handle_row_header_click(
    ev: &web_sys::MouseEvent,
    y: f64,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        let view = m.get_selected_view();
        let sheet = view.sheet;
        let fg = frozen_geometry(m, sheet);
        let row = pixel_to_row(m, sheet, view.top_row, y, &fg);
        if ev.shift_key() {
            m.nav_extend_selection(row, 1);
        } else {
            m.nav_select_row(row);
        }
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetArea::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click in the cell area: handles point-mode formula entry, autofill handle
/// drag start, Shift-click range extension, and regular single-cell navigation.
pub fn handle_cell_click(
    ev: &web_sys::MouseEvent,
    x: f64,
    y: f64,
    model: ModelStore,
    state: WorkbookState,
) {
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

    // Point mode: intercept click during formula entry.
    // When the cursor is at a syntactically valid reference position inside
    // a formula, clicking a cell inserts/replaces the reference rather than
    // committing the edit and navigating away.
    if let Some(ref edit) = state.editing_cell.get_untracked() {
        let already_pointing = matches!(state.drag.get_untracked(), DragState::Pointing { .. });
        let may_point = edit.mode == EditMode::Accept || edit.text_dirty || already_pointing;
        if may_point {
            let cursor = get_formula_cursor();
            if already_pointing || is_in_reference_mode(&edit.text, cursor) {
                let sheet = model.with_value(|m| m.active_cell().sheet);
                let ref_str = range_ref_str(row, col, row, col, sheet, sheet, "");
                let prev_span = if let DragState::Pointing { ref_span, .. } = state.drag.get() {
                    Some(ref_span)
                } else {
                    None
                };
                let text = edit.text.clone();
                let (new_text, new_start, new_end) = splice_ref(&text, cursor, &ref_str, prev_span);
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.text = new_text;
                    }
                });
                state.drag.set(DragState::Pointing {
                    range: CellArea::from_cell(row, col),
                    ref_span: (new_start, new_end),
                });
                return;
            }
        }
    }

    if near_handle {
        // Begin autofill drag - don't change the selection.
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

    // Emit the appropriate navigation event so toolbar/formula-bar
    // update and the canvas repaints via visual_events.
    if near_handle {
        // Autofill start: drag state change alone triggers the Effect.
    } else {
        if ev.shift_key() {
            let sheet_area = model.with_value(SheetArea::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged {
                    sheet_area: { sheet_area },
                },
            ));
        } else {
            let address = model.with_value(CellAddress::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionChanged { address },
            ));
        }
    }
}

/// Dispatch a mousedown event to the appropriate region handler.
///
/// Checks cursor position against header regions in priority order:
/// column-resize hit zone → row-resize hit zone → corner → col header →
/// row header → cell area.
pub fn handle_mousedown(ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;

    if y < HEADER_ROW_HEIGHT && x > HEADER_COL_WIDTH && try_begin_col_resize(&ev, x, model, state) {
        return;
    }
    if x < HEADER_COL_WIDTH && y > HEADER_ROW_HEIGHT && try_begin_row_resize(&ev, y, model, state) {
        return;
    }
    if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
        handle_corner_click(model, state);
        return;
    }
    if y < HEADER_ROW_HEIGHT && x >= HEADER_COL_WIDTH {
        handle_col_header_click(&ev, x, model, state);
        return;
    }
    if x < HEADER_COL_WIDTH && y >= HEADER_ROW_HEIGHT {
        handle_row_header_click(&ev, y, model, state);
        return;
    }
    handle_cell_click(&ev, x, y, model, state);
}

/// Expand selection, update resize drag, or update autofill/point-mode
/// preview while a button is held.
///
/// If no button is held when this fires, mouseup was missed (pointer left
/// the canvas). Reset drag state so the next interaction starts clean.
pub fn handle_mousemove(ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    if ev.buttons() == 0 {
        state.drag.set(DragState::Idle);
        return;
    }
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;

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
            let sheet = model.with_value(UserModel::get_selected_sheet);
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: Some(col),
                row: None,
            }));
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
            let sheet = model.with_value(UserModel::get_selected_sheet);
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: None,
                row: Some(row),
            }));
            ev.prevent_default();
            return;
        }
        DragState::Idle
        | DragState::Selecting
        | DragState::Extending { .. }
        | DragState::Pointing { .. } => {}
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
            state.drag.set(DragState::Extending {
                to_row: row,
                to_col: col,
            });
        }
        DragState::Pointing {
            range: pr,
            ref_span,
        } => {
            let sheet = model.with_value(UserModel::get_selected_sheet);
            let ref_str = range_ref_str(pr.r1, pr.c1, row, col, sheet, sheet, "");
            let cursor = ref_span.1;
            let new_state = state
                .editing_cell
                .get_untracked()
                .map(|edit| splice_ref(&edit.text, cursor, &ref_str, Some(ref_span)));
            if let Some((new_text, new_start, new_end)) = new_state {
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.text = new_text;
                    }
                });
                state.drag.set(DragState::Pointing {
                    range: CellArea {
                        r1: pr.r1,
                        c1: pr.c1,
                        r2: row,
                        c2: col,
                    },
                    ref_span: (new_start, new_end),
                });
            }
        }
        DragState::Selecting => {
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
            let sheet_area = model.with_value(SheetArea::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged { sheet_area },
            ));
        }
        DragState::Idle | DragState::ResizingCol { .. } | DragState::ResizingRow { .. } => {}
    }
}

/// Commit an autofill drag on button release, then reset drag state.
///
/// If no autofill drag was active, this is a no-op beyond resetting to `Idle`.
pub fn handle_mouseup(_ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    if let DragState::Extending { to_row, to_col } = state.drag.get_untracked() {
        mutate(model, EvaluationMode::Immediate, |m| {
            let norm = CellArea::from_view(m).normalized();
            let area = norm.to_area(m.get_selected_sheet());
            if to_row < norm.r1 || to_row > norm.r2 {
                warn_if_err(m.auto_fill_rows(&area, to_row), "auto_fill_rows");
            } else {
                warn_if_err(m.auto_fill_columns(&area, to_col), "auto_fill_columns");
            }
        });
        let sheet_area = model.with_value(SheetArea::from_view);
        state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
            sheet_area,
        }));
    }
    state.drag.set(DragState::Idle);
}

/// Right-click on a column or row header: store position and target for
/// the header context menu overlay.
///
/// Clicks in the cell grid are ignored — cell context menu not yet implemented.
pub fn handle_contextmenu(ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;

    let target = if y < HEADER_ROW_HEIGHT && x >= HEADER_COL_WIDTH {
        Some(HeaderContextMenu::Column(model.with_value(|m| {
            let v = m.get_selected_view();
            let fg = frozen_geometry(m, v.sheet);
            pixel_to_col(m, v.sheet, v.left_column, x, &fg)
        })))
    } else if x < HEADER_COL_WIDTH && y >= HEADER_ROW_HEIGHT {
        Some(HeaderContextMenu::Row(model.with_value(|m| {
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
}

/// Scroll the viewport on mouse wheel or trackpad swipe.
///
/// Trackpads emit many small-delta events; physical wheels emit large ones.
/// Small vertical deltas (< 100px) use single-row scroll; large ones use
/// page scroll. Horizontal deltas scroll left/right by one column.
pub fn handle_wheel(ev: web_sys::WheelEvent, model: ModelStore, state: WorkbookState) {
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
            // Small vertical delta — single-row scroll (trackpad).
            if dy > 0.0 {
                m.nav_arrow(ArrowKey::Down);
            } else {
                m.nav_arrow(ArrowKey::Up);
            }
        } else {
            // Large vertical delta — page scroll (mouse wheel).
            if dy > 0.0 {
                m.nav_page(PageDir::Down);
            } else {
                m.nav_page(PageDir::Up);
            }
        }
    });
    let (sheet, top_row, left_col) = model.with_value(|m| {
        let v = m.get_selected_view();
        (v.sheet, v.top_row, v.left_column)
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::ViewportScrolled {
            sheet,
            top_row,
            left_col,
        },
    ));
}

/// Enter edit mode with the existing cell content on double-click.
///
/// The preceding mousedown already navigated to the target cell, so this
/// only needs to open the editor at the current address.
pub fn handle_dblclick(ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;
    if x < HEADER_COL_WIDTH || y < HEADER_ROW_HEIGHT {
        return;
    }
    model.with_value(|m| {
        let ac = m.active_cell();
        let text = m.active_cell_content();
        state.editing_cell.set(Some(EditingCell {
            address: CellAddress {
                sheet: ac.sheet,
                row: ac.row,
                column: ac.column,
            },
            text,
            mode: EditMode::Edit,
            focus: EditFocus::Cell,
            text_dirty: false,
        }));
    });
}
