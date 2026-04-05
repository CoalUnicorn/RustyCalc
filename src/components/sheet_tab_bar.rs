use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::color_picker::TabColorPicker;
use crate::components::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
use crate::events::{FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent};
use crate::input::helpers::{mutate, try_mutate, EvaluationMode};
use crate::state::{ModelStore, WorkbookState};
use crate::storage;
use crate::util::warn_if_err;

#[derive(Debug, thiserror::Error)]
enum TabError {
    #[error("tab: {0}")]
    Engine(String),
}

/// IronCalc's canonical string value for a visible worksheet.
/// Used to guard against silent typos in state comparisons.
const SHEET_STATE_VISIBLE: &str = "visible";

// Main component

/// Sheet tab bar: `[ + ][ ≡ ][ Sheet1 ▾ | **Sheet2 ▾** | Sheet3 ▾ ]`
///
/// Holds all shared UI state as split signals and passes the appropriate halves
/// to children. Children that only read get a `ReadSignal`; children that need
/// to mutate get the paired `WriteSignal`. This matches the `WorkbookState` pattern.
#[component]
pub fn SheetTabBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Local UI state — split so read-only children don't re-run on writes from siblings.
    let (renaming, set_renaming) = signal(None::<u32>);

    let visible_sheets = move || {
        let _ = state.events.structure.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .into_iter()
                .enumerate()
                .filter(|(_, s)| s.state == SHEET_STATE_VISIBLE)
                .map(|(idx, s)| (s.sheet_id, idx as u32))
                .collect::<Vec<_>>()
        })
    };

    let on_add = move |_| {
        // Snapshot count before mutation — that index is the new sheet's position.
        let sheet_count = model.with_value(|m| m.get_worksheets_properties().len() as u32);
        mutate(model, &state, EvaluationMode::Deferred, |m| {
            m.new_sheet().ok();
        });
        if let Some(uuid) = state.current_uuid.get_untracked() {
            model.with_value(|m| storage::save(&uuid, m));
        }
        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::WorksheetAdded {
                sheet: sheet_count,
                name: format!("Sheet{}", sheet_count + 1),
            },
        ));
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
                    children=move |(_, sheet_idx)| view! {
                        <SheetTab
                            sheet_idx=sheet_idx
                            renaming=renaming
                            set_renaming=set_renaming
                        />
                    }
                />
            </div>
        </div>
    }
}

// Individual sheet tab

