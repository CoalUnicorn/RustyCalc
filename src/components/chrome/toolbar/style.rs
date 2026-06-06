//! Border picker for the toolbar.
//!
//! A "▦ ▾" trigger opens a [`Popover`] laid out as a compact 5-column grid of
//! the ten border presets, followed by two collapsible menu rows:
//! - **Border color** — option B: the row *is* the reusable base [`ColorPicker`]
//!   trigger, so border colors flow through the same palette/recent-colors UI as
//!   text/fill.
//! - **Border style** — expands to a Thin/Medium/Thick weight selector.
//!
//! Color and weight persist across preset clicks within one open session, and
//! seed from the active cell when the dropdown opens (see
//! `SheetQuery::toolbar_state`).

use leptos::prelude::*;

use crate::components::ui::color_picker::{ColorPicker, ColorPickerPlacement, ColorType};
use crate::components::ui::popover::Popover;
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::model::frontend_types::ToolbarState;
use crate::model::style_types::{BorderSide, BorderWeight, HexColor};
use crate::state::{ModelStore, WorkbookState};
use crate::util::refocus_workbook;

use super::icon::{BorderIcon, Icon};

/// Preset descriptor: `(label, glyph, side)`. Weight and color come from the
/// shared selector signals, so they are not baked into the preset.
type Preset = (&'static str, BorderIcon, BorderSide);

/// All ten presets, row-major for the 5-column grid: region fills on the top
/// row, single edges + clear on the bottom row.
const PRESETS: &[Preset] = &[
    ("All Borders", BorderIcon::All, BorderSide::All),
    ("Outside Borders", BorderIcon::Outer, BorderSide::Outer),
    ("Inside Borders", BorderIcon::Inner, BorderSide::Inner),
    (
        "Inside Horizontal",
        BorderIcon::CenterH,
        BorderSide::CenterH,
    ),
    ("Inside Vertical", BorderIcon::CenterV, BorderSide::CenterV),
    ("Top Border", BorderIcon::Top, BorderSide::Top),
    ("Bottom Border", BorderIcon::Bottom, BorderSide::Bottom),
    ("Left Border", BorderIcon::Left, BorderSide::Left),
    ("Right Border", BorderIcon::Right, BorderSide::Right),
    ("No Border", BorderIcon::None, BorderSide::None),
];

/// Toolbar border-preset picker.
#[component]
pub fn BorderPicker() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let toolbar_state = expect_context::<Memo<ToolbarState>>();

    let (open, set_open) = signal(false);
    let (pos, set_pos) = signal((0i32, 0i32));

    // Session state shared by the color row, weight selector, and presets.
    let border_color =
        RwSignal::new(HexColor::new("#000000").unwrap_or_else(|_| HexColor::transparent()));
    let border_weight = RwSignal::new(BorderWeight::Thin);

    let trigger_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        // Seed the picker from the active cell's dominant border before opening
        // so it reflects what's already applied to the selection.
        if !open.get_untracked() {
            let b = toolbar_state.get_untracked().border;
            border_color.set(HexColor::from_opt(Some(b.color.as_str().to_owned())));
            border_weight.set(b.weight);
        }
        set_pos.set((ev.client_x(), ev.client_y()));
        set_open.update(|v| *v = !*v);
    };

    let apply = move |side: BorderSide| {
        let hex = border_color.get_untracked();
        let weight = border_weight.get_untracked();
        execute(
            &SpreadsheetAction::set_border(side, weight, hex),
            model,
            &state,
        );
        set_open.set(false);
        refocus_workbook();
    };

    view! {
        <div class="tb-border">
            <button
                class="tb-btn"
                title="Borders"
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=trigger_click
            >
                <Icon icon=BorderIcon::All />
                " ▾"
            </button>
            <Popover open set_open pos class="tb-border-dropdown">
                <div class="tb-border-grid">
                    {PRESETS
                        .iter()
                        .map(|&(name, glyph, side)| {
                            view! {
                                <button
                                    class="tb-border-item"
                                    title=name
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        ev.stop_propagation();
                                        apply(side);
                                    }
                                >
                                    <Icon icon=glyph />
                                </button>
                            }
                        })
                        .collect::<Vec<_>>()}
                </div>
                <div class="tb-border-sep" />
                <BorderColorRow border_color=border_color />
                <BorderStyleRow border_weight=border_weight />
            </Popover>
        </div>
    }
}

