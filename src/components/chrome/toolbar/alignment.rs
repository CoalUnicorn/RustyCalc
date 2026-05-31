//! Horizontal (L/C/R) and vertical (Top/Middle/Bottom) alignment buttons.
//!
//! Each button toggles: clicking the active alignment reverts to the default
//! (`General` horizontally, `Bottom` vertically).

use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};
use leptos::prelude::*;

use super::icon::{Icon, IconName};
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::frontend_types::ToolbarState;
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

#[component]
pub fn AlignButtons() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let h_align = move || toolbar_state.with(|ts| ts.style.h_align.clone());

    // Each button needs: the target alignment, the button glyph, and the tooltip.
    // `is_active` maps the ironcalc variant to the canonical L/C/R bucket,
    // because Fill is a visual variant of Left and CenterContinuous of Center.
    let make_btn = move |target: HorizontalAlignment, icon: IconName, title: &'static str| {
        // Signal<bool> is Copy — both the class and click closures can capture it independently.
        let t = target.clone();
        let is_active = Signal::derive(move || match t {
            HorizontalAlignment::Left => {
                matches!(
                    h_align(),
                    HorizontalAlignment::Left | HorizontalAlignment::Fill
                )
            }
            HorizontalAlignment::Center => matches!(
                h_align(),
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous
            ),
            HorizontalAlignment::Right => matches!(h_align(), HorizontalAlignment::Right),
            _ => false,
        });

        view! {
            <button
                class=move || if is_active.get() { "tb-btn active" } else { "tb-btn" }
                title=title
                on:click=move |_: web_sys::MouseEvent| {
                    let next = if is_active.get_untracked() { HorizontalAlignment::General } else { target.clone() };
                    execute(&SpreadsheetAction::set_h_align(next), model, &state);
                    refocus_workbook();
                }
            >
                <Icon name=icon />
            </button>
        }
    };

    view! {
        {make_btn(HorizontalAlignment::Left,   IconName::AlignLeft,   "Align left")}
        {make_btn(HorizontalAlignment::Center, IconName::AlignCenter, "Align center")}
        {make_btn(HorizontalAlignment::Right,  IconName::AlignRight,  "Align right")}
    }
}

#[component]
pub fn VertAlignButtons() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let v_align = move || toolbar_state.with(|ts| ts.style.v_align.clone());

    let make_btn = move |target: VerticalAlignment, label: &'static str, title: &'static str| {
        let t = target.clone();
        let is_active = Signal::derive(move || v_align() == t);
        view! {
            <button
                class=move || if is_active.get() { "tb-btn active" } else { "tb-btn" }
                title=title
                on:click=move |_: web_sys::MouseEvent| {
                    let next = if is_active.get_untracked() { VerticalAlignment::Bottom } else { target.clone() };
                    execute(&SpreadsheetAction::set_v_align(next), model, &state);
                    refocus_workbook();
                }
            >
                {label}
            </button>
        }
    };

    view! {
        {make_btn(VerticalAlignment::Top,    "⬆", "Align top")}
        {make_btn(VerticalAlignment::Center, "↕", "Align middle")}
        {make_btn(VerticalAlignment::Bottom, "⬇", "Align bottom")}
    }
}
