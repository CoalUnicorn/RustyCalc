/*!
# Enhanced ColorPicker with Event-Driven State Management

Demonstrates how to integrate with the new event system for optimal performance.
Instead of subscribing to all changes via `request_redraw()`, components can
subscribe only to events they care about.

## Performance Benefits

- **Selective updates**: Only re-renders on color/theme changes, not cell edits
- **Type safety**: Can't accidentally subscribe to wrong events
- **Clear intent**: Obvious what triggers updates
- **Debugging**: Easy to trace why a component updated

## Migration Strategy

This file shows the "after" version. The original color_picker.rs remains
unchanged during migration for compatibility.
*/

use super::color_picker::{ColorPickerPlacement, ColorType};
use crate::events::*;
use crate::state::WorkbookState;
use crate::theme::{Theme, COLOR_PALETTE};
use leptos::prelude::*;
use leptos_use::use_toggle;

/// Enhanced color picker that subscribes to specific events
#[component]
pub fn EnhancedColorPicker(
    /// Type of color picker - affects CSS classes and behavior
    color_type: ColorType,
    /// Current color value (reactive) - used for highlighting selected color
    current_color: Signal<Option<String>>,
    /// Callback when color changes - receives Some(hex) or None for clear
    on_color_change: Callback<Option<String>>,
    /// Where to render the picker UI
    #[prop(default = ColorPickerPlacement::Dropdown)]
    placement: ColorPickerPlacement,
    /// Content to show in the trigger element
    children: Children,
    /// Whether to show custom hex color input
    #[prop(default = true)]
    allow_custom: bool,
    /// Whether to show "no color" clear button
    #[prop(default = true)]
    allow_clear: bool,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Optimized: Use leptos-use toggle for efficient boolean state
    let leptos_use::UseToggleReturn {
        toggle: toggle_picker,
        value: picker_open,
        set_value: set_picker_open,
    } = use_toggle(false);
    let custom_input: RwSignal<String> = RwSignal::new(String::new());

    // ===== Event-Driven Reactivity =====

    // Reactive computation that only runs on relevant changes
    let current_colors = Memo::new(move |_| {
        // Subscribe to format events and check for recent color updates
        let format_events = state.subscribe_to_format_events();
        let recent_updates = format_events()
            .into_iter()
            .filter_map(|e| match e {
                FormatEvent::RecentColorsUpdated { colors } => Some(colors),
                _ => None,
            })
            .last(); // Get the most recent update

        // Use event data if available, otherwise fall back to signal
        recent_updates.unwrap_or_else(|| state.get_recent_colors())
    });

    let current_theme = Memo::new(move |_| {
        // Subscribe to theme events and check for theme changes
        let theme_events = state.subscribe_to_theme_events();
        let theme_updates = theme_events()
            .into_iter()
            .filter_map(|e| match e {
                ThemeEvent::ThemeToggled { new_theme } => Some(new_theme),
                _ => None,
            })
            .last();

        theme_updates.unwrap_or_else(|| state.get_theme_preference())
    });

    // ===== Event Generation =====

    let select_color = move |color: Option<String>| {
        // Update the color through callback
        on_color_change.run(color.clone());

        // If adding to recent colors, use the enhanced method
        if let Some(ref hex) = color {
            state.add_recent_color(hex); // This automatically emits RecentColorsUpdated event
        }

        // Close picker using leptos-use controls
        set_picker_open.set(false);
        custom_input.set(String::new());
    };

    let container_class = format!(
        "color-picker color-picker-{} enhanced",
        color_type.css_class()
    );

    view! {
        <div class={container_class}>
            <button
                class="toolbar-btn color-picker-trigger"
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    // Toggle picker using leptos-use controls
                    toggle_picker();
                }
            >
                {children()}
            </button>

            <Show when=move || picker_open.get()>
                <EnhancedColorPickerDropdown
                    placement=placement
                    current_color=current_color
                    recent_colors=current_colors
                    current_theme=current_theme
                    custom_input=custom_input
                    allow_custom=allow_custom
                    allow_clear=allow_clear
                    on_color_select=select_color
                />
            </Show>
        </div>
    }
    .into_any()
}