/// "Border color" menu row — the reusable base [`ColorPicker`] (option B), whose
/// trigger renders as a full-width row: pencil icon, label, current-color swatch.
/// Clicking the row opens the color panel.
#[component]
fn BorderColorRow(border_color: RwSignal<HexColor>) -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Recent colors, sourced reactively from `WorkbookState` like the text/fill
    // pickers do, so the border palette shares the same history.
    let recent_colors = Signal::derive(move || {
        state
            .recent_colors
            .get()
            .into_iter()
            .filter_map(|c| HexColor::new(c.as_str()).ok())
            .collect::<Vec<_>>()
    });

    let current_color = Signal::derive(move || Some(border_color.get()));

    let on_color_change = Callback::new(move |color: Option<HexColor>| {
        if let Some(hex) = color {
            state.add_recent_color(hex.as_str());
            border_color.set(hex);
        }
        // Keep the border dropdown open — the user still needs to pick a preset.
    });

    view! {
        <ColorPicker
            color_type=ColorType::Generic
            current_color=current_color
            on_color_change=on_color_change
            placement=ColorPickerPlacement::Dropdown
            recent_colors=recent_colors
            allow_clear=false
        >
            <span class="tb-border-menu-ic" inner_html=PENCIL_SVG />
            <span class="tb-border-menu-label">"Border color"</span>
            <span
                class="tb-border-swatch"
                style=move || format!("background-color:{};", border_color.get().as_str())
            />
        </ColorPicker>
    }
}

/// "Border style" section — a label plus the three weight options shown directly,
/// each a button whose line-weight preview sits beneath its name.
#[component]
fn BorderStyleRow(border_weight: RwSignal<BorderWeight>) -> impl IntoView {
    let weights = [
        (BorderWeight::Thin, "Thin", 1),
        (BorderWeight::Medium, "Medium", 2),
        (BorderWeight::Thick, "Thick", 3),
    ];

    view! {
        <div class="tb-border-menu-label-row">
            <span class="tb-border-menu-ic" inner_html=DASHED_SVG />
            <span class="tb-border-menu-label">"Border style"</span>
        </div>
        <div class="tb-border-weight-row">
            {weights
                .into_iter()
                .map(|(w, label, px)| {
                    let preview_style = format!("border-top:{px}px solid currentColor;");
                    view! {
                        <button
                            class=move || {
                                if border_weight.get() == w {
                                    "tb-border-weight-btn tb-border-weight-btn--active"
                                } else {
                                    "tb-border-weight-btn"
                                }
                            }
                            on:click=move |ev: web_sys::MouseEvent| {
                                ev.stop_propagation();
                                border_weight.set(w);
                            }
                        >
                            <span class="tb-border-weight-label">{label}</span>
                            <span class="tb-border-weight-preview" style=preview_style />
                        </button>
                    }
                })
                .collect::<Vec<_>>()}
        </div>
    }
}

// Inline glyphs for the two menu rows (not border presets, so not in BorderIcon).
const PENCIL_SVG: &str = r#"<svg viewBox="0 0 24 24" fill="currentColor"><path d="M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a.996.996 0 0 0 0-1.41l-2.34-2.34a.996.996 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"/></svg>"#;
const DASHED_SVG: &str = r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="3" y1="12" x2="7" y2="12"/><line x1="11" y1="12" x2="15" y2="12"/><line x1="19" y1="12" x2="21" y2="12"/></svg>"#;
