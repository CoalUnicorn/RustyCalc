use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{ModelStore, WorkbookState};
use crate::theme::COLOR_PALETTE;

// ── Main component ───────────────────────────────────────────────────────────

/// Sheet tab bar: `[ + ][ ≡ ][ Sheet1 ▾ | **Sheet2 ▾** | Sheet3 ▾ ]`
#[component]
pub fn SheetTabBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Tracks which tab's context menu is open (by sheet_idx), or None.
    let menu_open: RwSignal<Option<u32>> = RwSignal::new(None);
    // Fixed position for the context menu (set on menu click).
    let menu_pos: RwSignal<(i32, i32)> = RwSignal::new((0, 0));
    // Tracks which tab is being renamed, or None.
    let renaming: RwSignal<Option<u32>> = RwSignal::new(None);

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
        <div class="tab-bar">
            <button class="tab-bar-add" on:click=on_add title="Add sheet">"+"</button>
            <AllSheetsMenu />
            <div class="tab-bar-divider" />
            <div class="tab-bar-scroll">
                <For
                    each=visible_sheets
                    key=|(sheet_id, sheet_idx)| (*sheet_id, *sheet_idx)
                    children=move |(_, sheet_idx)| {
                        view! {
                            <SheetTab
                                sheet_idx=sheet_idx
                                menu_open=menu_open
                                menu_pos=menu_pos
                                renaming=renaming
                            />
                        }
                    }
                />
            </div>
        </div>
    }
}

// ── Individual sheet tab ─────────────────────────────────────────────────────

#[component]
fn SheetTab(
    sheet_idx: u32,
    menu_open: RwSignal<Option<u32>>,
    menu_pos: RwSignal<(i32, i32)>,
    renaming: RwSignal<Option<u32>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

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

    let on_click = move |_: web_sys::MouseEvent| {
        model.update_value(|m| {
            m.set_selected_sheet(sheet_idx).ok();
        });
        state.request_redraw();
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        ev.prevent_default();
        menu_open.set(None);
        renaming.set(Some(sheet_idx));
    };

    let menu = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        if menu_open.get_untracked() == Some(sheet_idx) {
            menu_open.set(None);
        } else {
            // Position the fixed menu at the menu's screen location.
            menu_pos.set((ev.client_x(), ev.client_y()));
            menu_open.set(Some(sheet_idx));
        }
    };

    let tab_class = move || {
        if is_selected() {
            "sheet-tab selected"
        } else {
            "sheet-tab"
        }
    };

    // Color accent bar at the bottom of the tab (if a tab color is set).
    let color_bar_style = move || {
        tab_color()
            .map(|c| format!("background:{c};"))
            .unwrap_or_default()
    };

    view! {
        <div class=tab_class on:click=on_click on:dblclick=on_dblclick>
            <Show
                when=move || renaming.get() == Some(sheet_idx)
                fallback=move || view! { <span class="tab-name">{name}</span> }
            >
                <RenameInput sheet_idx=sheet_idx renaming=renaming />
            </Show>
            <span class="sheet-tab-menu" on:click=menu>"≓"</span>
            <div class="tab-color-bar" style=color_bar_style />

            <Show when=move || menu_open.get() == Some(sheet_idx)>
                <TabContextMenu sheet_idx=sheet_idx menu_open=menu_open menu_pos=menu_pos renaming=renaming />
            </Show>
        </div>
    }
}

// ── Context menu ─────────────────────────────────────────────────────────────

