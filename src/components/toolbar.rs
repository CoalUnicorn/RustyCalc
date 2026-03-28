use leptos::prelude::*;
use wasm_bindgen::UnwrapThrowExt;

use crate::input::action::{execute, SpreadsheetAction};
use crate::model::{FrontendModel, SafeFontFamily};
use crate::state::{ModelStore, WorkbookState};
use crate::util::warn_if_err;

const FONT_SIZES: &[f64] = &[
    6.0, 7.0, 8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0,
    36.0, 48.0, 72.0,
];

#[component]
pub fn Toolbar() -> impl IntoView {
    view! {
        <div class="toolbar">
            <UndoRedo />
            <div class="toolbar-sep" />
            <FontFamily />
            <div class="toolbar-sep" />
            <FontSize />
            <div class="toolbar-sep" />
            <FormatToggles />
            <div class="toolbar-sep" />
            <FreezePane />
        </div>
    }
}

// Undo / Redo
#[component]
fn UndoRedo() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let can_undo = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.can_undo())
    };
    let can_redo = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.can_redo())
    };

    let on_undo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::undo(), model, &state);
        crate::util::refocus_workbook();
    };
    let on_redo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::redo(), model, &state);
        crate::util::refocus_workbook();
    };

    view! {
        <button
            class="toolbar-btn"
            title="Undo (Ctrl+Z)"
            disabled=move || !can_undo()
            on:click=on_undo
        >
            "↺"
        </button>
        <button
            class="toolbar-btn"
            title="Redo (Ctrl+Y)"
            disabled=move || !can_redo()
            on:click=on_redo
        >
            "↻"
        </button>
    }
}

// Font family
#[component]
fn FontFamily() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let current_family = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.toolbar_state().font_family)
    };

    let on_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let target = ev
            .target()
            .unwrap_throw() // NOTE: Can this work ? instead of unwrap
            .unchecked_into::<web_sys::HtmlSelectElement>();
        let family = SafeFontFamily::from(Some(target.value().as_str()));
        execute(&SpreadsheetAction::set_font_family(family), model, &state);
        crate::util::refocus_workbook();
    };

    view! {
        <select class="toolbar-font-family" title="Font" on:change=on_change>
            {SafeFontFamily::ALL
                .iter()
                .map(|f| {
                    let model_name = f.model_name().to_owned();
                    let label = f.label();
                    let css = f.css_name().to_owned();
                    let family = *f;
                    view! {
                        <option
                            value=model_name
                            selected=move || current_family() == family
                            style=format!("font-family:{css}")
                        >
                            {label}
                        </option>
                    }
                })
                .collect::<Vec<_>>()}
        </select>
    }
}

// Font size
#[component]
fn FontSize() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let current_size = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.toolbar_state().font_size)
    };

    fn apply(size: f64, model: ModelStore, state: &WorkbookState) {
        execute(&SpreadsheetAction::set_font_size(size), model, state);
        crate::util::refocus_workbook();
    }

    let on_minus = move |_: web_sys::MouseEvent| {
        let next = snap_size(current_size(), SizeStep::Smaller);
        apply(next, model, &state);
    };

    let on_plus = move |_: web_sys::MouseEvent| {
        let next = snap_size(current_size(), SizeStep::Larger);
        apply(next, model, &state);
    };

    let on_blur = move |ev: web_sys::FocusEvent| {
        use wasm_bindgen::JsCast;
        if let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        {
            if let Ok(size) = input.value().parse::<f64>() {
                apply(size, model, &state);
            }
        }
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        use wasm_bindgen::JsCast;
        if ev.key() == "Enter" {
            ev.prevent_default();
            if let Some(input) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                if let Ok(size) = input.value().parse::<f64>() {
                    apply(size, model, &state);
                }
            }
        }
    };

    let display = move || {
        let s = current_size();
        if s.fract() == 0.0 {
            format!("{}", s as i32)
        } else {
            format!("{s}")
        }
    };

    view! {
        <button class="toolbar-btn font-size-btn" title="Decrease font size" on:click=on_minus>
            "−"
        </button>
        <input
            class="toolbar-font-size"
            type="text"
            title="Font size"
            prop:value=display
            on:blur=on_blur
            on:keydown=on_keydown
        />
        <button class="toolbar-btn font-size-btn" title="Increase font size" on:click=on_plus>
            "+"
        </button>
    }
}

