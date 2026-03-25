use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{ModelStore, WorkbookState};

/// Reduced 40-color palette for the tab color picker.
const PALETTE: &[&str] = &[
    "#000000", "#FFFFFF", "#FF0000", "#FF4500", "#FF8C00", "#FFD700", "#00CC44", "#008000",
    "#00BFFF", "#0000FF", "#C00000", "#FF6666", "#FF9966", "#FFCC44", "#AADD44", "#44AA66",
    "#44BBCC", "#4477DD", "#7755BB", "#CC44CC", "#7F0000", "#CC3333", "#CC6633", "#CC9922",
    "#88BB22", "#228844", "#228899", "#224499", "#553388", "#882288", "#400000", "#800000",
    "#804000", "#808000", "#406000", "#004000", "#004040", "#000080", "#400080", "#800040",
];

// ── Main component ───────────────────────────────────────────────────────────

/// Sheet tab bar: `[ + ][ Sheet1 | Sheet2 | Sheet3 ][ 1 hidden ▾ ]`
#[component]
pub fn SheetTabBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // None = not renaming; Some(sheet_idx) = the tab being renamed.
    // Shared across all SheetTab instances via prop.
    let renaming: RwSignal<Option<u32>> = RwSignal::new(None);

    // Visible sheets: returns (sheet_id, sheet_idx) pairs.
    // Keying <For> on (sheet_id, sheet_idx) ensures tabs recreate when
    // indices shift after add/delete. Everything else (name, color,
    // is_selected) is derived reactively inside SheetTab.
    let visible_sheets = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .into_iter()
                .enumerate()
                .filter(|(_, s)| s.state == "visible")
                .map(|(idx, s)| (s.sheet_id, idx as u32))
                .collect::<Vec<_>>()
        })
    };

    let on_add = move |_| {
        model.update_value(|m| {
            m.new_sheet().ok();
        });
        state.request_redraw();
    };

    view! {
        <div style="display:flex;flex-direction:row;align-items:center;min-height:38px;\
            flex-shrink:0;border-top:1px solid var(--border-inner);background:var(--bg-primary);\
            overflow:visible;font-size:12px;position:relative;z-index:400;">

            <button
                style="min-width:32px;min-height:32px;border:none;background:transparent;\
                    cursor:pointer;font-size:18px;color:var(--text-primary);flex-shrink:0;\
                    display:flex;align-items:center;justify-content:center;"
                on:click=on_add
                title="Add sheet"
            >
                "+"
            </button>

            <div style="width:1px;height:60%;background:var(--border-inner);flex-shrink:0;" />

            <div style="display:flex;flex-direction:row;overflow-x:auto;\
                scrollbar-width:none;flex:1;height:100%;align-items:center;\
                padding-left:4px;gap:2px;">
                <For
                    each=visible_sheets
                    key=|(sheet_id, sheet_idx)| (*sheet_id, *sheet_idx)
                    children=move |(_, sheet_idx)| {
                        view! { <SheetTab sheet_idx=sheet_idx renaming=renaming /> }
                    }
                />
            </div>

            <HiddenSheetsMenu />
        </div>
    }
}

// ── Individual sheet tab ─────────────────────────────────────────────────────

/// One tab in the sheet bar. Derives all display state reactively from the
/// model — name, color, is_selected — so the `<For>` key only needs identity.
#[component]
fn SheetTab(sheet_idx: u32, renaming: RwSignal<Option<u32>>) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // ── Reactive derived state ───────────────────────────────────────────────
    // These closures subscribe to `state.redraw` and re-evaluate when the
    // model changes. No captured values go stale.

    let is_selected = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.get_selected_view().sheet == sheet_idx)
    };

    let name = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .get(sheet_idx as usize)
                .map(|s| s.name.clone())
                .unwrap_or_default()
        })
    };

    let tab_color = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .get(sheet_idx as usize)
                .and_then(|s| s.color.clone())
        })
    };

    // ── Event handlers ───────────────────────────────────────────────────────

    let on_click = move |_: web_sys::MouseEvent| {
        model.update_value(|m| {
            m.set_selected_sheet(sheet_idx).ok();
        });
        state.request_redraw();
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        ev.prevent_default();
        renaming.set(Some(sheet_idx));
    };

    let tab_style = move || {
        if is_selected() {
            "display:flex;align-items:center;height:38px;padding:0 8px 0 12px;\
             border:1px solid var(--border-inner);border-bottom:2px solid var(--accent);\
             background:var(--tab-active-bg);border-radius:4px 4px 0 0;cursor:pointer;\
             font-weight:600;color:var(--accent);white-space:nowrap;font-size:12px;gap:4px;"
        } else {
            "display:flex;align-items:center;height:38px;padding:0 12px;\
             border:1px solid transparent;border-bottom:none;\
             background:var(--bg-primary);border-radius:4px 4px 0 0;cursor:pointer;\
             font-weight:400;color:var(--text-primary);white-space:nowrap;font-size:12px;gap:4px;"
        }
    };

    view! {
        <div style=tab_style on:click=on_click on:dblclick=on_dblclick>

            // Tab name — inline rename input when double-clicked
            <Show
                when=move || renaming.get() == Some(sheet_idx)
                fallback=move || view! { <span>{name}</span> }
            >
                <RenameInput sheet_idx=sheet_idx renaming=renaming />
            </Show>

            // Color swatch dot
            <TabColorSwatch sheet_idx=sheet_idx tab_color=tab_color />

            // Active-tab actions: hide + delete
            <Show when=move || is_selected()>
                <TabActions sheet_idx=sheet_idx />
            </Show>
        </div>
    }
}

