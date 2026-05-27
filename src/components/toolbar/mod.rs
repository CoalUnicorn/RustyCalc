mod number_format;

use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};
use leptos::prelude::*;
use wasm_bindgen::UnwrapThrowExt;

use crate::components::color_picker::{BackgroundColorPicker, TextColorPicker};
use crate::components::toolbar::number_format::NumberFormatPicker;
use crate::events::*;
use crate::input::error::FormatError;
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::{
    EvaluationMode, SafeFontFamily, SheetQuery, frontend_types::ToolbarState,
    style_types::HexColor, try_mutate,
};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
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

/// Top toolbar. Creates two shared memos once and provides them via context so
/// every sub-component reads the same reactive computation instead of each
/// instantiating its own (was: 4 x Memo, 12 subscriptions -> 2 x Memo, 6 subscriptions).
///
/// Context provided to children:
/// - `Memo<ToolbarState>`   - font size/family, bold/italic/color, etc.
/// - `Memo<(bool, bool)>`   - (can_undo, can_redo)
#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Re-runs on format changes (cell styling) AND navigation (selection change).
    // visual_events catches theme/canvas redraws that also affect cell style display.
    let toolbar_state: Memo<ToolbarState> = Memo::new(move |_| {
        let _ = state.events.format.get();
        let _ = state.events.navigation.get();
        let _ = state.events.content.get();
        model.with_value(|m| m.toolbar_state())
    });

    let undo_redo_state: Memo<(bool, bool)> = Memo::new(move |_| {
        let _ = state.events.content.get();
        model.with_value(|m| (m.can_undo(), m.can_redo()))
    });

    provide_context(toolbar_state);
    provide_context(undo_redo_state);

    view! {
        <div class="tb">
            <UndoRedo />
            <div class="tb-sep" />
            <NumberFormatPicker />
            <NumFmtQuickButtons />
            <div class="tb-sep" />
            <FontFamily />
            <div class="tb-sep" />
            <FontSize />
            <div class="tb-sep" />
            <TextColorPickerToolbar />
            <div class="tb-sep" />
            <BackgroundColorPickerToolbar />
            <div class="tb-sep" />
            <FormatToggles />
            <AlignButtons />
            <VertAlignButtons />
            <ClearFormat />
            <div class="tb-sep" />
            <FreezePane />
            <div class="tb-sep" />
            <NamedRangesButton />
        </div>
    }
}

// "Names" — opens the Manage Named Ranges modal.
#[component]
fn NamedRangesButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let on_click = move |_: web_sys::MouseEvent| {
        state.named_ranges_modal_open.set(true);
    };
    view! {
        <button
            class="tb-btn"
            title="Manage named ranges"
            on:click=on_click
        >
            "Names"
        </button>
    }
}

