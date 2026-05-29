//! `handle_mouseup`: commit drag → emit events, reset to Idle.

use leptos::prelude::*;

use crate::coord::{CellArea, SheetRange};
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::input::error::StructError;
use crate::model::{EvaluationMode, try_mutate};
use crate::state::{DragState, ModelStore, RefOverride, StatusMessage, WorkbookState};

use super::formula_ref::commit_formula_ref_drag;

/// re-runs on release and the overlay refreshes naturally.
pub fn handle_mouseup(_ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    state.autoscroll.cancel();
    let was_pointing = matches!(state.drag.get_untracked(), DragState::Pointing { .. });

    if let DragState::DraggingFormulaRef { ref_idx, .. } = state.drag.get_untracked()
        && let Some(RefOverride {
            range: new_range, ..
        }) = state.dragged_ref_override.get_untracked()
    {
        commit_formula_ref_drag(ref_idx, new_range, model, state);
    }

    if let DragState::Extending { to_row, to_col } = state.drag.get_untracked() {
        match try_mutate(
            model,
            EvaluationMode::Immediate,
            |m| -> Result<(), StructError> {
                let norm = CellArea::from_view(m).normalized();
                let area = norm.to_area(m.get_selected_sheet());
                if to_row < norm.r1 || to_row > norm.r2 {
                    m.auto_fill_rows(&area, to_row)
                        .map_err(StructError::Engine)?;
                } else {
                    m.auto_fill_columns(&area, to_col)
                        .map_err(StructError::Engine)?;
                }
                Ok(())
            },
        ) {
            Ok(()) => {
                let sheet_area = model.with_value(SheetRange::from_view);
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                    sheet_area,
                }));
            }
            Err(e) => state.status.set(Some(StatusMessage::Error(e.to_string()))),
        }
    }
    state.drag.set(DragState::Idle);
    state.dragged_ref_override.set(None);
    // After a point-mode drag, return focus to the formula input so the user
    // can continue typing the formula without clicking again.
    if was_pointing {
        state.refocus_formula_input();
    }
}
