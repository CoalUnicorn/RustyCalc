use leptos::prelude::*;
use wasm_bindgen::UnwrapThrowExt;

use crate::components::color_picker_enhanced::{
    EnhancedBackgroundColorPicker, EnhancedTextColorPicker,
};
use crate::events::*;
use crate::input::action::{execute, SpreadsheetAction};
use crate::model::{frontend_types::ToolbarState, FrontendModel, SafeFontFamily};
use crate::state::{ModelStore, WorkbookState};
use crate::util::{refocus_workbook, warn_if_err};

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

    // TODO: can it be:
    // let content_state = expect_context::<Memo<(bool, bool)>>();
    let undo_redo_state = Memo::new(move |_| {
        let content_events = state.subscribe_to_content_events();
        let _ = content_events(); // Single subscription
        model.with_value(|m| (m.can_undo(), m.can_redo())) // Single model access
    });

    // Efficient derived computations (no additional subscriptions)
    let can_undo = move || undo_redo_state.with(|(undo, _)| *undo);
    let can_redo = move || undo_redo_state.with(|(_, redo)| *redo);

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

// SHARED TOOLBAR STATE - Single memo for all format-related components
// This replaces 4+ separate format event subscriptions with one efficient shared subscription
fn get_shared_toolbar_state() -> Memo<ToolbarState> {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    Memo::new(move |_| {
        let _ = state.subscribe_to_format_events()();
        let _ = state.subscribe_to_navigation_events()();
        let _ = state.subscribe_to_visual_events()();
        model.with_value(|m| m.toolbar_state()) // Single model access
    })
}

// SHARED COLOR STATE - Single memo for color components that need both format + theme events
fn get_shared_color_state() -> (Memo<ToolbarState>, Memo<()>) {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let format_state = Memo::new(move |_| {
        let format_events = state.subscribe_to_format_events();
        let _ = format_events();
        model.with_value(|m| m.toolbar_state())
    });

    // FIXME: This need its own place
    let theme_state = Memo::new(move |_| {
        let theme_events = state.subscribe_to_theme_events();
        let _ = theme_events();
        // Return unit for now, can extend with actual theme data later
    });

    (format_state, theme_state)
}

// Font family
#[component]
fn FontFamily() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Use shared toolbar state
    let toolbar_state = get_shared_toolbar_state();
    let current_family = move || toolbar_state.with(|ts| ts.style.font_family);

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

    // Use shared toolbar state
    let toolbar_state = get_shared_toolbar_state();
    let current_size = move || toolbar_state.with(|ts| ts.style.font_size);

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

    let toolbar_state = get_shared_toolbar_state();
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

// Freeze panes

#[component]
fn FreezePane() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // TODO: Use format events for now (freeze panes affects layout, could be moved to structure events later)
    let is_frozen = Memo::new(move |_| {
        let format_events = state.subscribe_to_format_events();
        let _ = format_events(); // Subscribe to layout changes
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
        // Emit layout change event instead of generic redraw
        state.emit_event(crate::events::SpreadsheetEvent::Format(
            crate::events::FormatEvent::LayoutChanged {
                sheet: model.with_value(|m| m.get_selected_view().sheet),
                col: None,
                row: None,
            },
        ));
        crate::util::refocus_workbook();
    };

    let freeze_label = move || {
        if is_frozen.get() {
            "╔"
        } else {
            "╬"
        }
    };

    view! {
        <button class=move || if is_frozen.get() {"toolbar-btn active"} else {"toolbar-btn"}
            title=move || if is_frozen.get() {"Unfreeze panes"} else {"Freeze panes above and left of active cell"}
            on:click=on_freeze>
            {freeze_label}
        </button>
    }
}

// Text Color Picker - Event-Driven Version
#[component]
fn TextColorPickerToolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let _model = expect_context::<ModelStore>();

    let toolbar_state = get_shared_toolbar_state();

    let (format_state, _theme_state) = get_shared_color_state();
    let current_color = Signal::derive(move || {
        format_state.with(|_ts| {
            // TODO: Get actual text color from toolbar state
            // Some(ts.style.text_color.to_string())
            None::<String>
        })
    });
    let color = move || toolbar_state.with(|ts| ts.style.text_color.clone());

    // Handle color change with event emission
    let on_color_change = Callback::new(move |color: Option<String>| {
        // Add to recent colors - this automatically emits RecentColorsUpdated event
        if let Some(ref hex) = color {
            state.add_recent_color(hex);
        }

        // TODO: Apply color to selected cells
        web_sys::console::log_2(
            &"Text color changed to:".into(),
            &format!("{:?}", color).into(),
        );

        // Demo colors for testing
        if state.get_recent_colors_untracked().is_empty() {
            state.add_recent_color("#ff6b6b"); // Coral red
            state.add_recent_color("#4ecdc4"); // Turquoise
            state.add_recent_color("#45b7d1"); // Sky blue
        }

        // Emit specific format events instead of generic redraw
        // TODO: When you implement actual formatting:
        // execute(&SpreadsheetAction::set_text_color(color), model, &state);
        // state.notify_cell_style_changed(sheet, row, col);
    });

    view! {
        <EnhancedTextColorPicker
            current_color=current_color
            on_change=on_color_change
        />
    }
}

// Background Color Picker - Event-Driven Version
#[component]
fn BackgroundColorPickerToolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let _model = expect_context::<ModelStore>();

    let (format_state, _theme_state) = get_shared_color_state();
    let current_color = Signal::derive(move || {
        format_state.with(|_ts| {
            // TODO: Get actual background color from toolbar state
            // Some(ts.style.bg_color.as_ref().map(|c| c.to_string()))
            None::<String>
        })
    });

    // Handle color change with event emission
    let on_color_change = Callback::new(move |color: Option<String>| {
        // Add to recent colors - automatically emits RecentColorsUpdated event
        if let Some(ref hex) = color {
            state.add_recent_color(hex);
        }

        web_sys::console::log_2(
            &"Background color changed to:".into(),
            &format!("{:?}", color).into(),
        );

        // TODO: Apply background color and emit specific events
        // execute(&SpreadsheetAction::set_background_color(color), model, &state);
        // state.notify_cell_style_changed(sheet, row, col);
    });

    view! {
        <EnhancedBackgroundColorPicker
            current_color=current_color
            on_change=on_color_change
        />
    }
}
