/*!
# Color Picker Component

A reusable color picker for toolbar, context menus, and sheet tabs.

```
ColorPicker (base - no WorkbookState dep)
├ ColorPickerTrigger  (button or ctx-item depending on placement)
├ on_click_outside    (closes picker without swallowing the click)
└ ColorPickerDropdown (z-index 1100)
    ├ MainColorPalette
    ├ RecentColorsPalette
    ├ CustomColorInput
    └ ClearColorButton
```

## Usage

Toolbar (WorkbookState-aware convenience wrappers):
```rust
<TextColorPicker       current_color=sig on_change=cb />
<BackgroundColorPicker current_color=sig on_change=cb />
```

Context menu / tab bar:
```rust
<TabColorPicker current_color=sig on_change=cb />
```

Custom / without WorkbookState:
```rust
<ColorPicker color_type=ColorType::Text ... recent_colors=my_sig>
    // trigger content
</ColorPicker>
```
*/

use leptos::prelude::*;
use leptos_use::{on_click_outside, use_toggle};

use crate::model::style_types::HexColor;
use crate::state::WorkbookState;
use crate::theme::COLOR_PALETTE;

//  Public types
/// Which color role a picker is editing.
///
/// Used to build the container's CSS modifier class (e.g. `color-picker-text`)
/// and to distinguish pickers when multiple appear on the same toolbar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorType {
    Text,
    Background,
    Tab,
    #[allow(dead_code)]
    Generic,
}

impl ColorType {
    /// CSS modifier suffix appended to `color-picker-` for the container class.
    pub fn css_class(self) -> &'static str {
        match self {
            ColorType::Text => "text",
            ColorType::Background => "background",
            ColorType::Tab => "tab",
            ColorType::Generic => "generic",
        }
    }
}

/// Whether the picker opens as a toolbar dropdown or an inline context-menu item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorPickerPlacement {
    Dropdown,
    Inline,
}

// Base component

