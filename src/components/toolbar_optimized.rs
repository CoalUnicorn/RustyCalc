/*!
# Optimized Toolbar with Consolidated Memos

This demonstrates the "3 memos → 1 memo" optimization pattern using domain modeling 
and efficient Leptos reactivity patterns.

## Key Improvements

1. **Single Subscription**: One memo subscribes to format events, not 4+ separate ones
2. **PartialEq Types**: Efficient comparison with `ToolbarState: PartialEq`
3. **Derived Computations**: Extract individual values from the shared memo
4. **leptos-use Integration**: Use utilities for common patterns

## Performance Impact

- Before: 4+ separate `Memo::new` calls, each with format event subscription
- After: 1 shared memo + derived closures, single subscription
- Reactivity graph: Much simpler, fewer dependencies

The pattern applies to any component that extracts multiple related values from the same source.
*/

use leptos::*;
use leptos_use::*;

use crate::{
    input::{execute, SpreadsheetAction}, 
    model::{frontend_types::*, ModelStore, SafeFontFamily}, 
    state::WorkbookState
};

#[component]
pub fn OptimizedToolbar() -> impl IntoView {
    view! {
        <div class="toolbar">
            <UndoRedo/>
            <div class="toolbar-separator"/>
            <FontControls/>
            <div class="toolbar-separator"/>
            <FormatToggles/>
            <div class="toolbar-separator"/>
            <ColorControls/>
        </div>
    }
}

// ===============================================================================
// CONSOLIDATED TOOLBAR STATE PATTERN
// ===============================================================================

/// Provides shared toolbar state for all toolbar components
/// This replaces multiple separate memos with a single efficient one
#[component]  
pub fn ToolbarProvider(children: Children) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // SINGLE memo for all toolbar data (was 4+ separate memos)
    let toolbar_state = create_memo(move |_| {
        // Single subscription to format events
        let format_events = state.subscribe_to_format_events();
        let _ = format_events();
        
        // Single model access
        model.with_value(|m| m.toolbar_state())
    });
    
    // Also provide content state for undo/redo (was separate memos)
    let content_state = create_memo(move |_| {
        let content_events = state.subscribe_to_content_events();
        let _ = content_events();
        
        // Return tuple of both values we need
        model.with_value(|m| (m.can_undo(), m.can_redo()))
    });

    provide_context(toolbar_state);
    provide_context(content_state);
    
    children()
}

// ===============================================================================
// UNDO/REDO - Uses Content State
// ===============================================================================

#[component]
fn UndoRedo() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    
    // Use shared content state memo instead of creating separate ones
    let content_state = expect_context::<Memo<(bool, bool)>>();
    
    // Derived computations (no additional subscriptions needed)
    let can_undo = move || content_state.with(|(undo, _)| *undo);
    let can_redo = move || content_state.with(|(_, redo)| *redo);

    let on_undo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::undo(), model, &state);
        crate::util::refocus_workbook();
    };

    let on_redo = move |_: web_sys::MouseEvent| {
        execute(&SpreadsheetAction::redo(), model, &state);
        crate::util::refocus_workbook();
    };

    view! {
        <div class="undo-redo-group">
            <button
                class="toolbar-button undo"
                disabled=move || !can_undo()
                on:click=on_undo
                title="Undo"
            >
                "↶"
            </button>
            <button
                class="toolbar-button redo" 
                disabled=move || !can_redo()
                on:click=on_redo
                title="Redo"
            >
                "↷"
            </button>
        </div>
    }
}

// ===============================================================================
// FONT CONTROLS - Uses Toolbar State
// ===============================================================================

#[component]
fn FontControls() -> impl IntoView {
    let toolbar_state = expect_context::<Memo<ToolbarState>>();
    
    // Derived computations from shared state (no separate memos)
    let font_family = move || toolbar_state.with(|ts| ts.style.font_family);
    let font_size = move || toolbar_state.with(|ts| ts.style.font_size);

    view! {
        <div class="font-controls">
            <FontFamilySelector current_family=font_family />
            <FontSizeControls current_size=font_size />
        </div>
    }
}