#[component]
fn SheetTab(
    sheet_idx: u32,
    renaming: ReadSignal<Option<u32>>,
    set_renaming: WriteSignal<Option<u32>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Each SheetTab owns its own menu state — no parent coordination needed.
    // ContextMenu's backdrop ensures at most one menu is visible at a time.
    let (menu_open, set_menu_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));

    let is_selected = move || {
        let _ = state.events.navigation.get();
        model.with_value(|m| m.get_selected_view().sheet == sheet_idx)
    };

    let name = move || {
        let _ = state.events.structure.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .get(sheet_idx as usize)
                .map(|s| s.name.clone())
                .unwrap_or_default()
        })
    };

    // Single derived signal for the tab color — used by both color_bar_style
    // and TabColorPicker to avoid a duplicate reactive subscription.
    let current_tab_color = Signal::derive(move || {
        let _ = state.events.structure.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .get(sheet_idx as usize)
                .and_then(|s| s.color.clone())
        })
    });

    let on_click = move |_: web_sys::MouseEvent| {
        let previous_sheet = model.with_value(|m| m.get_selected_view().sheet);
        warn_if_err(
            try_mutate(model, &state, EvaluationMode::Deferred, |m| {
                m.set_selected_sheet(sheet_idx).map_err(TabError::Engine)
            }),
            "set_selected_sheet",
        );
        if previous_sheet != sheet_idx {
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::ActiveSheetChanged {
                    from_sheet: previous_sheet,
                    to_sheet: sheet_idx,
                },
            ));
        }
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        ev.prevent_default();
        set_renaming.set(Some(sheet_idx));
    };

    let on_menu_toggle = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        set_menu_pos.set((ev.client_x(), ev.client_y()));
        set_menu_open.update(|v| *v = !*v);
    };

    let color_bar_style = move || {
        current_tab_color
            .get()
            .map(|c| format!("background:{c};"))
            .unwrap_or_default()
    };

    // Context menu action handlers — inlined from the old TabContextMenu component.

    // Plain closure: avoids Memo::get() panicking when the reactive owner
    // is disposed mid-event-dispatch after a hide/delete mutation.
    let visible_count = move || {
        let _ = state.events.structure.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .iter()
                .filter(|s| s.state == SHEET_STATE_VISIBLE)
                .count()
        })
    };
    // `>` inside view! attributes parses as a closing tag — hoist the comparison.
    let can_hide_or_delete = move || visible_count() > 1;

    let on_rename = move || {
        set_renaming.set(Some(sheet_idx));
    };

    let on_color_change = Callback::new(move |color: Option<String>| {
        // IronCalc treats "" as "clear tab color" — intentional sentinel.
        let hex = color.as_deref().unwrap_or("");
        warn_if_err(
            try_mutate(model, &state, EvaluationMode::Deferred, |m| {
                m.set_sheet_color(sheet_idx, hex).map_err(TabError::Engine)
            }),
            "set_sheet_color",
        );
        if let Some(uuid) = state.current_uuid.get_untracked() {
            model.with_value(|m| storage::save(&uuid, m));
        }
        if !hex.is_empty() {
            state.add_recent_color(hex);
        }
        state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
            sheet: sheet_idx,
            col: None,
            row: None,
        }));
        set_menu_open.set(false);
    });

    let on_hide = move || {
        mutate(model, &state, EvaluationMode::Deferred, |m| {
            m.hide_sheet(sheet_idx).ok();
        });
        if let Some(uuid) = state.current_uuid.get_untracked() {
            model.with_value(|m| storage::save(&uuid, m));
        }
        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::WorksheetHidden { sheet: sheet_idx },
        ));
    };

    let on_delete = move || {
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
            mutate(model, &state, EvaluationMode::Deferred, |m| {
                m.delete_sheet(sheet_idx).ok();
            });
            if let Some(uuid) = state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorksheetDeleted { sheet: sheet_idx },
            ));
        }
    };

    view! {
        <div
            class=move || if is_selected() { "sheet-tab selected" } else { "sheet-tab" }
            on:click=on_click
            on:dblclick=on_dblclick
        >
            <Show
                when=move || renaming.get() == Some(sheet_idx)
                fallback=move || view! { <span class="tab-name">{name}</span> }
            >
                <RenameInput
                    sheet_idx=sheet_idx
                    renaming=renaming
                    set_renaming=set_renaming
                />
            </Show>
            <span
                class="sheet-tab-menu"
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=on_menu_toggle
            >"≓"</span>
            <div class="tab-color-bar" style=color_bar_style />
            <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos above_anchor=true>
                <ContextMenuItem on_click=on_rename icon="✏">"Rename"</ContextMenuItem>
                <TabColorPicker current_color=current_tab_color on_change=on_color_change />
                <Show when=can_hide_or_delete>
                    <ContextMenuItem on_click=on_hide icon="👁">"Hide sheet"</ContextMenuItem>
                </Show>
                <ContextMenuSeparator />
                <Show when=can_hide_or_delete>
                    <ContextMenuItem on_click=on_delete icon="🗑" destructive=true>"Delete"</ContextMenuItem>
                </Show>
            </ContextMenu>
        </div>
    }
}

// Rename input

