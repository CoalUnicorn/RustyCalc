/*!
# Reusable Color Picker Component

A flexible, modular color picker composed of focused sub-components.

## Architecture

```
ColorPicker (main orchestrator)
├── ColorPickerTrigger (button/link)
├── ColorPickerDropdown (container)
│   ├── MainColorPalette (40-color grid)
│   ├── RecentColorsPalette (recent colors grid)
│   ├── CustomColorInput (hex input field)
│   └── ClearColorButton ("no color" button)
```

Each sub-component has a single responsibility and can be tested/styled independently.
*/

use leptos::prelude::*;

use crate::theme::COLOR_PALETTE;

/// Color picker type - determines CSS classes and default behavior
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorType {
    /// Text/font color picker (toolbar)
    Text,
    /// Cell background color picker (toolbar)
    Background,
    /// Sheet tab color picker (tab bar context menu)
    Tab,
    /// Generic color picker (other contexts)
    Generic,
}

impl ColorType {
    /// CSS class suffix for this color type
    pub fn css_class(&self) -> &'static str {
        match self {
            ColorType::Text => "text",
            ColorType::Background => "background",
            ColorType::Tab => "tab",
            ColorType::Generic => "generic",
        }
    }
}

/// Where the color picker renders relative to its trigger
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorPickerPlacement {
    /// Dropdown below the trigger (toolbar buttons)
    Dropdown,
    /// Inline after the trigger (context menus)
    Inline,
}

/// Main color picker component - composes sub-components
#[component]
pub fn ColorPicker(
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
    /// Recent/custom colors signal
    #[prop(default = Signal::derive(|| Vec::new()))]
    recent_colors: Signal<Vec<String>>,
) -> impl IntoView {
    // Internal state: is the picker currently open?
    let picker_open: RwSignal<bool> = RwSignal::new(false);

    // Internal state: custom color input field value
    let custom_input: RwSignal<String> = RwSignal::new(String::new());

    // Handle color selection from any source
    let select_color = move |color: Option<String>| {
        on_color_change.run(color);
        picker_open.set(false);
        custom_input.set(String::new());
    };

    // Generate CSS class for the root container
    let container_class = format!("color-picker color-picker-{}", color_type.css_class());

    view! {
        <div class={container_class}>
            <ColorPickerTrigger
                placement=placement
                picker_open=picker_open
            >
                {children()}
            </ColorPickerTrigger>

            <Show when=move || picker_open.get()>
                <ColorPickerDropdown
                    placement=placement
                    current_color=current_color
                    recent_colors=recent_colors
                    custom_input=custom_input
                    allow_custom=allow_custom
                    allow_clear=allow_clear
                    on_color_select=select_color
                />
            </Show>
        </div>
    }
}

/// Trigger button/link that opens the color picker
#[component]
fn ColorPickerTrigger(
    placement: ColorPickerPlacement,
    picker_open: RwSignal<bool>,
    children: Children,
) -> impl IntoView {
    let toggle_picker = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        picker_open.update(|open| *open = !*open);
    };

    if placement == ColorPickerPlacement::Dropdown {
        view! {
            <button
                class="toolbar-btn color-picker-trigger"
                on:click=toggle_picker
            >
                {children()}
            </button>
        }
        .into_any()
    } else {
        view! {
            <div
                class="ctx-item color-picker-trigger"
                on:click=toggle_picker
            >
                {children()}
            </div>
        }
        .into_any()
    }
}

/// Container for the color picker dropdown/inline content
#[component]
fn ColorPickerDropdown(
    placement: ColorPickerPlacement,
    current_color: Signal<Option<String>>,
    recent_colors: Signal<Vec<String>>,
    custom_input: RwSignal<String>,
    allow_custom: bool,
    allow_clear: bool,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let picker_class = match placement {
        ColorPickerPlacement::Dropdown => "color-picker-dropdown",
        ColorPickerPlacement::Inline => "color-picker-inline",
    };

    view! {
        <div class={picker_class}>
            <MainColorPalette
                current_color=current_color
                on_color_select=on_color_select
            />

            <RecentColorsPalette
                recent_colors=recent_colors
                current_color=current_color
                on_color_select=on_color_select
            />

            <Show when=move || allow_custom>
                <CustomColorInput
                    custom_input=custom_input
                    on_color_select=on_color_select
                />
            </Show>

            <Show when=move || allow_clear>
                <ClearColorButton on_color_select=on_color_select />
            </Show>
        </div>
    }
}