#[component]
fn FontFamilySelector(current_family: impl Fn() -> SafeFontFamily + 'static) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let on_change = move |ev: web_sys::Event| {
        use wasm_bindgen::JsCast;
        let target = ev.target().unwrap().unchecked_into::<web_sys::HtmlSelectElement>();
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

#[component]
fn FontSizeControls(current_size: impl Fn() -> f64 + Copy + 'static) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Use leptos-use for common UI patterns
    let (dropdown_open, toggle_dropdown) = use_toggle(false);

    let apply_size = move |size: f64| {
        execute(&SpreadsheetAction::set_font_size(size), model, &state);
        crate::util::refocus_workbook();
    };

    let on_decrease = move |_: web_sys::MouseEvent| {
        let current = current_size();
        let new_size = snap_size(current, SizeStep::Smaller);
        apply_size(new_size);
    };

    let on_increase = move |_: web_sys::MouseEvent| {
        let current = current_size();
        let new_size = snap_size(current, SizeStep::Larger);
        apply_size(new_size);
    };

    view! {
        <div class="font-size-controls">
            <button 
                class="toolbar-button size-down" 
                on:click=on_decrease
                title="Decrease font size"
            >
                "A-"
            </button>
            
            <div class="font-size-display" class:open=dropdown_open>
                <button 
                    class="current-size" 
                    on:click=move |_| toggle_dropdown()
                >
                    {move || format!("{:.0}", current_size())}
                </button>
                
                <Show when=dropdown_open>
                    <div class="size-dropdown">
                        {FONT_SIZES.iter().map(|&size| {
                            view! {
                                <button 
                                    class="size-option"
                                    class:selected=move || (current_size() - size).abs() < 0.1
                                    on:click=move |_| {
                                        apply_size(size);
                                        toggle_dropdown();
                                    }
                                >
                                    {format!("{:.0}", size)}
                                </button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </Show>
            </div>
            
            <button 
                class="toolbar-button size-up" 
                on:click=on_increase
                title="Increase font size"
            >
                "A+"
            </button>
        </div>
    }
}

// ===============================================================================
// FORMAT TOGGLES - Uses Toolbar State  
// ===============================================================================

#[component]
fn FormatToggles() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();
    
    // Derived format state (no separate memo needed)
    let format = move || toolbar_state.with(|ts| ts.format.clone());

    let create_toggle = move |action: SpreadsheetAction| {
        move |_: web_sys::MouseEvent| {
            execute(&action, model, &state);
            crate::util::refocus_workbook();
        }
    };

    let on_bold = create_toggle(SpreadsheetAction::toggle_bold());
    let on_italic = create_toggle(SpreadsheetAction::toggle_italic());
    let on_underline = create_toggle(SpreadsheetAction::toggle_underline());

    view! {
        <div class="format-toggles">
            <button
                class="toolbar-button bold"
                class:active=move || format().bold
                on:click=on_bold
                title="Bold"
            >
                "B"
            </button>
            <button
                class="toolbar-button italic"
                class:active=move || format().italic
                on:click=on_italic
                title="Italic"
            >
                "I"
            </button>
            <button
                class="toolbar-button underline"
                class:active=move || format().underline
                on:click=on_underline
                title="Underline"
            >
                "U"
            </button>
        </div>
    }
}

// ===============================================================================
// COLOR CONTROLS - Combines Format + Theme State
// ===============================================================================

#[component]
fn ColorControls() -> impl IntoView {
    let toolbar_state = expect_context::<Memo<ToolbarState>>();
    
    // Derived color state from shared toolbar state
    let current_text_color = move || {
        toolbar_state.with(|ts| ts.style.text_color.to_string())
    };
    
    let current_bg_color = move || {
        toolbar_state.with(|ts| ts.style.bg_color.as_ref().map(|c| c.to_string()))
    };

    view! {
        <div class="color-controls">
            <TextColorPicker current_color=current_text_color />
            <BackgroundColorPicker current_color=current_bg_color />
        </div>
    }
}

#[component]
fn TextColorPicker(current_color: impl Fn() -> String + 'static) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    
    // Use leptos-use for toggle behavior
    let (picker_open, set_picker_open) = use_toggle(false);

    let on_color_change = move |color: String| {
        execute(&SpreadsheetAction::set_text_color(color), model, &state);
        set_picker_open(false);
        crate::util::refocus_workbook();
    };

    view! {
        <div class="color-picker-wrapper">
            <button 
                class="toolbar-button color-button text-color"
                on:click=move |_| set_picker_open(!picker_open.get())
                title="Text Color"
            >
                <div class="color-preview" style=move || format!("background-color: {}", current_color())/>
                "A"
            </button>
            
            <Show when=picker_open>
                <ColorPickerDropdown 
                    current_color=current_color
                    on_change=on_color_change 
                />
            </Show>
        </div>
    }
}

#[component]
fn BackgroundColorPicker(current_color: impl Fn() -> Option<String> + 'static) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    
    let (picker_open, set_picker_open) = use_toggle(false);

    let on_color_change = move |color: Option<String>| {
        let action = match color {
            Some(c) => SpreadsheetAction::set_background_color(c),
            None => SpreadsheetAction::clear_background_color(),
        };
        execute(&action, model, &state);
        set_picker_open(false);
        crate::util::refocus_workbook();
    };

    view! {
        <div class="color-picker-wrapper">
            <button 
                class="toolbar-button color-button bg-color"
                on:click=move |_| set_picker_open(!picker_open.get())
                title="Background Color"
            >
                <div 
                    class="color-preview" 
                    style=move || match current_color() {
                        Some(color) => format!("background-color: {}", color),
                        None => "background: repeating-linear-gradient(45deg, transparent, transparent 2px, #ccc 2px, #ccc 4px)".to_string()
                    }
                />
            </button>
            
            <Show when=picker_open>
                <BackgroundColorPickerDropdown 
                    current_color=current_color
                    on_change=on_color_change 
                />
            </Show>
        </div>
    }
}

// Placeholder components - implement with actual color picker UI
#[component]
fn ColorPickerDropdown(
    current_color: impl Fn() -> String + 'static,
    on_change: impl Fn(String) + 'static,
) -> impl IntoView {
    view! { <div>"Color picker dropdown"</div> }
}

#[component] 
fn BackgroundColorPickerDropdown(
    current_color: impl Fn() -> Option<String> + 'static,
    on_change: impl Fn(Option<String>) + 'static,
) -> impl IntoView {
    view! { <div>"Background color picker dropdown"</div> }
}

// ===============================================================================
// FONT SIZE UTILITIES (from existing code)
// ===============================================================================

const FONT_SIZES: &[f64] = &[8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 72.0];

enum SizeStep { Larger, Smaller }

fn snap_size(current: f64, step: SizeStep) -> f64 {
    match step {
        SizeStep::Larger => FONT_SIZES
            .iter()
            .find(|&&s| s > current + 0.01)
            .copied()
            .unwrap_or((current + 1.0).min(72.0)),
        SizeStep::Smaller => FONT_SIZES
            .iter()
            .rev()
            .find(|&&s| s < current - 0.01)
            .copied()
            .unwrap_or((current - 1.0).max(1.0)),
    }
}