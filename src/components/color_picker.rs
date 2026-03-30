/*!
# Reusable Color Picker Component

A flexible, reusable color picker that can be used across different contexts in the application.

## Usage Patterns

### 1. Toolbar Font Color
```rust
<ColorPicker
    color_type=ColorType::Text
    current_color=move || toolbar_state().text_color.map(|c| c.as_str().to_string())
    on_color_change=Callback::new(move |color| {
        execute(&SpreadsheetAction::set_text_color(color), model, &state);
    })
    placement=ColorPickerPlacement::Dropdown
    trigger_content=view! { <div class="color-indicator"></div> "A" }.into_view()
/>
```

## Component Architecture

The component follows the **"pure component"** pattern:
- **No direct model mutations** - uses callbacks to notify parent of color changes
- **Configurable via props** - behavior adapts to different use cases
- **Self-contained state** - manages its own open/closed state
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

/// Reusable color picker component
/// 
/// ## Props
/// - `color_type`: Visual/behavior variant (Text, Background, Tab, Generic)
/// - `current_color`: Currently selected color (for highlighting) 
/// - `on_color_change`: Callback when user picks a color
/// - `placement`: Dropdown (toolbar) or Inline (menus)
/// - `trigger_content`: What to show in the trigger button/link
/// - `allow_custom`: Show hex input field (default: true)
/// - `allow_clear`: Show "no color" button (default: true)
/// - `recent_colors`: Recent/custom colors to show below palette (optional)
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

    // Toggle picker open/closed state
    let toggle_picker = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        picker_open.update(|open| *open = !*open);
    };

    // Handle color selection from palette or custom input
    let select_color = move |color: Option<String>| {
        // Call the parent's callback
        on_color_change.run(color);
        // Close the picker
        picker_open.set(false);
        // Clear custom input
        custom_input.set(String::new());
    };

    // Handle custom color input submission
    let submit_custom_color = move |hex: String| {
        let trimmed = hex.trim();
        if trimmed.is_empty() {
            select_color(None);
        } else {
            // Ensure hex starts with #
            let normalized = if trimmed.starts_with('#') {
                trimmed.to_string()
            } else {
                format!("#{}", trimmed)
            };
            
            // Basic hex validation (could be more sophisticated)
            if is_valid_hex_color(&normalized) {
                select_color(Some(normalized));
            }
            // If invalid, just ignore (could show error state)
        }
    };

    // Check if a color from the palette is currently selected
    let is_color_selected = move |palette_color: &str| {
        current_color.get()
            .map(|c| c.eq_ignore_ascii_case(palette_color))
            .unwrap_or(false)
    };

    // Generate CSS class for the root container
    let container_class = format!("color-picker color-picker-{}", color_type.css_class());

    // Handle custom color input key events
    let on_custom_input_keydown = move |ev: web_sys::KeyboardEvent| {
        match ev.key().as_str() {
            "Enter" => {
                ev.prevent_default();
                let value = custom_input.get();
                submit_custom_color(value);
            }
            "Escape" => {
                ev.prevent_default();
                picker_open.set(false);
                custom_input.set(String::new());
            }
            _ => {}
        }
    };

    // Handle custom input blur (focus lost)
    let on_custom_input_blur = move |_: web_sys::FocusEvent| {
        let value = custom_input.get();
        if !value.trim().is_empty() {
            submit_custom_color(value);
        }
    };

    view! {
        <div class={container_class}>
            // Trigger element (button for dropdown, clickable item for inline)
            {if placement == ColorPickerPlacement::Dropdown {
                view! {
                    <button 
                        class="toolbar-btn color-picker-trigger"
                        on:click=toggle_picker
                    >
                        {children()}
                    </button>
                }.into_any()
            } else {
                view! {
                    <div 
                        class="ctx-item color-picker-trigger"
                        on:click=toggle_picker
                    >
                        {children()}
                    </div>
                }.into_any()
            }}

            // Color picker UI (shown when picker_open is true)
            <Show when=move || picker_open.get()>
                {
                    let picker_class = match placement {
                        ColorPickerPlacement::Dropdown => "color-picker-dropdown",
                        ColorPickerPlacement::Inline => "color-picker-inline",
                    };

                    view! {
                        <div class={picker_class}>
                            // Color palette grid
                            <div class="color-picker-palette">
                                {COLOR_PALETTE.iter().enumerate().map(|(_i, &hex)| {
                                    let is_selected = is_color_selected(hex);
                                    let swatch_class = move || {
                                        if is_selected {
                                            "color-picker-swatch color-picker-swatch--selected"
                                        } else {
                                            "color-picker-swatch"
                                        }
                                    };
                                    
                                    view! {
                                        <div
                                            class={swatch_class}
                                            style=format!("background-color: {};", hex)
                                            title={hex}
                                            on:click=move |ev: web_sys::MouseEvent| {
                                                ev.stop_propagation();
                                                select_color(Some(hex.to_string()));
                                            }
                                        />
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                            
                            // Recent colors section (if any exist)
                            <Show when=move || !recent_colors.get().is_empty()>
                                <div class="color-picker-recent-section">
                                    <div class="color-picker-recent-label">"Recent Colors"</div>
                                    <div class="color-picker-recent-palette">
                                        {move || {
                                            recent_colors.get().into_iter().map(|hex| {
                                                let is_selected = is_color_selected(&hex);
                                                let swatch_class = move || {
                                                    if is_selected {
                                                        "color-picker-swatch color-picker-swatch--selected"
                                                    } else {
                                                        "color-picker-swatch"
                                                    }
                                                };
                                                let hex_clone = hex.clone();
                                                
                                                view! {
                                                    <div
                                                        class={swatch_class}
                                                        style=format!("background-color: {};", hex)
                                                        title={hex.clone()}
                                                        on:click=move |ev: web_sys::MouseEvent| {
                                                            ev.stop_propagation();
                                                            select_color(Some(hex_clone.clone()));
                                                        }
                                                    />
                                                }
                                            }).collect::<Vec<_>>()
                                        }}
                                    </div>
                                </div>
                            </Show>

                            // Custom color input (if enabled)
                            <Show when=move || allow_custom>
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
                                        on:keydown=on_custom_input_keydown
                                        on:blur=on_custom_input_blur
                                    />
                                </div>
                            </Show>

                            // Clear color button (if enabled)
                            <Show when=move || allow_clear>
                                <button
                                    class="color-picker-clear"
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        ev.stop_propagation();
                                        select_color(None);
                                    }
                                >
                                    "No Color"
                                </button>
                            </Show>
                        </div>
                    }
                }
            </Show>
        </div>
    }
}

/// Basic hex color validation
/// 
/// Checks if a string looks like a valid hex color:
/// - Starts with #
/// - Followed by exactly 3 or 6 hex digits
/// - Case insensitive
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

/// Create a text color picker for the toolbar
/// 
/// Shows "A" with color indicator, opens dropdown below
#[component]
pub fn TextColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(default = Signal::derive(|| Vec::new()))]
    recent_colors: Signal<Vec<String>>,
) -> impl IntoView {
    // Create color indicator that shows current color
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
/// 
/// Shows fill icon with color indicator, opens dropdown below  
#[component]
pub fn BackgroundColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(default = Signal::derive(|| Vec::new()))]
    recent_colors: Signal<Vec<String>>,
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
/// 
/// Shows palette icon and text, renders inline
#[component]
pub fn TabColorPicker(
    current_color: Signal<Option<String>>,
    on_change: Callback<Option<String>>,
    #[prop(default = Signal::derive(|| Vec::new()))]
    recent_colors: Signal<Vec<String>>,
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
        assert!(!is_valid_hex_color("000"));        // No #
        assert!(!is_valid_hex_color("#"));          // Just #
        assert!(!is_valid_hex_color("#00"));        // Too short
        assert!(!is_valid_hex_color("#0000"));      // Wrong length
        assert!(!is_valid_hex_color("#00000"));     // Wrong length
        assert!(!is_valid_hex_color("#0000000"));   // Too long
        assert!(!is_valid_hex_color("#xyz"));       // Invalid chars
        assert!(!is_valid_hex_color("#gggggg"));    // Invalid chars
    }
}