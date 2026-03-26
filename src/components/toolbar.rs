use ironcalc_base::expressions::types::Area;
use ironcalc_base::types::HorizontalAlignment;
use ironcalc_base::UserModel;
use leptos::prelude::*;
use leptos::tachys::view;
use wasm_bindgen::JsCast;

use crate::model::FrontendModel;
use crate::state::{ModelStore, WorkbookState};

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let btn_ref = NodeRef::<leptos::html::Button>::new();

    let toolbar_state = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.toolbar_state())
    };

    let can_undo = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.can_undo())
    };
    let can_redo = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.can_redo())
    };

    let redraw = state.redraw.get();
    let _ = move || {
        let _ = state.redraw.get();
        let fp = model.with_value(|m| m.frozen_panes());
        fp.is_frozen()
    };

    view! {
        <div id="toolbar" class="toolbar">
            <button>"Undo ↺"</button>
            <button>"Redo ↻"</button>

        <button
        node_ref=btn_ref

            aria-label="Freeze Panes"
            title="Freeze Panes (freeze at active cell / unfreeze)"
        >"╔"
        </button>
        </div>
    }
}

fn toolbar_btn_style(active: bool) -> String {
    unimplemented!();
}