// ── Rename input ─────────────────────────────────────────────────────────────

/// Inline text input that replaces the tab name during rename.
#[component]
fn RenameInput(sheet_idx: u32, renaming: RwSignal<Option<u32>>) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let input_ref = NodeRef::<leptos::html::Input>::new();

    // Read current name for the initial value.
    let initial_name = model.with_value(|m| {
        m.get_worksheets_properties()
            .get(sheet_idx as usize)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    });

    // Focus + select-all after mount.
    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            // spawn_local defers to a microtask — prevents the dblclick
            // that triggered rename from stealing focus back.
            let el2 = el.clone();
            wasm_bindgen_futures::spawn_local(async move {
                el2.focus().ok();
                el2.select();
            });
        }
    });

    let commit_rename = move |new_name: String| {
        if !new_name.trim().is_empty() {
            model.update_value(|m| {
                m.rename_sheet(sheet_idx, &new_name).ok();
            });
            state.request_redraw();
        }
        renaming.set(None);
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        ev.stop_propagation();
        let key = ev.key();
        if key == "Enter" {
            ev.prevent_default();
            let new_name = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                .map(|i| i.value())
                .unwrap_or_default();
            commit_rename(new_name);
        } else if key == "Escape" {
            ev.prevent_default();
            renaming.set(None);
        }
    };

    let on_blur = move |ev: web_sys::FocusEvent| {
        // Guard: if Enter already committed, removing the input triggers
        // a spurious blur — skip it.
        if renaming.get_untracked() != Some(sheet_idx) {
            return;
        }
        let new_name = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|i| i.value())
            .unwrap_or_default();
        commit_rename(new_name);
    };

    view! {
        <input
            node_ref=input_ref
            type="text"
            style="width:80px;font-size:12px;border:none;outline:none;\
                   background:transparent;color:inherit;"
            prop:value=initial_name
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:mousedown=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:keydown=on_keydown
            on:blur=on_blur
        />
    }
}

// ── Color swatch + picker ────────────────────────────────────────────────────

