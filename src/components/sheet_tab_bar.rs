use crate::components::color_picker::TabColorPicker;
use crate::components::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
use crate::components::inline_rename::InlineRenameInput;
use crate::input::sheet::{execute_sheet, SheetAction};
use crate::model::CssColor;
use crate::model::{frontend_model::SHEET_STATE_VISIBLE, FrontendModel};
use crate::state::{ModelStore, WorkbookState};
use leptos::prelude::*;

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

    // Local UI state - split so read-only children don't re-run on writes from siblings.
    let (renaming, set_renaming) = signal(None::<u32>);

    let visible_sheets = move || {
        let _ = state.events.structure.get();
        let _ = state.events.navigation.get();
        model.with_value(|m| m.get_sheet_visible())
    };

    let on_add = move |_| {
        execute_sheet(&SheetAction::Add, model, &state);
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

    // Each SheetTab owns its own menu state - no parent coordination needed.
    // ContextMenu's backdrop ensures at most one menu is visible at a time.
    let (menu_open, set_menu_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));

    let is_selected = move || {
        let _ = state.events.navigation.get();
        model.with_value(|m| m.get_selected_view().sheet == sheet_idx)
    };

    let name = move || {
        let _ = state.events.structure.get();
        model.with_value(|m| m.get_sheet_name(sheet_idx as usize))
    };

    // Single derived signal for the tab color - used by both color_bar_style
    // and TabColorPicker to avoid a duplicate reactive subscription.
    let current_tab_color = Signal::derive(move || {
        let _ = state.events.structure.get();
        model.with_value(|m| m.get_sheet_tab_color(sheet_idx as usize))
    });

    let on_click = move |_: web_sys::MouseEvent| {
        execute_sheet(&SheetAction::Select(sheet_idx), model, &state);
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        ev.prevent_default();
        set_renaming.set(Some(sheet_idx));
    };

    let on_rename_commit = {
        Callback::new(move |new_name: String| {
            if !new_name.trim().is_empty() {
                execute_sheet(
                    &SheetAction::Rename {
                        sheet: sheet_idx,
                        name: new_name,
                    },
                    model,
                    &state,
                );
            }
            set_renaming.set(None);
        })
    };

    let on_rename_cancel = {
        Callback::new(move |()| {
            set_renaming.set(None);
        })
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

    // Context menu action handlers - inlined from the old TabContextMenu component.

    // Plain closure: avoids Memo::get() panicking when the reactive owner
    // is disposed mid-event-dispatch after a hide/delete mutation.
    let visible_count = move || {
        let _ = state.events.structure.get();
        model.with_value(|m| m.get_sheet_visible_count())
    };
    // `>` inside view! attributes parses as a closing tag - hoist the comparison.
    let can_hide_or_delete = move || visible_count() > 1;

    let on_rename = move || {
        set_renaming.set(Some(sheet_idx));
    };

    let on_color_change = Callback::new(move |color: Option<String>| {
        execute_sheet(
            &SheetAction::SetColor {
                sheet: sheet_idx,
                color,
            },
            model,
            &state,
        );
        set_menu_open.set(false);
    });

    let on_hide = move || {
        execute_sheet(&SheetAction::Hide(sheet_idx), model, &state);
    };

    let on_delete = move || {
        let sheet_name = model.with_value(|m| m.get_sheet_name(sheet_idx as usize));
        let confirmed = leptos::prelude::window()
            .confirm_with_message(&format!("Delete '{sheet_name}'? This cannot be undone."))
            .unwrap_or(false);
        if confirmed {
            execute_sheet(&SheetAction::Delete(sheet_idx), model, &state);
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
                <InlineRenameInput
                    value=name()
                    on_commit=on_rename_commit
                    on_cancel=on_rename_cancel
                    class="tab-rename-input"
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
        model.with_value(|m| m.get_sheet_all())
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
                            view! {
                                <div
                                    class=item_class
                                    on:click=move |_| {
                                        if is_hidden {
                                            execute_sheet(&SheetAction::Unhide(idx), model, &state);
                                        } else {
                                            execute_sheet(&SheetAction::Select(idx), model, &state);
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