enum SizeStep {
    Smaller,
    Larger,
}

/// Step through the standard font size ladder.
fn snap_size(current: f64, step: SizeStep) -> f64 {
    match step {
        SizeStep::Larger => FONT_SIZES
            .iter()
            .find(|&&s| s > current + 0.01)
            .copied()
            .unwrap_or(current + 1.0),
        SizeStep::Smaller => FONT_SIZES
            .iter()
            .rev()
            .find(|&&s| s < current - 0.01)
            .copied()
            .unwrap_or((current - 1.0).max(1.0)),
    }
}

// Bold / Italic / Underline / Strikethrough
#[component]
fn FormatToggles() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let toolbar_state = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.toolbar_state())
    };

    let is_bold = move || toolbar_state().bold;
    let is_italic = move || toolbar_state().italic;
    let is_underline = move || toolbar_state().underline;
    let is_strike = move || toolbar_state().strikethrough;

    let on_bold = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::toggle_bold(), model, &state);
        crate::util::refocus_workbook();
    };
    let on_italic = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::toggle_italic(), model, &state);
        crate::util::refocus_workbook();
    };
    let on_underline = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::toggle_underline(), model, &state);
        crate::util::refocus_workbook();
    };
    let on_strike = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::toggle_strikethrough(), model, &state);
        crate::util::refocus_workbook();
    };

    view! {
        <button
            class=move || if is_bold() { "toolbar-btn active" } else { "toolbar-btn" }
            title="Bold (Ctrl+B)"
            on:click=on_bold
        >
            <strong>"B"</strong>
        </button>
        <button
            class=move || if is_italic() { "toolbar-btn active" } else { "toolbar-btn" }
            title="Italic (Ctrl+I)"
            on:click=on_italic
        >
            <em>"I"</em>
        </button>
        <button
            class=move || if is_underline() { "toolbar-btn active" } else { "toolbar-btn" }
            title="Underline (Ctrl+U)"
            on:click=on_underline
        >
            <span style="text-decoration:underline">"U"</span>
        </button>
        <button
            class=move || if is_strike() { "toolbar-btn active" } else { "toolbar-btn" }
            title="Strikethrough"
            on:click=on_strike
        >
            <span style="text-decoration:line-through">"S"</span>
        </button>
    }
}

// Freeze panes

#[component]
fn FreezePane() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let is_frozen = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.frozen_panes().is_frozen())
    };

    let on_freeze = move |_: web_sys::MouseEvent| {
        model.update_value(|m| {
            let sheet = m.get_selected_view().sheet;
            let fp = m.frozen_panes();

            if fp.is_frozen() {
                warn_if_err(m.set_frozen_rows_count(sheet, 0), "set_frozen_rows_count");
                warn_if_err(
                    m.set_frozen_columns_count(sheet, 0),
                    "set_frozen_columns_count",
                );
            } else {
                let row = m.get_selected_view().row;
                let col = m.get_selected_view().column;
                if row > 1 || col > 1 {
                    warn_if_err(
                        m.set_frozen_rows_count(sheet, (row - 1).max(0)),
                        "set_frozen_rows_count",
                    );
                    warn_if_err(
                        m.set_frozen_columns_count(sheet, (col - 1).max(0)),
                        "set_frozen_columns_count",
                    );
                }
            }
        });
        state.request_redraw();
        crate::util::refocus_workbook();
    };

    let freeze_label = move || {
        if is_frozen() {
            "╔"
        } else {
            "╬"
        }
    };

    view! {
        <button class=move || if is_frozen() {"toolbar-btn active"} else {"toolbar-btn"}
            title=move || if is_frozen() {"Unfreeze panes"} else {"Freeze panes above and left of active cell"}
            on:click=on_freeze>
            {freeze_label}
        </button>
    }
}