/// Main 40-color palette grid
#[component]
fn MainColorPalette(
    current_color: Signal<Option<String>>,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    // Check if a color from the palette is currently selected
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
                    <ColorSwatch
                        hex=hex.to_string()
                        class_name=swatch_class
                        on_click=move || on_color_select(Some(hex.to_string()))
                    />
                }
            }).collect::<Vec<_>>()}
        </div>
    }
}

/// Recent colors palette (appears below main palette when colors exist)
#[component]
fn RecentColorsPalette(
    recent_colors: Signal<Vec<String>>,
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
            <div class="color-picker-recent-section">
                <div class="color-picker-recent-label">"Recent Colors"</div>
                <div class="color-picker-recent-palette">
                    {move || {
                        recent_colors.get().into_iter().map(|hex| {
                            let selected = is_selected(&hex);
                            let swatch_class = if selected {
                                "color-picker-swatch color-picker-swatch--selected".to_string()
                            } else {
                                "color-picker-swatch".to_string()
                            };
                            let hex_clone = hex.clone();

                            view! {
                                <ColorSwatch
                                    hex=hex
                                    class_name=swatch_class
                                    on_click=move || on_color_select(Some(hex_clone.clone()))
                                />
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>
        </Show>
    }
}

/// Individual color swatch (reusable)
#[component]
fn ColorSwatch(
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
}

/// Custom hex color input field
#[component]
fn CustomColorInput(
    custom_input: RwSignal<String>,
    on_color_select: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let submit_custom_color = move |hex: String| {
        let trimmed = hex.trim();
        if trimmed.is_empty() {
            on_color_select(None);
        } else {
            // Ensure hex starts with #
            let normalized = if trimmed.starts_with('#') {
                trimmed.to_string()
            } else {
                format!("#{}", trimmed)
            };

            // Basic hex validation
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
}

/// Clear color button
#[component]
fn ClearColorButton(
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

// Convenience Components

/// Create a text color picker for the toolbar
#[component]
pub fn TextColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(default = Signal::derive(|| Vec::new()))] recent_colors: Signal<Vec<String>>,
) -> impl IntoView {
    let color_indicator_style = move || {
        if let Some(color) = current_color.get() {
            format!("background-color: {};", color)
        } else {
            "background-color: transparent; border: 1px solid var(--border-color);".to_string()
        }
    };

    view! {
        <ColorPicker
            color_type=ColorType::Text
            current_color=current_color
            on_color_change=on_change
            placement=ColorPickerPlacement::Dropdown
            recent_colors=recent_colors
        >
            <div class="color-indicator" style=color_indicator_style></div>
            "A"
        </ColorPicker>
    }
}

/// Create a background color picker for the toolbar
#[component]
pub fn BackgroundColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(default = Signal::derive(|| Vec::new()))] recent_colors: Signal<Vec<String>>,
) -> impl IntoView {
    let color_indicator_style = move || {
        if let Some(color) = current_color.get() {
            format!("background-color: {};", color)
        } else {
            "background-color: transparent; border: 1px solid var(--border-color);".to_string()
        }
    };

    view! {
        <ColorPicker
            color_type=ColorType::Background
            current_color=current_color
            on_color_change=on_change
            placement=ColorPickerPlacement::Dropdown
            recent_colors=recent_colors
        >
            <div class="fill-icon">"■"</div>
            <div class="color-indicator" style=color_indicator_style></div>
        </ColorPicker>
    }
}

/// Create a tab color picker for context menus
#[component]
pub fn TabColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(default = Signal::derive(|| Vec::new()))] recent_colors: Signal<Vec<String>>,
) -> impl IntoView {
    view! {
        <ColorPicker
            color_type=ColorType::Tab
            current_color=current_color
            on_color_change=on_change
            placement=ColorPickerPlacement::Inline
            recent_colors=recent_colors
        >
            <span class="ctx-icon">"🎨"</span>
            "Change Color"
        </ColorPicker>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_color_validation() {
        // Valid colors
        assert!(is_valid_hex_color("#000"));
        assert!(is_valid_hex_color("#000000"));
        assert!(is_valid_hex_color("#ABC"));
        assert!(is_valid_hex_color("#abcdef"));
        assert!(is_valid_hex_color("#123456"));

        // Invalid colors
        assert!(!is_valid_hex_color("000")); // No #
        assert!(!is_valid_hex_color("#")); // Just #
        assert!(!is_valid_hex_color("#00")); // Too short
        assert!(!is_valid_hex_color("#0000")); // Wrong length
        assert!(!is_valid_hex_color("#00000")); // Wrong length
        assert!(!is_valid_hex_color("#0000000")); // Too long
        assert!(!is_valid_hex_color("#xyz")); // Invalid chars
        assert!(!is_valid_hex_color("#gggggg")); // Invalid chars
    }
}