#[component]
fn TabContextMenu(
    sheet_idx: u32,
    menu_open: RwSignal<Option<u32>>,
    menu_pos: RwSignal<(i32, i32)>,
    renaming: RwSignal<Option<u32>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let color_sub_open: RwSignal<bool> = RwSignal::new(false);

    let visible_count = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .iter()
                .filter(|s| s.state == "visible")
                .count()
        })
    };

    let on_rename = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        menu_open.set(None);
        renaming.set(Some(sheet_idx));
    };

    let on_color_toggle = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        color_sub_open.update(|v| *v = !*v);
    };

    let set_color = move |hex: &str| {
        model.update_value(|m| {
            m.set_sheet_color(sheet_idx, hex).ok();
        });
        state.request_redraw();
        menu_open.set(None);
    };

    let on_hide = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        menu_open.set(None);
        model.update_value(|m| {
            m.hide_sheet(sheet_idx).ok();
        });
        state.request_redraw();
    };

    let on_delete = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        menu_open.set(None);
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
        // Click-away backdrop
        <div class="click-away-backdrop" on:click=move |_| menu_open.set(None) />

        <div
            class="tab-context-menu"
            style=move || {
                let (x, y) = menu_pos.get();
                format!("left:{x}px;bottom:calc(100vh - {y}px + 4px);")
            }
        >
            <div class="ctx-item" on:click=on_rename>
                <span class="ctx-icon">"✏"</span> "Rename"
            </div>
            <div class="ctx-item" on:click=on_color_toggle>
                <span class="ctx-icon">"🎨"</span> "Change Color"
            </div>
            // TODO: extract to component
            <Show when=move || color_sub_open.get()>
                <div class="color-picker-inline">
                    <div class="color-picker-grid">
                        {COLOR_PALETTE.iter().map(|&hex| {
                            view! {
                                <div
                                    title=hex
                                    class="color-picker-swatch"
                                    style=format!("background:{hex};")
                                    on:click=move |ev: web_sys::MouseEvent| {
                                        ev.stop_propagation();
                                        set_color(hex);
                                    }
                                />
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                    <button
                        class="color-picker-clear"
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            set_color("");
                        }
                    >
                        "No color"
                    </button>
                </div>
            </Show>
            <Show when=move || { visible_count() > 1 }>
                <div class="ctx-item" on:click=on_hide>
                    <span class="ctx-icon">"👁"</span> "Hide sheet"
                </div>
            </Show>
            <div class="ctx-divider" />
            <Show when=move || { visible_count() > 1 }>
                <div class="ctx-item delete" on:click=on_delete>
                    <span class="ctx-icon">"🗑"</span> "Delete"
                </div>
            </Show>
        </div>
    }
}

// ── Rename input ─────────────────────────────────────────────────────────────

#[component]
fn RenameInput(sheet_idx: u32, renaming: RwSignal<Option<u32>>) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let input_ref = NodeRef::<leptos::html::Input>::new();

    let initial_name = model.with_value(|m| {
        m.get_worksheets_properties()
            .get(sheet_idx as usize)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    });

    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
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
            class="tab-rename-input"
            prop:value=initial_name
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:mousedown=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:keydown=on_keydown
            on:blur=on_blur
        />
    }
}

// ── All sheets menu (hamburger) ──────────────────────────────────────────────

/// `≡` button that lists all sheets (visible + hidden) for quick navigation.
#[component]
fn AllSheetsMenu() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let open: RwSignal<bool> = RwSignal::new(false);
    let btn_ref = NodeRef::<leptos::html::Button>::new();
    let menu_pos: RwSignal<(i32, i32)> = RwSignal::new((0, 0));

    let all_sheets = move || {
        let _ = state.redraw.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .into_iter()
                .enumerate()
                .map(|(idx, s)| (idx as u32, s.name.clone(), s.state.clone()))
                .collect::<Vec<_>>()
        })
    };

    let selected_sheet = move || {
        let _ = state.redraw.get();
        model.with_value(|m| m.get_selected_view().sheet)
    };

    let on_toggle = move |_: web_sys::MouseEvent| {
        if let Some(el) = btn_ref.get() {
            let rect = el.get_bounding_client_rect();
            menu_pos.set((rect.left() as i32, rect.top() as i32));
        }
        open.update(|v| *v = !*v);
    };

    view! {
        <div>
            <button
                node_ref=btn_ref
                class="tab-bar-hamburger"
                title="All sheets"
                on:click=on_toggle
            >
                "≡"
            </button>
            <Show when=move || open.get()>
                <div class="click-away-backdrop" on:click=move |_| open.set(false) />
                <div
                    class="all-sheets-menu"
                    style=move || {
                        let (x, y) = menu_pos.get();
                        format!("left:{x}px;bottom:calc(100vh - {y}px + 4px);")
                    }
                >
                    {move || {
                        let sheets = all_sheets();
                        let selected = selected_sheet();
                        sheets.into_iter().map(|(idx, name, sheet_state)| {
                            let is_hidden = sheet_state != "visible";
                            let is_active = idx == selected;
                            let item_class = if is_active {
                                "all-sheets-item active"
                            } else if is_hidden {
                                "all-sheets-item hidden"
                            } else {
                                "all-sheets-item"
                            };
                            view! {
                                <div
                                    class=item_class
                                    on:click=move |_| {
                                        if is_hidden {
                                            model.update_value(|m| { m.unhide_sheet(idx).ok(); });
                                        }
                                        model.update_value(|m| { m.set_selected_sheet(idx).ok(); });
                                        state.request_redraw();
                                        open.set(false);
                                    }
                                >
                                    {name}
                                    {if is_hidden { " (hidden)" } else { "" }}
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </Show>
        </div>
    }
}
