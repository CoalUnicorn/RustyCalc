//! Text and background color pickers. Each wraps the shared `color_picker`
//! UI component, feeds it the current cell color, and applies the chosen color
//! while recording it in the recent-colors list.

use leptos::prelude::*;

use crate::components::ui::color_picker::{BackgroundColorPicker, TextColorPicker};
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::{frontend_types::ToolbarState, style_types::HexColor};
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn TextColorPickerToolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let current_color = Signal::derive(move || {
        HexColor::new(toolbar_state.with(|ts| ts.style.text_color.as_str().to_owned())).ok()
    });

    let on_color_change = Callback::new(move |color: Option<HexColor>| {
        if let Some(ref hex) = color {
            state.add_recent_color(hex.as_str());
        }
        execute(
            &SpreadsheetAction::set_text_color(color.unwrap_or_else(HexColor::transparent)),
            model,
            &state,
        );
        refocus_workbook();
    });

    view! {
        <TextColorPicker current_color=current_color on_change=on_color_change />
    }
}

#[component]
pub fn BackgroundColorPickerToolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let current_color = Signal::derive(move || {
        toolbar_state.with(|ts| {
            ts.style
                .bg_color
                .as_ref()
                .and_then(|c| HexColor::new(c.as_str()).ok())
        })
    });

    let on_color_change = Callback::new(move |color: Option<HexColor>| {
        if let Some(ref hex) = color {
            state.add_recent_color(hex.as_str());
        }
        execute(
            &SpreadsheetAction::set_background_color(color.unwrap_or_else(HexColor::transparent)),
            model,
            &state,
        );
        refocus_workbook();
    });

    view! {
        <BackgroundColorPicker current_color=current_color on_change=on_color_change />
    }
}
