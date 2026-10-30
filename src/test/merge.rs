//! Merge command execution: the host sends the operation to the engine and
//! surfaces the engine's own error, and every accepted change announces
//! `StructureEvent::MergedCellsChanged`.

use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::input::structure::StructAction;
use crate::model::EvaluationMode;
use crate::state::{ModelStore, StatusMessage, WorkbookState};
use ironcalc_base::UserModel;
use leptos::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn new_model() -> UserModel<'static> {
    match UserModel::new_empty("test", "en", "UTC", "en") {
        Ok(m) => m,
        Err(e) => panic!("empty workbook must construct: {e}"),
    }
}

fn struc(a: StructAction) -> SpreadsheetAction {
    SpreadsheetAction::Structure(a)
}

/// Inclusive engine bounds of the first merge on sheet 0, or `None`.
fn first_merge(model: ModelStore) -> Option<(i32, i32, i32, i32)> {
    model.with_value(|m| {
        m.get_merged_cells(0).ok().and_then(|merges| {
            merges.first().map(|mc| {
                (
                    mc.row,
                    mc.column,
                    mc.row + mc.height - 1,
                    mc.column + mc.width - 1,
                )
            })
        })
    })
}

fn merge_count(model: ModelStore) -> usize {
    model.with_value(|m| m.get_merged_cells(0).map(|v| v.len()).unwrap_or(0))
}

/// Select `(r1, c1)..=(r2, c2)`. The engine requires the active cell to lie
/// inside the range, so the anchor is set first — the same order the model's
/// own column/row selection helpers use.
#[allow(clippy::unwrap_used)]
fn select(model: ModelStore, r1: i32, c1: i32, r2: i32, c2: i32) {
    crate::model::mutate(model, EvaluationMode::Immediate, |m| {
        m.set_selected_cell(r1, c1).unwrap();
        m.set_selected_range(r1, c1, r2, c2).unwrap();
    });
}

#[allow(clippy::unwrap_used)]
#[wasm_bindgen_test]
fn merge_cells_merges_the_selection_and_announces_it() {
    let owner = Owner::new();
    owner.with(|| {
        let model = StoredValue::new_local(new_model());
        let state = WorkbookState::new(crate::events::EventBus::new());
        select(model, 2, 2, 3, 4);

        execute(&struc(StructAction::MergeCells), model, &state);

        assert_eq!(first_merge(model), Some((2, 2, 3, 4)));
        assert_eq!(state.status.get_untracked(), None);
        let announced = state
            .events
            .structure
            .get_untracked()
            .iter()
            .any(|e| matches!(e, crate::events::StructureEvent::MergedCellsChanged { .. }));
        assert!(
            announced,
            "an accepted merge must announce it, or an idle renderer never repaints"
        );
    });
}

/// The engine rejects a merge with more than one content cell. The host
/// surfaces that error verbatim and leaves the model unmerged.
#[allow(clippy::unwrap_used)]
#[wasm_bindgen_test]
fn merge_rejects_more_than_one_content_cell() {
    let owner = Owner::new();
    owner.with(|| {
        let model = StoredValue::new_local(new_model());
        let state = WorkbookState::new(crate::events::EventBus::new());
        crate::model::mutate(model, EvaluationMode::Immediate, |m| {
            m.set_user_input(0, 2, 2, "a").ok();
            m.set_user_input(0, 2, 3, "b").ok();
        });
        select(model, 2, 2, 2, 3);

        execute(&struc(StructAction::MergeCells), model, &state);

        assert_eq!(merge_count(model), 0, "a rejected merge must not apply");
        assert!(
            matches!(state.status.get_untracked(), Some(StatusMessage::Error(_))),
            "the engine's rejection must reach the status bar"
        );
    });
}

/// A merge with exactly one content cell moves that content to the anchor.
#[allow(clippy::unwrap_used)]
#[wasm_bindgen_test]
fn merge_moves_a_sole_covered_content_cell_to_the_anchor() {
    let owner = Owner::new();
    owner.with(|| {
        let model = StoredValue::new_local(new_model());
        let state = WorkbookState::new(crate::events::EventBus::new());
        // Content only in the covered (bottom-right) cell.
        crate::model::mutate(model, EvaluationMode::Immediate, |m| {
            m.set_user_input(0, 3, 3, "moved").ok();
        });
        select(model, 2, 2, 3, 3);

        execute(&struc(StructAction::MergeCells), model, &state);

        assert_eq!(first_merge(model), Some((2, 2, 3, 3)));
        let anchor = model.with_value(|m| m.get_formatted_cell_value(0, 2, 2).unwrap_or_default());
        assert_eq!(anchor, "moved");
    });
}

/// Merge-across is one undo step: undo restores the unmerged range.
#[allow(clippy::unwrap_used)]
#[wasm_bindgen_test]
fn merge_across_is_one_undo_step() {
    let owner = Owner::new();
    owner.with(|| {
        let model = StoredValue::new_local(new_model());
        let state = WorkbookState::new(crate::events::EventBus::new());
        select(model, 2, 1, 3, 2);

        execute(&struc(StructAction::MergeCellsAcross), model, &state);
        assert_eq!(merge_count(model), 2, "one merge per row");

        execute(&SpreadsheetAction::undo(), model, &state);
        assert_eq!(merge_count(model), 0, "one undo must clear every row");

        execute(&SpreadsheetAction::redo(), model, &state);
        assert_eq!(merge_count(model), 2);
    });
}