// Undo / Redo
#[component]
fn UndoRedo() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let undo_redo_state = expect_context::<Memo<(bool, bool)>>();

    let can_undo = move || undo_redo_state.with(|(undo, _)| *undo);
    let can_redo = move || undo_redo_state.with(|(_, redo)| *redo);

    let on_undo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::undo(), model, &state);
        refocus_workbook();
    };
    let on_redo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::redo(), model, &state);
        refocus_workbook();
    };

    view! {
        <button
            class="tb-btn"
            title="Undo (Ctrl+Z)"
            disabled=move || !can_undo()
            on:click=on_undo
        >
            "↺"
        </button>
        <button
            class="tb-btn"
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

// Font size - +/- buttons step through FONT_SIZES ladder; input accepts direct entry.
#[component]
fn FontSize() -> impl IntoView {
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

// Bold / Italic / Underline / Strikethrough
#[component]
fn FormatToggles() -> impl IntoView {
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
            <strong>"B"</strong>
        </button>
        <button
            class=move || if format().italic { "tb-btn active" } else { "tb-btn" }
            title="Italic (Ctrl+I)"
            on:click=on_italic
        >
            <em>"I"</em>
        </button>
        <button
            class=move || if format().underline { "tb-btn active" } else { "tb-btn" }
            title="Underline (Ctrl+U)"
            on:click=on_underline
        >
            <span style="text-decoration:underline">"U"</span>
        </button>
        <button
            class=move || if format().strikethrough { "tb-btn active" } else { "tb-btn" }
            title="Strikethrough"
            on:click=on_strike
        >
            <span style="text-decoration:line-through">"S"</span>
        </button>
    }
}

// Freeze panes - has its own layout-specific memo (not toolbar state).
#[component]
fn FreezePane() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let is_frozen = Memo::new(move |_| {
        let _ = state.events.format.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| m.frozen_panes().is_frozen())
    });

    let on_freeze = move |_: web_sys::MouseEvent| {
        let result = try_mutate(
            model,
            EvaluationMode::Deferred,
            |m| -> Result<(), FormatError> {
                let sheet = m.get_selected_sheet();
                let fp = m.frozen_panes();
                if fp.is_frozen() {
                    m.set_frozen_rows_count(sheet, 0)
                        .map_err(FormatError::Engine)?;
                    m.set_frozen_columns_count(sheet, 0)
                        .map_err(FormatError::Engine)?;
                } else {
                    let row = m.get_selected_view().row;
                    let col = m.get_selected_view().column;
                    if row > 1 || col > 1 {
                        m.set_frozen_rows_count(sheet, (row - 1).max(0))
                            .map_err(FormatError::Engine)?;
                        m.set_frozen_columns_count(sheet, (col - 1).max(0))
                            .map_err(FormatError::Engine)?;
                    }
                }
                Ok(())
            },
        );
        if let Err(e) = result {
            state.status.set(Some(StatusMessage::Error(e.to_string())));
            refocus_workbook();
            return;
        }
        state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
            sheet: model.with_value(|m| m.get_selected_view().sheet),
            col: None,
            row: None,
        }));
        refocus_workbook();
    };

    let freeze_label = move || if is_frozen.get() { "╔" } else { "╬" };

    view! {
        <button
            class=move || if is_frozen.get() { "tb-btn active" } else { "tb-btn" }
            title=move || if is_frozen.get() {
                "Unfreeze panes"
            } else {
                "Freeze panes above and left of active cell"
            }
            on:click=on_freeze
        >
            {freeze_label}
        </button>
    }
}

// Text Color Picker
#[component]
fn TextColorPickerToolbar() -> impl IntoView {
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

// Horizontal alignment — Left / Center / Right
#[component]
fn AlignButtons() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let h_align = move || toolbar_state.with(|ts| ts.style.h_align.clone());

    // Each button needs: the target alignment, the button glyph, and the tooltip.
    // `is_active` maps the ironcalc variant to the canonical L/C/R bucket,
    // because Fill is a visual variant of Left and CenterContinuous of Center.
    let make_btn = move |target: HorizontalAlignment, label: &'static str, title: &'static str| {
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
                {label}
            </button>
        }
    };

    view! {
        {make_btn(HorizontalAlignment::Left,   "⇤", "Align left")}
        {make_btn(HorizontalAlignment::Center, "⇔", "Align center")}
        {make_btn(HorizontalAlignment::Right,  "⇥", "Align right")}
    }
}

// Vertical alignment — Top / Middle / Bottom
#[component]
fn VertAlignButtons() -> impl IntoView {
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

// Number format quick-access: currency (£), percentage (%), and decimal ±
#[component]
fn NumFmtQuickButtons() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // let on_gbp = move |_: web_sys::MouseEvent| {
    //     execute(&SpreadsheetAction::set_num_fmt("£#,##0.00"), model, &state);
    //     refocus_workbook();
    // };
    let on_pct = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::set_num_fmt("0%"), model, &state);
        refocus_workbook();
    };
    let on_dec_less = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::decrease_decimals(), model, &state);
        refocus_workbook();
    };
    let on_dec_more = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::increase_decimals(), model, &state);
        refocus_workbook();
    };

    view! {
        // <button class="tb-btn" title="Currency (GBP)" on:click=on_gbp>"£"</button>
        <button class="tb-btn" title="Percentage" on:click=on_pct>"%"</button>
        <button class="tb-btn" title="Decrease decimal places" on:click=on_dec_less>".0<-"</button>
        <button class="tb-btn" title="Increase decimal places" on:click=on_dec_more>".0->"</button>
    }
}

// Clear all formatting from the selection
#[component]
fn ClearFormat() -> impl IntoView {
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

// Background Color Picker
#[component]
fn BackgroundColorPickerToolbar() -> impl IntoView {
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