/// Enhanced dropdown that uses computed colors and theme
#[component]
fn EnhancedColorPickerDropdown(
    placement: ColorPickerPlacement,
    current_color: Signal<Option<String>>,
    recent_colors: Memo<Vec<String>>, // Note: Memo instead of Signal
    current_theme: Memo<Theme>,       // Note: Memo instead of Signal
    custom_input: RwSignal<String>,
    allow_custom: bool,
    allow_clear: bool,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let picker_class = match placement {
        ColorPickerPlacement::Dropdown => "color-picker-dropdown enhanced",
        ColorPickerPlacement::Inline => "color-picker-inline enhanced",
    };

    view! {
        <div class={picker_class} class:dark=move || current_theme.get() == Theme::Dark>
            <EnhancedMainColorPalette
                current_color=current_color
                on_color_select=on_color_select
            />

            <EnhancedRecentColorsPalette
                recent_colors=recent_colors
                current_color=current_color
                on_color_select=on_color_select
            />

            <Show when=move || allow_custom>
                <EnhancedCustomColorInput
                    custom_input=custom_input
                    on_color_select=on_color_select
                />
            </Show>

            <Show when=move || allow_clear>
                <EnhancedClearColorButton on_color_select=on_color_select />
            </Show>
        </div>
    }
    .into_any()
}

/// Enhanced recent colors that uses memo for efficiency
#[component]
fn EnhancedRecentColorsPalette(
    recent_colors: Memo<Vec<String>>, // Uses memo - only updates when event fires
    current_color: Signal<Option<String>>,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let has_recent_colors = move || !recent_colors.get().is_empty();

    let is_selected = move |color: &str| {
        current_color
            .get()
            .map(|c| c.eq_ignore_ascii_case(color))
            .unwrap_or(false)
    };

    view! {
        <Show when=has_recent_colors>
            <div class="color-picker-recent-section enhanced">
                <div class="color-picker-recent-label">"Recent Colors"</div>
                <div class="color-picker-recent-palette">
                    // This For will only update when recent_colors memo changes
                    <For
                        each=move || recent_colors.get()
                        key=|color| color.clone()
                        children=move |hex| {
                            let selected = is_selected(&hex);
                            let swatch_class = if selected {
                                "color-picker-swatch color-picker-swatch--selected".to_string()
                            } else {
                                "color-picker-swatch".to_string()
                            };
                            let hex_clone = hex.clone();

                            view! {
                                <EnhancedColorSwatch
                                    hex=hex
                                    class_name=swatch_class
                                    on_click=move || on_color_select(Some(hex_clone.clone()))
                                />
                            }.into_any()
                        }
                    />
                </div>
            </div>
        </Show>
    }
}

/// Enhanced main color palette grid
#[component]
fn EnhancedMainColorPalette(
    current_color: Signal<Option<String>>,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let is_selected = move |palette_color: &str| {
        current_color
            .get()
            .map(|c| c.eq_ignore_ascii_case(palette_color))
            .unwrap_or(false)
    };

    view! {
        <div class="color-picker-palette">
            {COLOR_PALETTE.iter().map(|&hex| {
                let selected = is_selected(hex);
                let swatch_class = if selected {
                    "color-picker-swatch color-picker-swatch--selected".to_string()
                } else {
                    "color-picker-swatch".to_string()
                };

                view! {
                    <EnhancedColorSwatch
                        hex=hex.to_string()
                        class_name=swatch_class
                        on_click=move || on_color_select(Some(hex.to_string()))
                    />
                }.into_any()
            }).collect::<Vec<_>>()}
        </div>
    }
    .into_any()
}

/// Enhanced individual color swatch
#[component]
fn EnhancedColorSwatch(
    hex: String,
    class_name: String,
    on_click: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div
            class=class_name
            style=format!("background-color: {};", hex)
            title={hex.clone()}
            on:click=move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                on_click();
            }
        />
    }
    .into_any()
}

/// Enhanced custom color input
#[component]
fn EnhancedCustomColorInput(
    custom_input: RwSignal<String>,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let submit_custom_color = move |hex: String| {
        let trimmed = hex.trim();
        if trimmed.is_empty() {
            on_color_select(None);
        } else {
            let normalized = if trimmed.starts_with('#') {
                trimmed.to_string()
            } else {
                format!("#{}", trimmed)
            };

            if is_valid_hex_color(&normalized) {
                on_color_select(Some(normalized));
            }
        }
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            let value = custom_input.get();
            submit_custom_color(value);
        }
        "Escape" => {
            ev.prevent_default();
            custom_input.set(String::new());
        }
        _ => {}
    };

    let on_blur = move |_: web_sys::FocusEvent| {
        let value = custom_input.get();
        if !value.trim().is_empty() {
            submit_custom_color(value);
        }
    };

    view! {
        <div class="color-picker-custom">
            <label class="color-picker-custom-label">"Custom:"</label>
            <input
                type="text"
                class="color-picker-custom-input"
                placeholder="#hex"
                prop:value=move || custom_input.get()
                on:input=move |ev| {
                    let value = event_target_value(&ev);
                    custom_input.set(value);
                }
                on:keydown=on_keydown
                on:blur=on_blur
            />
        </div>
    }
    .into_any()
}

