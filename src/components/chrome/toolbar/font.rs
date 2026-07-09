//! Font family selector and font-size stepper.
//!
//! Size +/- buttons walk the standard `FONT_SIZES` ladder; the text input
//! accepts a direct value on blur / Enter.

use leptos::prelude::*;
use wasm_bindgen::UnwrapThrowExt;

use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::{SafeFontFamily, frontend_types::ToolbarState};
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

const FONT_SIZES: &[f64] = &[
    6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0,
    48.0, 72.0,
];

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

#[component]
pub fn FontFamily() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let current_family = move || toolbar_state.with(|ts| ts.style.font_family);

    let on_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let target = ev
            .target()
            .unwrap_throw()
            .unchecked_into::<web_sys::HtmlSelectElement>();
        let family = SafeFontFamily::from(Some(target.value().as_str()));
        execute(&SpreadsheetAction::set_font_family(family), model, &state);
        refocus_workbook();
    };

    view! {
        <select class="tb-font" title="Font" on:change=on_change>
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

#[component]
pub fn FontSize() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let current_size = move || toolbar_state.with(|ts| ts.style.font_size);

    fn apply(size: f64, model: ModelStore, state: &WorkbookState) {
        execute(&SpreadsheetAction::set_font_size(size), model, state);
        refocus_workbook();
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
            && let Ok(size) = input.value().parse::<f64>()
        {
            apply(size, model, &state);
        }
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        use wasm_bindgen::JsCast;
        if ev.key() == "Enter" {
            ev.prevent_default();
            if let Some(input) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                && let Ok(size) = input.value().parse::<f64>()
            {
                apply(size, model, &state);
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
        <button class="tb-btn tb-size-btn" title="Decrease font size" on:click=on_minus>
            "−"
        </button>
        <input
            class="tb-size"
            type="text"
            title="Font size"
            prop:value=display
            on:blur=on_blur
            on:keydown=on_keydown
        />
        <button class="tb-btn tb-size-btn" title="Increase font size" on:click=on_plus>
            "+"
        </button>
    }
}
