//! `<FormulaOverlay>` — colored ref-token overlay rendered behind a
//! transparent `<textarea>` or `<input>` while a formula is being edited.
//!
//! The two callers (cell editor + formula bar) wrap their text element in
//! an `.fe-host` div and drop a `FormulaOverlay` as the first child. The
//! component subscribes to the live `text` and `refs` signals from
//! `WorkbookState.editing_cell`, so colors update on every keystroke —
//! `sync_edit` already re-runs `formula_analysis` synchronously, keeping
//! `ActiveRef.color_idx` stable across keystrokes for the same target cell.
//!
//! ## Performance
//!
//! `split_formula_by_refs` returns byte ranges + palette indices — zero
//! per-segment `String` allocations. The formula text is stored once in
//! the Memo output, and the view function slices `&str` references from it.
//! For `=A1+B2+C3` with 3 refs, this saves 6 heap allocations per keystroke
//! compared to the old design that cloned each segment into an owned String.

use iron_canvas_core::theme::FORMULA_REF_COLORS;
use leptos::prelude::*;

use crate::coord::ActiveRef;
use crate::input::formula_overlay::split_formula_by_refs;

#[component]
pub fn FormulaOverlay(
    #[prop(into)] text: Signal<String>,
    #[prop(into)] refs: Signal<Vec<ActiveRef>>,
    /// `true` for the cell-editor textarea (wraps); `false` for the
    /// formula-bar input (single line, horizontal scroll).
    #[prop(optional)]
    multiline: bool,
    /// Optional `NodeRef` so callers can scroll-sync without
    /// `query_selector` on every scroll tick.
    #[prop(optional)]
    node_ref: NodeRef<leptos::html::Div>,
) -> impl IntoView {
    let palette_len = FORMULA_REF_COLORS.len();

    /// Holds the formula text + segment ranges so the view can slice `&str`
    /// without cloning the formula per segment.
    #[derive(Clone, PartialEq)]
    struct OverlayData {
        formula: String,
        segments: Vec<crate::input::formula_overlay::FormulaSegment>,
    }

    let overlay_data = Memo::new(move |_| {
        let formula = text.get();
        let r = refs.get();
        let segments = split_formula_by_refs(&formula, &r, palette_len);
        OverlayData { formula, segments }
    });

    let overlay_class = if multiline {
        "fe-overlay fe-text fe-overlay--multiline"
    } else {
        "fe-overlay fe-text fe-overlay--singleline"
    };

    view! {
        <div class=overlay_class aria-hidden="true" node_ref=node_ref>
            {move || {
                let data = overlay_data.get();
                let palette = FORMULA_REF_COLORS;
                data.segments
                    .into_iter()
                    .map(|seg| {
                        let slice = &data.formula[seg.range];
                        match seg.color_idx {
                            Some(idx) => {
                                let color = palette[idx as usize % palette.len()];
                                view! {
                                    <span class="fe-ref" style:color=color>{slice.to_string()}</span>
                                }.into_any()
                            }
                            None => view! {
                                <span class="fe-op">{slice.to_string()}</span>
                            }.into_any(),
                        }
                    })
                    .collect_view()
            }}
        </div>
    }
}
