//! Row-height autofit on cell commits.
//!
//! Excel grows a row to fit its content when text wraps to multiple lines
//! (an Alt+Enter newline, or a long line soft-wrapped under `wrap_text`). The
//! commit pipeline (`execute_edit`) is deliberately canvas-free, so it can't
//! measure glyphs. This reactive effect lives where the `CanvasHandle` does
//! and watches `ContentEvent::CellChanged`: it asks the renderer to measure
//! the edited row's fitted height and, *grow-only*, dispatches `SetRowHeight`.
//!
//! Grow-only is deliberate — shrinking would fight a row the user dragged
//! taller. Re-fit-down stays on the double-click-the-border gesture.
//!
//! No feedback loop: this watches `content`; `SetRowHeight` emits only a
//! `Format` event (`StructAction::SetRowHeight` → `FormatEvent::LayoutChanged`).

use leptos::prelude::*;

use crate::events::ContentEvent;
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::input::mouse::CanvasHandle;
use crate::input::structure::StructAction;
use crate::model::SheetQuery;
use crate::state::{ModelStore, WorkbookState};
use iron_canvas_core::geometry::constants::DEFAULT_ROW_HEIGHT;

/// Slack so a fitted height a hair above the current one (float rounding in
/// the measure pass) doesn't churn a redundant resize + undo step.
const GROW_EPSILON: f64 = 0.5;

pub(super) fn install_autofit_effect(
    state: WorkbookState,
    canvas_handle: CanvasHandle,
    model: ModelStore,
) {
    Effect::new(move |_| {
        let events = state.events.content.get();
        if events.is_empty() {
            return;
        }

        // A single commit emits one CellChanged, but a batch (multi-cell edit)
        // can touch several rows — fit each affected row once.
        let mut rows: Vec<i32> = events
            .iter()
            .filter_map(|ev| match ev {
                ContentEvent::CellChanged { address, .. } => Some(address.row),
                _ => None,
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        rows.sort_unstable();
        rows.dedup();

        // The row span to measure across is the sheet's used-column range —
        // the same span the double-click autofit feeds `fit_row_height`.
        let sheet = model.with_value(|m| m.get_selected_sheet());
        let dim = model.with_value(|m| m.sheet_dimension());

        for row in rows {
            // Renderer measures wrapped line count against live model text.
            let Some(fitted) = canvas_handle.with_value(|slot| {
                slot.as_ref()
                    .and_then(|ic| ic.fit_row_height(row, dim.c1, dim.c2))
            }) else {
                continue;
            };
            let current = model
                .with_value(|m| m.get_row_height(sheet, row))
                .unwrap_or(DEFAULT_ROW_HEIGHT);
            if fitted > current + GROW_EPSILON {
                execute(
                    &SpreadsheetAction::Structure(StructAction::SetRowHeight {
                        row,
                        count: 1,
                        height: fitted,
                    }),
                    model,
                    &state,
                );
            }
        }
    });
}
