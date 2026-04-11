use leptos::prelude::*;
use wasm_bindgen::UnwrapThrowExt;

use crate::components::color_picker::{BackgroundColorPicker, TextColorPicker};
use crate::events::*;
use crate::input::action::{execute, SpreadsheetAction};
use crate::model::{
    frontend_types::ToolbarState, style_types::HexColor, FrontendModel, SafeFontFamily,
};
use crate::state::{ModelStore, WorkbookState};
use crate::util::{refocus_workbook, warn_if_err};

const FONT_SIZES: &[f64] = &[
    6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0,
    48.0, 72.0,
];

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
        <div class="toolbar">
            <UndoRedo />
            <div class="toolbar-sep" />
            <FontFamily />
            <div class="toolbar-sep" />
            <FontSize />
            <div class="toolbar-sep" />
            <TextColorPickerToolbar />
            <div class="toolbar-sep" />
            <BackgroundColorPickerToolbar />
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
            class=move || if format().bold { "toolbar-btn active" } else { "toolbar-btn" }
            title="Bold (Ctrl+B)"
            on:click=on_bold
        >
            <strong>"B"</strong>
        </button>
        <button
            class=move || if format().italic { "toolbar-btn active" } else { "toolbar-btn" }
            title="Italic (Ctrl+I)"
            on:click=on_italic
        >
            <em>"I"</em>
        </button>
        <button
            class=move || if format().underline { "toolbar-btn active" } else { "toolbar-btn" }
            title="Underline (Ctrl+U)"
            on:click=on_underline
        >
            <span style="text-decoration:underline">"U"</span>
        </button>
        <button
            class=move || if format().strikethrough { "toolbar-btn active" } else { "toolbar-btn" }
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
            class=move || if is_frozen.get() { "toolbar-btn active" } else { "toolbar-btn" }
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
        Some(toolbar_state.with(|ts| ts.style.text_color.as_str().to_owned()))
    });

    let on_color_change = Callback::new(move |color: Option<String>| {
        if let Some(ref hex) = color {
            state.add_recent_color(hex);
        }
        execute(
            &SpreadsheetAction::set_text_color(HexColor::from_opt(color)),
            model,
            &state,
        );
        refocus_workbook();
    });

    view! {
        <TextColorPicker current_color=current_color on_change=on_color_change />
    }
}

// Background Color Picker
#[component]
fn BackgroundColorPickerToolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let current_color = Signal::derive(move || {
        toolbar_state.with(|ts| ts.style.bg_color.as_ref().map(|c| c.as_str().to_owned()))
    });

    let on_color_change = Callback::new(move |color: Option<String>| {
        if let Some(ref hex) = color {
            state.add_recent_color(hex);
        }
        execute(
            &SpreadsheetAction::set_background_color(HexColor::from_opt(color)),
            model,
            &state,
        );
        refocus_workbook();
    });

    view! {
        <BackgroundColorPicker current_color=current_color on_change=on_color_change />
    }
}
