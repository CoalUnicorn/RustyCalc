//! Collapsible left sidebar showing all saved workbooks.
//!
//! Lists every workbook stored in the localStorage registry, optionally
//! grouped by `WorkbookMeta::group`.  Clicking a row saves the current
//! workbook and loads the selected one.  The x button deletes a workbook.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::context_menu::{
    ContextMenu, ContextMenuButton, ContextMenuItem, ContextMenuSeparator,
};
use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::state::{DragState, ModelStore, WorkbookState};
use crate::storage::{self, WorkbookMeta};
use std::collections::{HashMap, HashSet};

/// Entry ready for rendering: uuid, metadata, and whether it's the active workbook.
struct DrawerEntry {
    uuid: String,
    meta: WorkbookMeta,
    active: bool,
}

/// A group of workbook entries sharing the same `group` label.
struct DrawerGroup {
    /// Group label — `None` means ungrouped.
    label: Option<String>,
    entries: Vec<DrawerEntry>,
}

// Grouping logic

fn group_entries(entries: Vec<DrawerEntry>) -> Vec<DrawerGroup> {
    let mut map: HashMap<Option<String>, Vec<DrawerEntry>> = HashMap::new();

    for entry in entries {
        map.entry(entry.meta.group.clone()).or_default().push(entry);
    }

    let mut groups: Vec<DrawerGroup> = map
        .into_iter()
        .map(|(label, mut entries)| {
            entries.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
            DrawerGroup { label, entries }
        })
        .collect();

    groups.sort_by(|a, b| a.label.cmp(&b.label));

    groups
}

// Component