#[component]
fn RenameInput(
    sheet_idx: u32,
    renaming: ReadSignal<Option<u32>>,
    set_renaming: WriteSignal<Option<u32>>,
) -> impl IntoView {
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
            let old_name = model.with_value(|m| {
                m.get_worksheets_properties()
                    .get(sheet_idx as usize)
                    .map(|s| s.name.clone())
                    .unwrap_or_default()
            });
            mutate(model, &state, EvaluationMode::Deferred, |m| {
                m.rename_sheet(sheet_idx, &new_name).ok();
            });
            if let Some(uuid) = state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorksheetRenamed {
                    sheet: sheet_idx,
                    old_name,
                    new_name,
                },
            ));
        }
        set_renaming.set(None);
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        ev.stop_propagation();
        match ev.key().as_str() {
            "Enter" => {
                ev.prevent_default();
                let new_name = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .map(|i| i.value())
                    .unwrap_or_default();
                commit_rename(new_name);
            }
            "Escape" => {
                ev.prevent_default();
                set_renaming.set(None);
            }
            _ => {}
        }
    };

    let on_blur = move |ev: web_sys::FocusEvent| {
        // Guard: if Enter already fired commit_rename, set_renaming(None) ran first,
        // so this tab is no longer the active rename target — skip the double-commit.
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

// All sheets menu (hamburger)

/// `≡` button that lists all sheets (visible + hidden) for quick navigation.
#[component]
fn AllSheetsMenu() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let btn_ref = NodeRef::<leptos::html::Button>::new();

    let (open, set_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));

    let all_sheets = move || {
        let _ = state.events.structure.get();
        model.with_value(|m| {
            m.get_worksheets_properties()
                .into_iter()
                .enumerate()
                .map(|(idx, s)| (idx as u32, s.name.clone(), s.state.clone()))
                .collect::<Vec<_>>()
        })
    };

    let selected_sheet = move || {
        let _ = state.events.navigation.get();
        model.with_value(|m| m.get_selected_view().sheet)
    };

    let on_toggle = move |_: web_sys::MouseEvent| {
        if let Some(el) = btn_ref.get() {
            let rect = el.get_bounding_client_rect();
            set_menu_pos.set((rect.left() as i32, rect.top() as i32));
        }
        set_open.update(|v| *v = !*v);
    };

    view! {
        <div>
            <button
                node_ref=btn_ref
                class="tab-bar-hamburger"
                title="All sheets"
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=on_toggle
            >
                "≡"
            </button>
            <ContextMenu open=open set_open=set_open pos=menu_pos above_anchor=true>
                <div class="all-sheets-menu">
                    {move || {
                        let sheets = all_sheets();
                        let selected = selected_sheet();
                        sheets.into_iter().map(|(idx, name, sheet_state)| {
                            let is_hidden = sheet_state != SHEET_STATE_VISIBLE;
                            let is_active = idx == selected;
                            let item_class = if is_active {
                                "all-sheets-item active"
                            } else if is_hidden {
                                "all-sheets-item hidden"
                            } else {
                                "all-sheets-item"
                            };
                            let name_for_event = name.clone();
                            view! {
                                <div
                                    class=item_class
                                    on:click=move |_| {
                                        let previous_sheet =
                                            model.with_value(|m| m.get_selected_view().sheet);
                                        if is_hidden {
                                            mutate(model, &state, EvaluationMode::Deferred, |m| {m.unhide_sheet(idx).ok(); });
                                            state.emit_event(SpreadsheetEvent::Structure(
                                                StructureEvent::WorksheetUnhidden {
                                                    sheet: idx,
                                                    name: name_for_event.clone(),
                                                },
                                            ));
                                        }
                                        warn_if_err(
                            try_mutate(model, &state, EvaluationMode::Deferred, |m| {
                                m.set_selected_sheet(idx).map_err(TabError::Engine)
                            }),
                            "set_selected_sheet",
                        );
                                        if previous_sheet != idx {
                                            state.emit_event(SpreadsheetEvent::Navigation(
                                                NavigationEvent::ActiveSheetChanged {
                                                    from_sheet: previous_sheet,
                                                    to_sheet: idx,
                                                },
                                            ));
                                        }
                                        set_open.set(false);
                                    }
                                >
                                    {name}
                                    {if is_hidden { " (hidden)" } else { "" }}
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </ContextMenu>
        </div>
    }
}
