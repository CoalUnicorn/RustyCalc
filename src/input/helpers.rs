//! Shared helpers for action execution.

use ironcalc_base::expressions::types::Area;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::state::{ModelStore, WorkbookState};

/// Whether `mutate` should recalculate formulas after applying the closure.
///
/// Pass `Eval::Yes` when the mutation may change formula results
/// (cell writes, row/column inserts/deletes).
/// Pass `Eval::No` for pure navigation, selection, or formatting changes.
#[derive(Clone, Copy)]
pub enum Eval {
    Yes,
    No,
}

/// Run `f` on the model, optionally call `evaluate`.
///
/// **PERFORMANCE OPTIMIZED:** Many `UserModel` methods call `evaluate()` internally.
/// We pause evaluation before `f` so the model is evaluated at most once — after
/// all mutations are done. This prevents double evaluation and can halve execution time.
/// See docs/performance-evaluation.md for details.
///
/// **CALLER RESPONSIBILITY:** This function no longer automatically triggers redraws.
/// The caller must emit appropriate events using `state.emit_event()` or `state.request_redraw()`.
pub fn mutate(
    model: ModelStore,
    state: &WorkbookState,
    evaluate: Eval,
    f: impl FnOnce(&mut UserModel<'static>),
) {
    model.update_value(|m| {
        m.pause_evaluation();
        f(m);
        m.resume_evaluation();
        if matches!(evaluate, Eval::Yes) {
            m.evaluate();
        }
    });
    // No automatic redraw - caller must emit specific events
}

// Area needs its own type in input
/// Build an `Area` from selection corners, normalising min/max automatically.
pub fn make_area(sheet: u32, r1: i32, c1: i32, r2: i32, c2: i32) -> Area {
    Area {
        sheet,
        row: r1.min(r2),
        column: c1.min(c2),
        height: (r2 - r1).abs() + 1,
        width: (c2 - c1).abs() + 1,
    }
}

/// Build an `Area` covering the current selection (single cell or range).
pub fn selection_area(m: &UserModel<'static>) -> Area {
    let v = m.get_selected_view();
    let [r1, c1, r2, c2] = v.range;
    make_area(v.sheet, r1, c1, r2, c2)
}

/// Returns `((min_row, max_row), (min_col, max_col))` from a `[r1,c1,r2,c2]` range.
pub fn selection_bounds(range: [i32; 4]) -> ((i32, i32), (i32, i32)) {
    let [r1, c1, r2, c2] = range;
    ((r1.min(r2), r1.max(r2)), (c1.min(c2), c1.max(c2)))
}