#[allow(clippy::unwrap_used)]
#[wasm_bindgen_test]
fn unmerge_restores_the_range() {
    let owner = Owner::new();
    owner.with(|| {
        let model = StoredValue::new_local(new_model());
        let state = WorkbookState::new(crate::events::EventBus::new());
        select(model, 2, 2, 3, 4);
        execute(&struc(StructAction::MergeCells), model, &state);
        assert_eq!(merge_count(model), 1);

        execute(&struc(StructAction::UnmergeCells), model, &state);
        assert_eq!(merge_count(model), 0);
    });
}

/// The autofill ghost and the commit must submit the same target, and the
/// snapped target must be one the engine accepts. A drag that stops inside a
/// merge previews the merge's far edge; submitting the raw pointer cell instead
/// asks for a fill that cuts the merge, which the engine rejects.
#[allow(clippy::unwrap_used)]
#[wasm_bindgen_test]
fn autofill_preview_and_commit_submit_one_target() {
    use crate::input::mouse::resolved_fill_target;
    use crate::model::try_mutate;
    use ironcalc_base::expressions::types::Area;

    let owner = Owner::new();
    owner.with(|| {
        let model = StoredValue::new_local(new_model());
        crate::model::mutate(model, EvaluationMode::Immediate, |m| {
            m.set_user_input(0, 1, 1, "src").ok();
            m.merge_cells(&Area {
                sheet: 0,
                row: 6,
                column: 1,
                width: 1,
                height: 3,
            })
            .ok();
        });
        select(model, 1, 1, 1, 1);

        // The pointer stops at row 7, inside the merge at A6:A8.
        let target = resolved_fill_target(model, 7, 1);
        assert_eq!(
            target.row, 8,
            "the preview and the commit both use the merge's far edge"
        );

        let source = Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 1,
        };
        let raw = try_mutate(model, EvaluationMode::Immediate, |m| {
            m.auto_fill_rows(&source, 7)
        });
        assert!(
            raw.is_err(),
            "the engine must reject the raw target that cuts the merge"
        );
        let snapped = try_mutate(model, EvaluationMode::Immediate, |m| {
            m.auto_fill_rows(&source, target.row)
        });
        assert!(
            snapped.is_ok(),
            "the snapped target must be the operation the engine accepts: {snapped:?}"
        );
        assert_eq!(
            model.with_value(|m| m.get_formatted_cell_value(0, 8, 1).unwrap_or_default()),
            "src",
            "the accepted fill must actually reach the merge's far row"
        );
    });
}

/// A covered pointer resolves to the merge's anchor link and to the whole
/// merged fragment, so the hover tooltip anchors to the cell the user sees.
#[wasm_bindgen_test]
fn tooltip_resolves_a_covered_pointer_to_the_anchor_link_and_fragment() {
    use crate::components::panels::link_tooltip::hovered_link;
    use crate::scene::{SceneHandle, request_for};
    use iron_canvas::{OverlayState, RevisionToken};
    use iron_canvas_core::{CanvasSize, CanvasTheme, CellCoord};
    use ironcalc_base::expressions::types::Area;
    use ironcalc_base::types::Link;
    use wasm_bindgen::JsCast;

    Owner::new().with(|| {
        let state = crate::state::WorkbookState::new(crate::events::EventBus::new());
        let document = web_sys::window()
            .expect("window")
            .document()
            .expect("document");
        let make_canvas = || {
            document
                .create_element("canvas")
                .expect("element")
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .expect("canvas")
        };
        let mut m = new_model();
        m.merge_cells(&Area {
            sheet: 0,
            row: 2,
            column: 2,
            width: 3,
            height: 2,
        })
        .expect("merge");
        m.set_cell_link(
            0,
            2,
            2,
            Link::Internal {
                location: "A20".to_string(),
                tooltip: Some("destination".to_string()),
            },
            None,
        )
        .expect("anchor link");
        let size = CanvasSize { w: 400.0, h: 300.0 };
        let mut handle = SceneHandle::new(make_canvas(), size, 1.0).expect("scene session");
        let request = request_for(
            0,
            CellCoord { row: 1, col: 1 },
            size,
            CanvasTheme::light(),
            OverlayState::default(),
            RevisionToken {
                workbook_id: 0,
                revision: 0,
            },
        );
        let model = StoredValue::new_local(m);
        model.with_value(|model| handle.render(model, &request).expect("render"));
        let point = handle.cell_rect(3, 3).expect("covered cell").center();
        let slot = StoredValue::new_local(Some(handle));
        let cell = hovered_link(
            slot,
            model,
            state,
            Some((f64::from(point.x), f64::from(point.y))),
        )
        .expect("logical tooltip");
        // The covered pointer resolves through the merge to the anchor's link
        // (its text) and to the whole merged fragment (its width).
        assert_eq!(cell.text, "destination");
        assert!(cell.rect.width > 80);
    });
}
