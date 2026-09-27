//! Merge-cells controls. Merge, merge & center, merge across, merge down, and
//! unmerge the current selection; the engine owns every validation rule, so its
//! error surfaces verbatim through the status bar.

use leptos::prelude::*;

use super::icon::{Icon, SheetIcon};
use crate::coord::CellArea;
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::input::structure::StructAction;
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

/// Whether the current selection can be merged and whether it is already
/// inside a merge. Read in one `Memo` so all five controls agree on one
/// answer.
#[derive(Clone, Copy, PartialEq, Eq)]
struct MergeState {
    /// The selection spans more than one cell and is not already merged.
    can_merge: bool,
    /// The selection intersects an existing merge.
    is_merged: bool,
}

#[component]
pub fn MergeCellsControls() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let merge_state = Memo::new(move |_| {
        // Tracks the same signals `FreezePane` tracks: a merge changes the
        // structure, and moving the selection changes the answer.
        let _ = state.events.structure.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| {
            let view = m.get_selected_view();
            let area = CellArea::from(view.range).normalized();
            let is_merged = m
                .get_merged_cells(view.sheet)
                .map(|merges| {
                    merges.iter().any(|mc| {
                        let r2 = mc.row + mc.height - 1;
                        let c2 = mc.column + mc.width - 1;
                        mc.row <= area.r2
                            && area.r1 <= r2
                            && mc.column <= area.c2
                            && area.c1 <= c2
                    })
                })
                .unwrap_or(false);
            MergeState {
                can_merge: !area.is_single_cell() && !is_merged,
                is_merged,
            }
        })
    });

    let run = move |action: StructAction| {
        execute(
            &SpreadsheetAction::Structure(action),
            model,
            &state,
        );
        refocus_workbook();
    };

    view! {
        <button
            class=move || if merge_state.get().can_merge { "tb-btn" } else { "tb-btn disabled" }
            disabled=move || !merge_state.get().can_merge
            title="Merge cells"
            on:click=move |_| run(StructAction::MergeCells)
        >
            <Icon icon=SheetIcon::Merge />
        </button>
        <button
            class=move || if merge_state.get().can_merge { "tb-btn" } else { "tb-btn disabled" }
            disabled=move || !merge_state.get().can_merge
            title="Merge and center"
            on:click=move |_| run(StructAction::MergeCellsCenter)
        >
            <Icon icon=SheetIcon::MergeCenter />
        </button>
        <button
            class=move || if merge_state.get().can_merge { "tb-btn" } else { "tb-btn disabled" }
            disabled=move || !merge_state.get().can_merge
            title="Merge across each row"
            on:click=move |_| run(StructAction::MergeCellsAcross)
        >
            <Icon icon=SheetIcon::MergeAcross />
        </button>
        <button
            class=move || if merge_state.get().can_merge { "tb-btn" } else { "tb-btn disabled" }
            disabled=move || !merge_state.get().can_merge
            title="Merge down each column"
            on:click=move |_| run(StructAction::MergeCellsDown)
        >
            <Icon icon=SheetIcon::MergeDown />
        </button>
        <button
            class=move || if merge_state.get().is_merged { "tb-btn active" } else { "tb-btn disabled" }
            disabled=move || !merge_state.get().is_merged
            title="Unmerge cells"
            on:click=move |_| run(StructAction::UnmergeCells)
        >
            <Icon icon=SheetIcon::Unmerge />
        </button>
    }
}
