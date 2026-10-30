//! Drag operations for direct formula references.

use leptos::prelude::*;

use crate::coord::{CellAddress, FormulaRefKind, LAST_COLUMN, LAST_ROW, SheetRange};
use crate::input::formula::splice_dragged_ref;
use crate::model::FormulaAnalyzer;
use crate::state::{DragState, ModelStore, WorkbookState};
use iron_canvas::{FormulaRefHit, RefCorner, RefZone};
use iron_canvas_core::Side;

pub(super) fn draggable_ref_indices(refs: &[crate::coord::ActiveRef]) -> Vec<usize> {
    refs.iter()
        .enumerate()
        .filter_map(|(index, reference)| {
            matches!(reference.kind, FormulaRefKind::Direct).then_some(index)
        })
        .collect()
}

pub(super) fn handle_formula_ref_mousedown(
    ev: &web_sys::MouseEvent,
    hit: FormulaRefHit,
    state: WorkbookState,
) {
    let Some(editing) = state.editing_cell.get_untracked() else {
        return;
    };
    let Some(reference) = editing.formula_analysis.refs().get(hit.ref_idx) else {
        return;
    };
    if !matches!(reference.kind, FormulaRefKind::Direct) {
        return;
    }

    let anchor = reference.sheet_area;
    let grab_cell = CellAddress {
        sheet: anchor.sheet,
        row: hit.grab_cell.row,
        column: hit.grab_cell.col,
    };
    state.drag.set(DragState::DraggingFormulaRef {
        ref_idx: hit.ref_idx,
        zone: hit.zone,
        anchor,
        grab_cell,
        preview: anchor,
    });
    ev.prevent_default();
}

pub(super) fn commit_formula_ref_drag(
    ref_idx: usize,
    new_range: SheetRange,
    model: ModelStore,
    state: WorkbookState,
) {
    let Some(edit) = state.editing_cell.get_untracked() else {
        return;
    };
    let Some(reference) = edit.formula_analysis.refs().get(ref_idx) else {
        return;
    };
    let Some((new_text, new_span)) = splice_dragged_ref(
        &edit.text,
        reference.span,
        &reference.ref_node,
        new_range,
        edit.address,
    ) else {
        return;
    };

    state.editing_cell.update(|current| {
        if let Some(edit) = current {
            edit.cursor = new_span.end;
            edit.formula_analysis = model.with_value(|m| m.analyze_at(&new_text, edit.address));
            edit.text = new_text;
        }
    });
}

pub(crate) fn dragged_ref_range(
    anchor: SheetRange,
    zone: RefZone,
    grab_cell: CellAddress,
    cursor: CellAddress,
) -> SheetRange {
    let area = anchor.area;
    let resize = |row1: i32, col1: i32, row2: i32, col2: i32| {
        let row1 = row1.max(1).min(row2);
        let col1 = col1.max(1).min(col2);
        let row2 = row2.clamp(1, LAST_ROW);
        let col2 = col2.clamp(1, LAST_COLUMN);
        SheetRange::new(anchor.sheet, row1, col1, row2, col2)
    };

    match zone {
        RefZone::Body => {
            let normalized = area.normalized();
            let delta_row = cursor.row - grab_cell.row;
            let delta_column = cursor.column - grab_cell.column;
            let max_row1 = LAST_ROW - normalized.height() + 1;
            let max_col1 = LAST_COLUMN - normalized.width() + 1;
            let row1 = (normalized.r1 + delta_row).clamp(1, max_row1);
            let col1 = (normalized.c1 + delta_column).clamp(1, max_col1);
            let actual_row_delta = row1 - normalized.r1;
            let actual_column_delta = col1 - normalized.c1;
            SheetRange::new(
                anchor.sheet,
                row1,
                col1,
                normalized.r2 + actual_row_delta,
                normalized.c2 + actual_column_delta,
            )
        }
        RefZone::Edge(Side::Top) => resize(cursor.row, area.c1, area.r2, area.c2),
        RefZone::Edge(Side::Bottom) => resize(area.r1, area.c1, cursor.row, area.c2),
        RefZone::Edge(Side::Left) => resize(area.r1, cursor.column, area.r2, area.c2),
        RefZone::Edge(Side::Right) => resize(area.r1, area.c1, area.r2, cursor.column),
        RefZone::Corner(RefCorner::TopLeft) => resize(cursor.row, cursor.column, area.r2, area.c2),
        RefZone::Corner(RefCorner::TopRight) => resize(cursor.row, area.c1, area.r2, cursor.column),
        RefZone::Corner(RefCorner::BottomLeft) => {
            resize(area.r1, cursor.column, cursor.row, area.c2)
        }
        RefZone::Corner(RefCorner::BottomRight) => {
            resize(area.r1, area.c1, cursor.row, cursor.column)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(row: i32, column: i32) -> CellAddress {
        CellAddress {
            sheet: 3,
            row,
            column,
        }
    }

    #[test]
    fn body_drag_keeps_range_size_and_tracks_the_grabbed_cell() {
        assert_eq!(
            dragged_ref_range(
                SheetRange::new(3, 4, 4, 6, 6),
                RefZone::Body,
                address(5, 5),
                address(7, 9),
            ),
            SheetRange::new(3, 6, 8, 8, 10),
        );
    }

    #[test]
    fn edge_and_corner_drags_resize_only_the_selected_sides() {
        let anchor = SheetRange::new(3, 4, 4, 6, 6);
        assert_eq!(
            dragged_ref_range(
                anchor,
                RefZone::Edge(Side::Top),
                address(4, 4),
                address(2, 4),
            ),
            SheetRange::new(3, 2, 4, 6, 6),
        );
        assert_eq!(
            dragged_ref_range(
                anchor,
                RefZone::Corner(RefCorner::BottomRight),
                address(6, 6),
                address(8, 9),
            ),
            SheetRange::new(3, 4, 4, 8, 9),
        );
    }
}
