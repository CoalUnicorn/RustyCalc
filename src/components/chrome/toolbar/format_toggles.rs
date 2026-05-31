//! Character-style toggles (Bold / Italic / Underline / Strikethrough) and the
//! "Clear formatting" button.

use leptos::prelude::*;

use super::icon::{Icon, IconName};
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::frontend_types::ToolbarState;
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn FormatToggles() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let format = move || toolbar_state.with(|ts| ts.format.clone());

    let create_toggle = move |action: SpreadsheetAction| {
        move |_: web_sys::MouseEvent| {
            execute(&action, model, &state);
            refocus_workbook();
        }
    };

    let on_bold = create_toggle(SpreadsheetAction::toggle_bold());
    let on_italic = create_toggle(SpreadsheetAction::toggle_italic());
    let on_underline = create_toggle(SpreadsheetAction::toggle_underline());
    let on_strike = create_toggle(SpreadsheetAction::toggle_strikethrough());

    view! {
        <button
            class=move || if format().bold { "tb-btn active" } else { "tb-btn" }
            title="Bold (Ctrl+B)"
            on:click=on_bold
        >
            <Icon name=IconName::Bold />
        </button>
        <button
            class=move || if format().italic { "tb-btn active" } else { "tb-btn" }
            title="Italic (Ctrl+I)"
            on:click=on_italic
        >
            <Icon name=IconName::Italic />
        </button>
        <button
            class=move || if format().underline { "tb-btn active" } else { "tb-btn" }
            title="Underline (Ctrl+U)"
            on:click=on_underline
        >
            <Icon name=IconName::Underline />
        </button>
        <button
            class=move || if format().strikethrough { "tb-btn active" } else { "tb-btn" }
            title="Strikethrough"
            on:click=on_strike
        >
            <Icon name=IconName::Strikethrough />
        </button>
    }
}

#[component]
pub fn ClearFormat() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    view! {
        <button
            class="tb-btn"
            title="Clear formatting"
            on:click=move |_: web_sys::MouseEvent| {
                execute(&SpreadsheetAction::clear_formatting(), model, &state);
                refocus_workbook();
            }
        >
            "✕"
        </button>
    }
}
