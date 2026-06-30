/*!
# Color Picker Component

A reusable color picker for toolbar, context menus, and sheet tabs.

```text
ColorPicker (base - no WorkbookState dep)
├ Dropdown placement -> <button> trigger + <Popover> (click-outside + viewport clamp)
├ Inline placement   -> ctx-item trigger + inline expansion (position derived from parent menu)
└ ColorPickerPanel   (placement-agnostic contents)
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

use crate::components::ui::popover::Popover;
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
    let custom_input = RwSignal::new(String::new());
    let container_class = format!("cp cp-{}", color_type.css_class());

    match placement {
        // Toolbar: a button trigger + a floating panel delegated to `Popover`,
        // which owns click-outside dismiss and viewport-edge clamping. The panel
        // anchors to the trigger button's bottom-left rect so it opens directly
        // beneath the button; Popover then clamps it on-screen.
        ColorPickerPlacement::Dropdown => {
            let (open, set_open) = signal(false);
            let (pos, set_pos) = signal((0i32, 0i32));

            let select_color = move |color: Option<HexColor>| {
                on_color_change.run(color);
                set_open.set(false);
                custom_input.set(String::new());
            };

            let on_trigger = move |ev: web_sys::MouseEvent| {
                use wasm_bindgen::JsCast;
                // current_target = the button the listener is bound to, never an
                // inner icon/bar — so the anchor rect is always the button's.
                if let Some(el) = ev
                    .current_target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                {
                    let rect = el.get_bounding_client_rect();
                    set_pos.set((rect.left() as i32, rect.bottom() as i32));
                }
                set_open.update(|v| *v = !*v);
            };

            view! {
                <div class=container_class>
                    <button
                        class="tb-btn cp-trigger"
                        on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                        on:click=on_trigger
                    >
                        {children()}
                    </button>
                    <Popover open set_open pos class="cp-drop">
                        <ColorPickerPanel
                            current_color=current_color
                            recent_colors=recent_colors
                            custom_input=custom_input
                            allow_custom=allow_custom
                            allow_clear=allow_clear
                            on_color_select=select_color
                        />
                    </Popover>
                </div>
            }
            .into_any()
        }
        // Context menu: the panel expands inline (normal document flow) inside
        // the menu, so its position is derived entirely from the parent menu —
        // no floating panel of its own. The parent `ContextMenu` (itself a
        // `Popover`) re-clamps when this expansion grows it.
        ColorPickerPlacement::Inline => {
            let leptos_use::UseToggleReturn {
                toggle: toggle_picker,
                value: picker_open,
                set_value: set_picker_open,
            } = use_toggle(false);

            let select_color = move |color: Option<HexColor>| {
                on_color_change.run(color);
                set_picker_open.set(false);
                custom_input.set(String::new());
            };

            // Click-outside on the whole row (trigger + expansion), not stopping
            // the event — a mis-click elsewhere closes the picker.
            let container_ref = NodeRef::<leptos::html::Div>::new();
            let _ = on_click_outside(container_ref, move |_| set_picker_open.set(false));

            view! {
                <div class=container_class node_ref=container_ref>
                    <div
                        class="ctx-item cp-trigger"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            toggle_picker();
                        }
                    >
                        {children()}
                    </div>
                    <Show when=move || picker_open.get()>
                        <div class="cp-inline">
                            <ColorPickerPanel
                                current_color=current_color
                                recent_colors=recent_colors
                                custom_input=custom_input
                                allow_custom=allow_custom
                                allow_clear=allow_clear
                                on_color_select=select_color
                            />
                        </div>
                    </Show>
                </div>
            }
            .into_any()
        }
    }
}

// Private sub-components

/// Placement-agnostic panel contents: main palette, recent colors, optional
/// custom-hex input, optional clear button. The wrapping surface (floating
/// `Popover` vs inline `cp-inline` div) is the caller's responsibility.
#[component]
fn ColorPickerPanel(
    current_color: Signal<Option<HexColor>>,
    recent_colors: Signal<Vec<HexColor>>,
    custom_input: RwSignal<String>,
    allow_custom: bool,
    allow_clear: bool,
    on_color_select: impl Fn(Option<HexColor>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
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
    }
}

#[component]
fn MainColorPalette(
    current_color: Signal<Option<HexColor>>,
    on_color_select: impl Fn(Option<HexColor>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div class="cp-palette">
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
            <div class="cp-recent">
                <div class="cp-recent-label">"Recent Colors"</div>
                <div class="cp-recent-grid">
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
                "cp-swatch cp-swatch selected"
            } else {
                "cp-swatch"
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
        <div class="cp-custom">
            <label class="cp-label">"Custom:"</label>
            <input
                type="text"
                class="cp-input"
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
            class="cp-clear"
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
            <div class="cp-bar" style=indicator_style />
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
            <div class="cp-fill">"■"</div>
            <div class="cp-bar" style=indicator_style />
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