/// Generic color picker - no WorkbookState dependency.
///
/// `on_color_change` is called with `Some(hex)` on selection or `None` on clear.
/// Adding the color to recent-colors history is the caller's responsibility.
///
/// For toolbar/tab use, prefer the context-aware wrappers
/// [`TextColorPicker`], [`BackgroundColorPicker`], or [`TabColorPicker`].
#[component]
pub fn ColorPicker(
    color_type: ColorType,
    current_color: Signal<Option<HexColor>>,
    on_color_change: Callback<Option<HexColor>>,
    #[prop(default = ColorPickerPlacement::Dropdown)] placement: ColorPickerPlacement,
    children: Children,
    #[prop(default = true)] allow_custom: bool,
    #[prop(default = true)] allow_clear: bool,
    #[prop(default = Signal::derive(|| Vec::new()))] recent_colors: Signal<Vec<HexColor>>,
) -> impl IntoView {
    let leptos_use::UseToggleReturn {
        toggle: toggle_picker,
        value: picker_open,
        set_value: set_picker_open,
    } = use_toggle(false);

    let custom_input = RwSignal::new(String::new());

    let select_color = move |color: Option<HexColor>| {
        on_color_change.run(color);
        set_picker_open.set(false);
        custom_input.set(String::new());
    };

    // on_click_outside fires for any click whose target is outside this div,
    // without consuming/stopping the event - so a mis-click on Bold closes the
    // picker AND toggles bold in a single click.
    let container_ref = NodeRef::<leptos::html::Div>::new();
    let _ = on_click_outside(container_ref, move |_| set_picker_open.set(false));

    let container_class = format!("color-picker color-picker-{}", color_type.css_class());

    view! {
        <div class={container_class} node_ref=container_ref>
            <ColorPickerTrigger
                placement=placement
                on_toggle=Callback::new(move |_| toggle_picker())
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

// Private sub-components

#[component]
fn ColorPickerTrigger(
    placement: ColorPickerPlacement,
    on_toggle: Callback<()>,
    children: Children,
) -> impl IntoView {
    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        on_toggle.run(());
    };

    if placement == ColorPickerPlacement::Dropdown {
        view! {
            <button class="toolbar-btn color-picker-trigger" on:click=on_click>
                {children()}
            </button>
        }
        .into_any()
    } else {
        view! {
            <div class="ctx-item color-picker-trigger" on:click=on_click>
                {children()}
            </div>
        }
        .into_any()
    }
}

#[component]
fn ColorPickerDropdown(
    placement: ColorPickerPlacement,
    current_color: Signal<Option<HexColor>>,
    recent_colors: Signal<Vec<HexColor>>,
    custom_input: RwSignal<String>,
    allow_custom: bool,
    allow_clear: bool,
    on_color_select: impl Fn(Option<HexColor>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let picker_class = match placement {
        ColorPickerPlacement::Dropdown => "color-picker-dropdown",
        ColorPickerPlacement::Inline => "color-picker-inline",
    };

    view! {
        <div class={picker_class}>
            <MainColorPalette current_color=current_color on_color_select=on_color_select />
            <RecentColorsPalette
                recent_colors=recent_colors
                current_color=current_color
                on_color_select=on_color_select
            />
            <Show when=move || allow_custom>
                <CustomColorInput custom_input=custom_input on_color_select=on_color_select />
            </Show>
            <Show when=move || allow_clear>
                <ClearColorButton on_color_select=on_color_select />
            </Show>
        </div>
    }
}

#[component]
fn MainColorPalette(
    current_color: Signal<Option<HexColor>>,
    on_color_select: impl Fn(Option<HexColor>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div class="color-picker-palette">
            {COLOR_PALETTE
                .iter()
                .filter_map(|&hex_str| HexColor::new(hex_str).ok())
                .map(|swatch| {
                    let swatch_cmp = swatch.clone();
                    view! {
                        <ColorSwatch
                            hex=swatch
                            is_selected=move || {
                                current_color
                                    .get()
                                    .map(|c| c == swatch_cmp)
                                    .unwrap_or(false)
                            }
                            on_click=Callback::new(move |h: HexColor| on_color_select(Some(h)))
                        />
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

#[component]
fn RecentColorsPalette(
    recent_colors: Signal<Vec<HexColor>>,
    current_color: Signal<Option<HexColor>>,
    on_color_select: impl Fn(Option<HexColor>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <Show when=move || !recent_colors.get().is_empty()>
            <div class="color-picker-recent-section">
                <div class="color-picker-recent-label">"Recent Colors"</div>
                <div class="color-picker-recent-palette">
                    <For
                        each=move || recent_colors.get()
                        key=|hex: &HexColor| hex.as_str().to_string()
                        children=move |hex| {
                            let h = hex.clone();
                            view! {
                                <ColorSwatch
                                    hex=hex
                                    is_selected=move || {
                                        current_color
                                            .get()
                                            .map(|c| c == h)
                                            .unwrap_or(false)
                                    }
                                    on_click=Callback::new(move |h: HexColor| {
                                        on_color_select(Some(h))
                                    })
                                />
                            }
                        }
                    />
                </div>
            </div>
        </Show>
    }
}

/// Individual color swatch.
///
/// `is_selected` is a reactive closure so the selected ring updates when
/// `current_color` changes without re-rendering the whole palette.
/// `on_click` receives the swatch's hex string - the component clones it
/// internally on click, so callers never need to capture hex separately.
#[component]
fn ColorSwatch(
    hex: HexColor,
    is_selected: impl Fn() -> bool + Send + Sync + 'static,
    on_click: Callback<HexColor>,
) -> impl IntoView {
    let style = format!("background-color: {};", hex.as_str());
    let title = hex.as_str().to_string();
    view! {
        <div
            class=move || if is_selected() {
                "color-picker-swatch color-picker-swatch--selected"
            } else {
                "color-picker-swatch"
            }
            style=style
            title=title
            on:click=move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                on_click.run(hex.clone());
            }
        />
    }
}

#[component]
fn CustomColorInput(
    custom_input: RwSignal<String>,
    on_color_select: impl Fn(Option<HexColor>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let submit = move |raw: String| {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            on_color_select(None);
            return;
        }
        let normalized = if trimmed.starts_with('#') {
            trimmed
        } else {
            format!("#{trimmed}")
        };
        if let Ok(color) = HexColor::new(&normalized) {
            on_color_select(Some(color));
        }
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            submit(custom_input.get());
        }
        "Escape" => {
            ev.prevent_default();
            custom_input.set(String::new());
        }
        _ => {}
    };

    let on_blur = move |_: web_sys::FocusEvent| {
        let v = custom_input.get();
        if !v.trim().is_empty() {
            submit(v);
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
                on:input=move |ev| custom_input.set(event_target_value(&ev))
                on:keydown=on_keydown
                on:blur=on_blur
            />
        </div>
    }
}

#[component]
fn ClearColorButton(
    on_color_select: impl Fn(Option<HexColor>) + Send + Sync + 'static,
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

// Context-aware wrappers
// These pull recent_colors reactively from WorkbookState so callers don't have
// to wire it up manually. Adding colors to history remains the caller's job
// (done in the on_change callback at the toolbar / tab-bar level).

fn workbook_recent_colors(state: WorkbookState) -> Signal<Vec<HexColor>> {
    // recent_colors is a split signal; reading it here makes this derived signal
    // reactive - it re-runs whenever add_recent_color() writes the signal.
    Signal::derive(move || {
        state
            .recent_colors
            .get()
            .into_iter()
            .filter_map(|c| HexColor::new(c.as_str()).ok())
            .collect()
    })
}

fn color_indicator_style(current_color: Signal<Option<HexColor>>) -> impl Fn() -> String {
    move || match current_color.get() {
        Some(c) => format!("background-color: {};", c.as_str()),
        None => "background-color: transparent; border: 1px solid var(--border-color);".to_string(),
    }
}

/// Toolbar text-color picker. Pulls `recent_colors` from [`WorkbookState`] automatically.
#[component]
pub fn TextColorPicker(
    current_color: Signal<Option<HexColor>>,
    on_change: Callback<Option<HexColor>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let recent_colors = workbook_recent_colors(state);
    let indicator_style = color_indicator_style(current_color);

    view! {
        <ColorPicker
            color_type=ColorType::Text
            current_color=current_color
            on_color_change=on_change
            recent_colors=recent_colors
        >
            <div class="color-indicator" style=indicator_style />
            "A"
        </ColorPicker>
    }
}

/// Toolbar background-fill picker. Pulls `recent_colors` from [`WorkbookState`] automatically.
#[component]
pub fn BackgroundColorPicker(
    current_color: Signal<Option<HexColor>>,
    on_change: Callback<Option<HexColor>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let recent_colors = workbook_recent_colors(state);
    let indicator_style = color_indicator_style(current_color);

    view! {
        <ColorPicker
            color_type=ColorType::Background
            current_color=current_color
            on_color_change=on_change
            recent_colors=recent_colors
        >
            <div class="fill-icon">"■"</div>
            <div class="color-indicator" style=indicator_style />
        </ColorPicker>
    }
}

/// Sheet-tab color picker, rendered as a context-menu item. Pulls `recent_colors` from [`WorkbookState`].
#[component]
pub fn TabColorPicker(
    current_color: Signal<Option<HexColor>>,
    on_change: Callback<Option<HexColor>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let recent_colors = workbook_recent_colors(state);

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
    use crate::model::style_types::is_valid_hex_color;

    #[test]
    fn test_hex_color_validation() {
        // Testing the unified validation function from style_types
        assert!(is_valid_hex_color("#000"));
        assert!(is_valid_hex_color("#000000"));
        assert!(is_valid_hex_color("#ABC"));
        assert!(is_valid_hex_color("#abcdef"));
        assert!(is_valid_hex_color("#123456"));
        assert!(!is_valid_hex_color("000"));
        assert!(!is_valid_hex_color("#"));
        assert!(!is_valid_hex_color("#00"));
        assert!(!is_valid_hex_color("#0000"));
        assert!(!is_valid_hex_color("#00000"));
        assert!(!is_valid_hex_color("#0000000"));
        assert!(!is_valid_hex_color("#xyz"));
        assert!(!is_valid_hex_color("#gggggg"));
    }
}
