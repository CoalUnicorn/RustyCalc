//! Context menu overlay for column and row headers.
//!
//! Reads [`WorkbookState::context_menu`] set by
//! [`crate::input::mouse::handle_contextmenu`] on right-click and renders a
//! [`ContextMenu`] with structural actions for the targeted header.

use leptos::prelude::*;

use crate::components::ui::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
use crate::components::ui::popover::Popover;
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::input::mouse::header_span::Axis;
use crate::input::structure::StructAction;
use crate::model::frontend_model::ActiveCellQuery;
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
    let is_frozen = move || model.with_value(|m| m.frozen_panes().is_frozen());

    let dispatch = move |action: StructAction| {
        state.context_menu.set(None);
        execute(&SpreadsheetAction::Structure(action), model, &state);
    };

    // Popover signals for the resize-by-value panel.
    // These are independent of `state.context_menu` so the popover survives
    // after the menu closes and clears that signal.
    let (resize_open, set_resize_open) = signal(false);
    let (resize_pos, set_resize_pos) = signal((0i32, 0i32));
    // (axis, first_index, count, current_extent)
    let (resize_target, set_resize_target) = signal::<Option<(Axis, i32, i32, f64)>>(None);
    let (resize_text, set_resize_text) = signal(String::new());

    // Snapshot menu position + current extent, then open the resize popover.
    // Called from on_click handlers, so .get_untracked() avoids reactive warnings.
    let open_resize = move |axis: Axis, first: i32, count: i32| {
        let pos = state
            .context_menu
            .get_untracked()
            .map(|c| (c.x, c.y))
            .unwrap_or((0, 0));
        let extent = model.with_value(|m| {
            let sheet = m.get_selected_sheet();
            match axis {
                Axis::Col => m.get_column_width(sheet, first).unwrap_or(0.0),
                Axis::Row => m.get_row_height(sheet, first).unwrap_or(0.0),
            }
        });
        state.context_menu.set(None);
        set_resize_pos.set(pos);
        set_resize_target.set(Some((axis, first, count, extent)));
        set_resize_text.set(format!("{:.0}", extent));
        set_resize_open.set(true);
    };

    view! {
        <>
            <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
                {move || match state.context_menu.get() {
                    Some(ctx) => match ctx.target {
                        HeaderContextMenu::Column { col, count } => view! {
                            <ContextMenuItem
                                icon="<->"
                                on_click=move || open_resize(Axis::Col, col, count)
                            >
                                "Column width..."
                            </ContextMenuItem>
                            <ContextMenuSeparator />
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
                                icon="<-"
                                on_click=move || dispatch(StructAction::MoveColumn { col, delta: -1 })
                            >
                                "Move Left"
                            </ContextMenuItem>
                            <ContextMenuItem
                                icon="->"
                                on_click=move || dispatch(StructAction::MoveColumn { col, delta: 1 })
                            >
                                "Move Right"
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem
                                icon="❄"
                                on_click=move || dispatch(StructAction::FreezeUpToColumn { col })
                            >
                                {format!("Freeze up to column {}", col)}
                            </ContextMenuItem>
                            {move || is_frozen().then(|| view! {
                                <ContextMenuItem
                                    icon="✕"
                                    on_click=move || dispatch(StructAction::Unfreeze)
                                >
                                    "Unfreeze panes"
                                </ContextMenuItem>
                            })}
                        }
                        .into_any(),
                        HeaderContextMenu::Row { row, count } => view! {
                            <ContextMenuItem
                                icon="↕"
                                on_click=move || open_resize(Axis::Row, row, count)
                            >
                                "Row height..."
                            </ContextMenuItem>
                            <ContextMenuSeparator />
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
                                {format!("Freeze up to row {}", row)}
                            </ContextMenuItem>
                            {move || is_frozen().then(|| view! {
                                <ContextMenuItem
                                    icon="✕"
                                    on_click=move || dispatch(StructAction::Unfreeze)
                                >
                                    "Unfreeze panes"
                                </ContextMenuItem>
                            })}
                        }.into_any(),
                    }.into_any(),
                    None => ().into_any(),
                }}
            </ContextMenu>
            <Popover open=resize_open set_open=set_resize_open pos=resize_pos class="resize-popover">
                {move || match resize_target.get() {
                    Some((axis, first, count, _)) => {
                        let commit = move || {
                            if let Ok(v) = resize_text.get_untracked().trim().parse::<f64>() {
                                let action = match axis {
                                    Axis::Col => StructAction::SetColumnWidth { col: first, count, width: v },
                                    Axis::Row => StructAction::SetRowHeight { row: first, count, height: v },
                                };
                                dispatch(action);
                            }
                            set_resize_open.set(false);
                        };
                        view! {
                            <input
                                type="number"
                                prop:value=move || resize_text.get()
                                on:input=move |ev| set_resize_text.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" { commit(); }
                                    if ev.key() == "Escape" { set_resize_open.set(false); }
                                }
                            />
                            <button on:click=move |_| commit()>"OK"</button>
                        }.into_any()
                    }
                    None => ().into_any(),
                }}
            </Popover>
        </>
    }
}