/// Colored dot showing the tab color. Clicking opens an inline palette popup.
#[component]
fn TabColorSwatch(
    sheet_idx: u32,
    /// Reactive closure returning the current tab color.
    tab_color: impl Fn() -> Option<String> + Send + Sync + 'static,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let color_open: RwSignal<bool> = RwSignal::new(false);
    let picker_left: RwSignal<i32> = RwSignal::new(0);

    let dot_style = move || {
        let bg = tab_color().unwrap_or_else(|| "transparent".to_owned());
        if bg != "transparent" {
            format!(
                "width:22px;height:22px;border-radius:50%;background:{bg};\
                 border:1px solid rgba(0,0,0,0.25);cursor:pointer;flex-shrink:0;"
            )
        } else {
            "width:22px;height:22px;border-radius:50%;background:transparent;\
             border:1px solid rgba(128,128,128,0.4);cursor:pointer;flex-shrink:0;"
                .to_owned()
        }
    };

    let on_dot_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        picker_left.set(ev.client_x());
        color_open.update(|v| *v = !*v);
    };

    let set_color = move |hex: &str| {
        model.update_value(|m| {
            m.set_sheet_color(sheet_idx, hex).ok();
        });
        state.request_redraw();
        color_open.set(false);
    };

    view! {
        <div style="position:relative;display:inline-flex;align-items:center;">
            <span title="Set tab color" style=dot_style on:click=on_dot_click />

            <Show when=move || color_open.get()>
                // Click-away backdrop
                <div
                    style="position:fixed;inset:0;z-index:1099;"
                    on:click=move |_| color_open.set(false)
                />
                // Palette popup
                <div style=move || format!(
                    "position:fixed;bottom:36px;left:{}px;z-index:1100;\
                     background:var(--bg-primary);border:1px solid var(--border-color);\
                     border-radius:4px;box-shadow:var(--shadow-sm, 0 2px 8px rgba(0,0,0,0.15));\
                     padding:5px;display:flex;flex-direction:column;gap:3px;",
                    picker_left.get()
                )>
                    <div style="display:grid;grid-template-columns:repeat(10,28px);gap:2px;">
                        {PALETTE.iter().map(|&hex| {
                            view! {
                                <div
                                    title=hex
                                    style=format!(
                                        "width:28px;height:28px;background:{hex};cursor:pointer;\
                                         border-radius:2px;border:1px solid rgba(0,0,0,0.2);"
                                    )
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        ev.stop_propagation();
                                        set_color(hex);
                                    }
                                />
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                    <button
                        style="font-size:11px;padding:2px 4px;cursor:pointer;\
                               border:1px solid var(--border-color);border-radius:3px;\
                               background:var(--btn-bg);color:var(--text-primary);text-align:left;"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            set_color("");
                        }
                    >
                        "No color"
                    </button>
                </div>
            </Show>
        </div>
    }
}

// ── Tab action buttons (hide / delete) ───────────────────────────────────────

/// Hide and delete buttons shown on the active tab.
///
/// Derives name and visible count from the model (via context) rather than
/// taking prop closures — keeps all captures `Copy` so the closures work
/// inside `<Show>` (which requires `Fn`, not `FnOnce`).
#[component]
fn TabActions(sheet_idx: u32) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let visible_count = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .iter()
                .filter(|s| s.state == "visible")
                .count()
        })
    };

    let on_hide = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let ok = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message(
                    "Hide this sheet? You can unhide it from the \
                     hidden-sheets menu at the right side of the tab bar.",
                )
                .ok()
            })
            .unwrap_or(false);
        if ok {
            model.update_value(|m| {
                m.hide_sheet(sheet_idx).ok();
            });
            state.request_redraw();
        }
    };

    let on_delete = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let sheet_name = model.with_value(|m| {
            m.get_worksheets_properties()
                .get(sheet_idx as usize)
                .map(|s| s.name.clone())
                .unwrap_or_default()
        });
        let confirmed = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message(&format!("Delete '{sheet_name}'? This cannot be undone."))
                    .ok()
            })
            .unwrap_or(false);
        if confirmed {
            model.update_value(|m| {
                m.delete_sheet(sheet_idx).ok();
            });
            state.request_redraw();
        }
    };

    view! {
        <button
            style="border:none;background:transparent;cursor:pointer;\
                   color:var(--text-dim);font-size:12px;padding:0 2px;line-height:1;"
            title="Hide sheet"
            on:click=on_hide
        >
            "\u{22EE}"
        </button>

        <Show when=move || { visible_count() > 1 }>
            <button
                style="border:none;background:transparent;cursor:pointer;\
                       color:var(--text-dim);font-size:11px;padding:0 2px;line-height:1;"
                title="Delete sheet"
                on:click=on_delete
            >
                "\u{00D7}"
            </button>
        </Show>
    }
}

// ── Hidden sheets dropdown ───────────────────────────────────────────────────

/// Dropdown listing hidden sheets with click-to-unhide. Only renders when
/// at least one sheet is hidden.
#[component]
fn HiddenSheetsMenu() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let hidden_sheets = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .into_iter()
                .enumerate()
                .filter(|(_, s)| s.state != "visible")
                .map(|(idx, s)| (idx as u32, s.name.clone()))
                .collect::<Vec<_>>()
        })
    };

    // Only render the <details> when there are hidden sheets.
    move || {
        let hidden = hidden_sheets();
        if hidden.is_empty() {
            return None;
        }
        let count = hidden.len();
        Some(view! {
            <details style="position:relative;flex-shrink:0;margin-left:4px;">
                <summary style="list-style:none;cursor:pointer;padding:0 8px;\
                                font-size:11px;color:var(--text-dim);user-select:none;">
                    {format!("{count} hidden")}
                </summary>
                <div style="position:absolute;bottom:100%;left:0;z-index:200;\
                            background:var(--bg-primary);border:1px solid var(--border-color);\
                            border-radius:4px;padding:4px 0;min-width:140px;\
                            box-shadow:var(--shadow-sm, 0 2px 8px rgba(0,0,0,0.15));">
                    {hidden.into_iter().map(|(idx, sheet_name)| {
                        view! {
                            <div
                                style="padding:6px 12px;cursor:pointer;font-size:12px;\
                                       color:var(--text-primary);"
                                on:click=move |_| {
                                    model.update_value(|m| {
                                        m.unhide_sheet(idx).ok();
                                    });
                                    state.request_redraw();
                                }
                            >
                                {sheet_name}
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </details>
        })
    }
}
