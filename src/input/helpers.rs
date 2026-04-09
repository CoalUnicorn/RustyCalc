//! Shared helpers for action execution.

use ironcalc_base::expressions::types::Area;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::state::{ModelStore, WorkbookState};

/// Whether `mutate` should recalculate formulas after applying the closure.
///
/// Pass `EvaluationMode::Immediate` when the mutation may change formula results
/// (cell writes, row/column inserts/deletes).
/// Pass `EvaluationMode::Deferred` for pure navigation, selection, or formatting changes.
#[derive(Clone, Copy)]
pub enum EvaluationMode {
    Immediate,
    Deferred,
}

/// Run `f` on the model, optionally call `evaluate`.
///
/// **PERFORMANCE OPTIMIZED:** Many `UserModel` methods call `evaluate()` internally.
/// We pause evaluation before `f` so the model is evaluated at most once - after
/// all mutations are done. This prevents double evaluation and can halve execution time.
/// See docs/performance-evaluation.md for details.
///
/// **CALLER RESPONSIBILITY:** This function no longer automatically triggers redraws.
/// The caller must emit appropriate events using `state.emit_event()`.
///
pub fn mutate(
    model: ModelStore,
    _state: &WorkbookState,
    evaluate: EvaluationMode,
    f: impl FnOnce(&mut UserModel<'static>),
) {
    model.update_value(|m| {
        m.pause_evaluation();
        f(m);
        m.resume_evaluation();
        if matches!(evaluate, EvaluationMode::Immediate) {
            m.evaluate();
        }
    });
    // No automatic redraw - caller must emit specific events
}

/// Fallible variant of [`mutate`]: the closure returns `Result<(), E>`.
///
/// `resume_evaluation()` always runs to leave the model in a consistent state.
/// `evaluate()` is skipped when the closure returns `Err`.
pub fn try_mutate<E>(
    model: ModelStore,
    _state: &WorkbookState,
    evaluate: EvaluationMode,
    f: impl FnOnce(&mut UserModel<'static>) -> Result<(), E>,
) -> Result<(), E> {
    let mut outcome: Result<(), E> = Ok(());
    model.update_value(|m| {
        m.pause_evaluation();
        outcome = f(m);
        m.resume_evaluation();
        if outcome.is_ok() && matches!(evaluate, EvaluationMode::Immediate) {
            m.evaluate();
        }
    });
    outcome
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
