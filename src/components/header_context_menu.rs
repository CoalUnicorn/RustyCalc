//! Context menu overlay for column and row headers.
//!
//! Reads [`WorkbookState::context_menu`] set by
//! [`crate::input::mouse::handle_contextmenu`] on right-click and renders a
//! [`ContextMenu`] with structural actions for the targeted header.

use leptos::prelude::*;

use crate::components::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
use crate::input::keyboard::{execute, SpreadsheetAction};
use crate::input::structure::StructAction;
use crate::state::{HeaderContextMenu, ModelStore, WorkbookState};

/// Viewport-level overlay for column and row header right-click menus.
///
/// Place once inside the workbook layout. The menu closes automatically when
/// the user clicks outside or selects an action; the underlying
/// `state.context_menu` signal is cleared on close.
#[component]
pub fn HeaderContextMenuOverlay() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // ContextMenu needs (ReadSignal<bool>, WriteSignal<bool>, ReadSignal<(i32,i32)>).
    // state.context_menu is Split<Option<ContextMenuState>>.
    // Two Effects bridge them: one pushes opens/position in; one pushes closes out.
    let (menu_open, set_menu_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));

    // Push opens/position in from state.
    Effect::new(move |_| match state.context_menu.get() {
        Some(ctx) => {
            set_menu_pos.set((ctx.x, ctx.y));
            set_menu_open.set(true);
        }
        None => set_menu_open.set(false),
    });

    // Push closes out to state (true -> false transition only).
    Effect::new(move |prev: Option<bool>| {
        let is_open = menu_open.get();
        if prev == Some(true) && !is_open {
            state.context_menu.set(None);
        }
        is_open // becomes `prev` on the next run
    });

    // `dispatch` clears `state.context_menu` before executing the action
    // because `ContextMenuItem`'s `use_context::<WriteSignal<bool>>()` lookup
    // does not cross the reactive `move || match` closure boundary into
    // `ContextMenu`'s `provide_context` call.
    let dispatch = move |action: StructAction| {
        state.context_menu.set(None);
        execute(&SpreadsheetAction::Structure(action), model, &state);
    };

    view! {
        <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
            {move || match state.context_menu.get() {
                Some(ctx) => match ctx.target {
                    HeaderContextMenu::Column { col, count } => view! {
                        <ContextMenuItem
                            icon="+"
                            on_click=move || dispatch(if count > 1 {
                                StructAction::InsertColumns
                            } else {
                                StructAction::InsertColumnAt { col }
                            })
                        >
                            {if count > 1 { format!("Insert {count} Columns") } else { "Insert Column".to_string() }}
                        </ContextMenuItem>
                        <ContextMenuItem
                            icon="×"
                            destructive=true
                            on_click=move || dispatch(if count > 1 {
                                StructAction::DeleteColumns
                            } else {
                                StructAction::DeleteColumnAt { col }
                            })
                        >
                            {if count > 1 { format!("Delete {count} Columns") } else { "Delete Column".to_string() }}
                        </ContextMenuItem>
                        <ContextMenuSeparator />
                        <ContextMenuItem
                            icon="←"
                            on_click=move || dispatch(StructAction::MoveColumn { col, delta: -1 })
                        >
                            "Move Left"
                        </ContextMenuItem>
                        <ContextMenuItem
                            icon="→"
                            on_click=move || dispatch(StructAction::MoveColumn { col, delta: 1 })
                        >
                            "Move Right"
                        </ContextMenuItem>
                        <ContextMenuSeparator />
                        <ContextMenuItem
                            icon="❄"
                            on_click=move || dispatch(StructAction::FreezeUpToColumn { col })
                        >
                            "Freeze to Here"
                        </ContextMenuItem>
                    }
                    .into_any(),
                    HeaderContextMenu::Row { row, count } => view! {
                        <ContextMenuItem
                            icon="+"
                            on_click=move || dispatch(if count > 1 {
                                StructAction::InsertRows
                            } else {
                                StructAction::InsertRowAt { row }
                            })
                        >
                            {if count > 1 { format!("Insert {count} Rows") } else { "Insert Row".to_string() }}
                        </ContextMenuItem>
                        <ContextMenuItem
                            icon="×"
                            destructive=true
                            on_click=move || dispatch(if count > 1 {
                                StructAction::DeleteRows
                            } else {
                                StructAction::DeleteRowAt { row }
                            })
                        >
                            {if count > 1 { format!("Delete {count} Rows") } else { "Delete Row".to_string() }}
                        </ContextMenuItem>
                        <ContextMenuSeparator />
                        <ContextMenuItem
                            icon="↑"
                            on_click=move || dispatch(StructAction::MoveRow { row, delta: -1 })
                        >
                            "Move Up"
                        </ContextMenuItem>
                        <ContextMenuItem
                            icon="↓"
                            on_click=move || dispatch(StructAction::MoveRow { row, delta: 1 })
                        >
                            "Move Down"
                        </ContextMenuItem>
                        <ContextMenuSeparator />
                        <ContextMenuItem
                            icon="❄"
                            on_click=move || dispatch(StructAction::FreezeUpToRow { row })
                        >
                            "Freeze to Here"
                        </ContextMenuItem>
                    }
                    .into_any(),
                },
                None => ().into_any(),
            }}
        </ContextMenu>
    }
}