/// Collapsible left sidebar listing saved workbooks with optional grouping.
#[component]
pub fn LeftDrawer() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // Local UI state - split so read-only children don't re-run on writes from siblings.
    let (renaming, set_renaming) = signal(None::<String>);

    // Workbook list
    //
    // Re-reads when structure events fire (workbook created/deleted/switched).
    let workbook_groups = move || {
        let _ = state.events.structure.get(); // register dependency
        let current = state.current_uuid.get();
        let registry = storage::load_registry();

        let entries: Vec<DrawerEntry> = registry
            .into_iter()
            .map(|(uuid, meta)| {
                let active = current.as_deref() == Some(&uuid);
                DrawerEntry { uuid, meta, active }
            })
            .collect();

        group_entries(entries)
    };

    let assign_group = move |(uuid, group): (String, Option<String>)| {
        storage::update_group(&uuid, group);
        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::WorkbookGroupChanged { uuid },
        ));
    };

    // Switch handler
    let switch_workbook = move |target_uuid: String| {
        let cur = state.current_uuid.get_untracked();
        if cur.as_deref() == Some(&target_uuid) {
            return;
        }
        // Save current model before switching.
        if let Some(uuid) = &cur {
            model.with_value(|m| storage::save(uuid, m));
        }
        // Load the target workbook into the same ModelStore.
        if let Some(new_model) = storage::load(&target_uuid) {
            model.update_value(|m| *m = new_model);
            storage::set_selected_uuid(&target_uuid);
            state.current_uuid.set(Some(target_uuid.clone()));
            state.editing_cell.set(None);
            state.drag.set(DragState::Idle);

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorkbookSwitched {
                    from_uuid: cur,
                    to_uuid: target_uuid,
                },
            ));
        }
    };

    // Delete handler
    let delete_workbook = move |uuid: String| {
        let cur = state.current_uuid.get_untracked();

        let wb_name = model.with_value(|m| m.get_name());
        let confirmed = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message(&format!("Delete '{wb_name}'? This cannot be undone."))
                    .ok()
            })
            .unwrap_or(false);
        if confirmed {
            storage::delete(&uuid);
        }

        // If we deleted the active workbook, load the next available (or create fresh).
        if cur.as_deref() == Some(&uuid) {
            let (next_uuid, next_model) =
                storage::load_selected().unwrap_or_else(storage::create_new);
            model.update_value(|m| *m = next_model);
            storage::set_selected_uuid(&next_uuid);
            state.current_uuid.set(Some(next_uuid));
            state.editing_cell.set(None);
            state.drag.set(DragState::Idle);
        }

        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::WorkbookDeleted { uuid },
        ));
    };

    // Create handler
    let create_workbook = move |_| {
        // Save current before creating.
        if let Some(uuid) = state.current_uuid.get_untracked() {
            model.with_value(|m| storage::save(&uuid, m));
        }
        let (new_uuid, new_model) = storage::create_new();
        model.update_value(|m| *m = new_model);
        state.current_uuid.set(Some(new_uuid.clone()));
        state.editing_cell.set(None);
        state.drag.set(DragState::Idle);

        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::WorkbookCreated {
                uuid: new_uuid.clone(),
                name: model.with_value(|m| m.get_name()),
            },
        ));
    };

    // Toggle group collapse
    let toggle_group = move |group_label: String| {
        state.collapsed_groups.update(|groups| {
            if let Some(pos) = groups.iter().position(|g| g == &group_label) {
                groups.remove(pos);
            } else {
                groups.push(group_label);
            }
        });
    };

    let on_switch = Callback::new(switch_workbook);
    let on_delete = Callback::new(delete_workbook);
    let on_toggle = Callback::new(toggle_group);
    let on_group = Callback::new(assign_group);

    view! {
        <Show when=move || state.sidebar_open.get()>
            <div class="left-drawer">
                <div class="left-drawer__header">
                    <span class="left-drawer__title">"Workbooks"</span>
                    <button
                        class="left-drawer__btn left-drawer__btn--add"
                        title="New workbook"
                        on:click=create_workbook
                    >
                        "+"
                    </button>
                </div>

                <div class="left-drawer__body">
                    {move || {
                        let groups = workbook_groups();
                        let collapsed = state.collapsed_groups.get();
                        let _ = renaming.get();

                        groups.into_iter().map(|group| {
                            let is_collapsed = group.label.as_ref()
                                .is_some_and(|l| collapsed.contains(l));

                            view! {
                                <div class="left-drawer__group">
                                    {group.label.clone().map(|label| view! {
                                        <GroupHeader label is_collapsed on_toggle />
                                    })}
                                    <Show when=move || !is_collapsed>
                                        {group.entries.iter().map(|entry| view! {
                                            <EntryRow
                                                uuid=entry.uuid.clone()
                                                name=entry.meta.name.clone()
                                                active=entry.active
                                                current_group=entry.meta.group.clone().unwrap_or_default()
                                                on_switch
                                                on_delete
                                                on_group
                                                renaming
                                                set_renaming
                                            />
                                        }).collect::<Vec<_>>()}
                                    </Show>
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </div>
        </Show>
    }
}

// Child components

#[component]
fn GroupHeader(label: String, is_collapsed: bool, on_toggle: Callback<String>) -> impl IntoView {
    let label_click = label.clone();
    view! {
        <div
            class="left-drawer__group-header"
            on:click=move |_| on_toggle.run(label_click.clone())
        >
            <span class="left-drawer__chevron">
                {if is_collapsed { "\u{25b6}" } else { "\u{25bc}" }}
            </span>
            <span>{label}</span>
        </div>
    }
}

#[component]
fn EntryRow(
    uuid: String,
    name: String,
    active: bool,
    current_group: String,
    on_switch: Callback<String>,
    on_delete: Callback<String>,
    on_group: Callback<(String, Option<String>)>,
    renaming: ReadSignal<Option<String>>,
    set_renaming: WriteSignal<Option<String>>,
) -> impl IntoView {
    let uuid_switch = uuid.clone();
    let uuid_delete = uuid.clone();
    let is_renaming = renaming.get_untracked().as_deref() == Some(uuid.as_str());

    let (menu_open, set_menu_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));
    let (group_name, set_group_name) = signal(String::new());

    let existing_groups: Vec<String> = {
        let mut groups: Vec<String> = storage::load_registry()
            .values()
            .filter_map(|m| m.group.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        groups.sort();
        groups
    };

    let has_group = !current_group.is_empty();

    let uuid_for_group = uuid.clone();
    let commit_group = Callback::new(move |name: String| {
        if !name.trim().is_empty() {
            on_group.run((uuid_for_group.clone(), Some(name)));
        }
        set_group_name.set(String::new());
        set_menu_open.set(false);
    });

    let on_grp_keydown = move |ev: web_sys::KeyboardEvent| {
        ev.stop_propagation();
        match ev.key().as_str() {
            "Enter" => {
                ev.prevent_default();
                commit_group.run(group_name.get());
            }
            "Escape" => {
                ev.prevent_default();
                set_group_name.set(String::new());
                set_menu_open.set(false);
            }
            _ => {}
        }
    };

    view! {
        <div
            class="left-drawer__entry"
            class:active=active
            on:click=move |_| on_switch.run(uuid_switch.clone())
        >
            <ContextMenuButton
                set_open=set_menu_open
                set_pos=set_menu_pos
                class="left-drawer__btn left-drawer__btn--group"
            >
                "\u{22ee}"
            </ContextMenuButton>

            {if is_renaming {
                view! {
                    <RenameInput
                        uuid=uuid.clone()
                        initial_name=name.clone()
                        renaming
                        set_renaming
                    />
                }.into_any()
            } else {
                let uuid_dbl = uuid.clone();
                view! {
                    <span
                        class="left-drawer__entry-name"
                        on:dblclick=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            set_renaming.set(Some(uuid_dbl.clone()));
                        }
                    >
                        {name}
                    </span>
                }.into_any()
            }}

            <button
                class="left-drawer__btn left-drawer__btn--delete"
                title="Delete workbook"
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    on_delete.run(uuid_delete.clone());
                }
            >
                "\u{00d7}"
            </button>

            <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
                {has_group.then(|| {
                    let uuid_rm = uuid.clone();
                    view! {
                        <ContextMenuItem on_click=move || on_group.run((uuid_rm.clone(), None))>
                            "No group"
                        </ContextMenuItem>
                    }
                })}

                {existing_groups.into_iter().map(|group| {
                    let uuid_g = uuid.clone();
                    let g = group.clone();
                    let is_current = current_group == group;
                    view! {
                        <ContextMenuItem on_click=move || on_group.run((uuid_g.clone(), Some(g.clone())))>
                            {if is_current { "\u{2713} " } else { "" }}
                            {group}
                        </ContextMenuItem>
                    }
                }).collect::<Vec<_>>()}

                <ContextMenuSeparator />

                <div class="left-drawer__group-new">
                    <input
                        type="text"
                        placeholder="New group..."
                        prop:value=move || group_name.get()
                        on:input=move |ev| set_group_name.set(event_target_value(&ev))
                        on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                        on:keydown=on_grp_keydown
                    />
                </div>
            </ContextMenu>
        </div>
    }
}

// Rename input

#[component]
fn RenameInput(
    uuid: String,
    initial_name: String,
    renaming: ReadSignal<Option<String>>,
    set_renaming: WriteSignal<Option<String>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let input_ref = NodeRef::<leptos::html::Input>::new();

    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            let el2 = el.clone();
            wasm_bindgen_futures::spawn_local(async move {
                el2.focus().ok();
                el2.select();
            });
        }
    });

    let uuid_for_commit = uuid.clone();
    let commit_rename = Callback::new(move |new_name: String| {
        if !new_name.trim().is_empty() {
            let old_name = storage::load_registry()
                .get(&uuid_for_commit)
                .map(|m| m.name.clone())
                .unwrap_or_default();
            storage::update_name(&uuid_for_commit, &new_name);
            // Keep the in-memory model in sync so the next save() won't revert the name.
            if state.current_uuid.get_untracked().as_deref() == Some(&uuid_for_commit) {
                model.update_value(|m| m.set_name(&new_name));
            }
            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorkbookRenamed {
                    uuid: uuid_for_commit.clone(),
                    old_name,
                    new_name,
                },
            ));
        }
        set_renaming.set(None);
    });

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
                commit_rename.run(new_name);
            }
            "Escape" => {
                ev.prevent_default();
                set_renaming.set(None);
            }
            _ => {}
        }
    };

    let uuid_for_blur = uuid.clone();
    let on_blur = move |ev: web_sys::FocusEvent| {
        if renaming.get_untracked().as_deref() != Some(uuid_for_blur.as_str()) {
            return;
        }
        let new_name = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|i| i.value())
            .unwrap_or_default();
        commit_rename.run(new_name);
    };

    view! {
        <input
            node_ref=input_ref
            type="text"
            class="left-drawer__rename-input"
            prop:value=initial_name
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:mousedown=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:keydown=on_keydown
            on:blur=on_blur
        />
    }
}