/// Enhanced clear color button
#[component]
fn EnhancedClearColorButton(
    on_color_select: impl Fn(Option<String>) + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <button
            class="color-picker-clear"
            on:click=move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                on_color_select(None);
            }
        >
            "No Color"
        </button>
    }
    .into_any()
}

/// Basic hex color validation
fn is_valid_hex_color(hex: &str) -> bool {
    if !hex.starts_with('#') {
        return false;
    }

    let digits = &hex[1..];
    if digits.len() != 3 && digits.len() != 6 {
        return false;
    }

    digits.chars().all(|c| c.is_ascii_hexdigit())
}

// ===== Convenience Components (Enhanced Versions) =====

#[component]
pub fn EnhancedTextColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
) -> impl IntoView {
    let color_indicator_style = move || {
        if let Some(color) = current_color.get() {
            format!("background-color: {};", color)
        } else {
            "background-color: transparent; border: 1px solid var(--border-color);".to_string()
        }
    };

    view! {
        <EnhancedColorPicker
            color_type=ColorType::Text
            current_color=current_color
            on_color_change=on_change
            placement=ColorPickerPlacement::Dropdown
        >
            <div class="color-indicator" style=color_indicator_style></div>
            "A"
        </EnhancedColorPicker>
    }
}

#[component]
pub fn EnhancedBackgroundColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
) -> impl IntoView {
    let color_indicator_style = move || {
        if let Some(color) = current_color.get() {
            format!("background-color: {};", color)
        } else {
            "background-color: transparent; border: 1px solid var(--border-color);".to_string()
        }
    };

    view! {
        <EnhancedColorPicker
            color_type=ColorType::Background
            current_color=current_color
            on_color_change=on_change
            placement=ColorPickerPlacement::Dropdown
        >
            <div class="fill-icon">"■"</div>
            <div class="color-indicator" style=color_indicator_style></div>
        </EnhancedColorPicker>
    }
}

// ===== Performance Comparison Example =====

/// Example component showing before/after performance characteristics
#[component]
pub fn ColorPickerPerformanceDemo() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let (current_color, set_current_color) = signal(Some("#ff0000".to_string()));

    view! {
        <div class="performance-demo">
            <h3>"Performance Comparison"</h3>

            <div class="demo-section">
                <h4>"Original (subscribes to all changes)"</h4>
                <super::color_picker::TextColorPicker
                    current_color=current_color.into()
                    on_change=Callback::new(move |color| set_current_color.set(color))
                    recent_colors=state.recent_colors.0.into()
                />
                <p class="performance-note">
                    "↑ Re-renders on any spreadsheet change (cell edits, navigation, etc.)"
                </p>
            </div>

            <div class="demo-section">
                <h4>"Enhanced (event-driven)"</h4>
                <EnhancedTextColorPicker
                    current_color=current_color.into()
                    on_change=Callback::new(move |color| set_current_color.set(color))
                />
                <p class="performance-note">
                    "↑ Only re-renders on color/theme changes"
                </p>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::*;
    use crate::model::CellAddress;

    // Mock test to show the concept - would need proper test setup
    #[test]
    fn test_event_filtering() {
        // This would test that color picker components only respond to relevant events
        let events = vec![
            SpreadsheetEvent::Content(ContentEvent::CellChanged {
                address: CellAddress {
                    sheet: 1,
                    row: 1,
                    column: 1,
                },
                old_value: None,
                new_value: None,
            }),
            SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated {
                colors: vec!["#ff0000".into()],
            }),
            SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled {
                new_theme: Theme::Dark,
            }),
        ];

        // Color picker should only care about the Format and Theme events
        let relevant_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, SpreadsheetEvent::Format(_) | SpreadsheetEvent::Theme(_)))
            .collect();

        assert_eq!(relevant_events.len(), 2);
    }
}
