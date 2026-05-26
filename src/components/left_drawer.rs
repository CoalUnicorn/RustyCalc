//! Collapsible left sidebar showing all saved workbooks.
//!
//! Lists every workbook stored in the localStorage registry, optionally
//! grouped by `WorkbookMeta::group`.  Clicking a row saves the current
//! workbook and loads the selected one.  The x button deletes a workbook.

use leptos::prelude::*;

use crate::app_state::AppState;
use crate::components::context_menu::{
    ContextMenu, ContextMenuButton, ContextMenuItem, ContextMenuSeparator,
};
use crate::components::inline_rename::InlineRenameInput;
use crate::input::workbook::{execute_workbook, WorkbookAction};
use crate::state::{ModelStore, WorkbookState};
use crate::storage::{self, WorkbookGroup, WorkbookId, WorkbookMeta};
use std::collections::{HashMap, HashSet};

/// Entry ready for rendering: uuid, metadata, and whether it's the active workbook.
struct DrawerEntry {
    uuid: WorkbookId,
    meta: WorkbookMeta,
    active: bool,
}

struct DrawerGroup {
    label: WorkbookGroup,
    entries: Vec<DrawerEntry>,
}

// Grouping logic

fn group_entries(entries: Vec<DrawerEntry>) -> Vec<DrawerGroup> {
    let mut map: HashMap<WorkbookGroup, Vec<DrawerEntry>> = HashMap::new();

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
    let app = expect_context::<AppState>();

    // Local UI state - split so read-only children don't re-run on writes from siblings.
    let (renaming, set_renaming) = signal(None::<WorkbookId>);
    let (existing_groups, set_existing_groups) = signal(Vec::<String>::new());

    // Workbook list
    //
    // Re-reads when registry changes (workbook created/deleted/renamed/grouped)
    // or active workbook switches (current_uuid).
    let workbook_groups = move || {
        let _ = app.registry_version.get();
        let current = state.current_uuid.get();
        let registry = storage::load_registry();

        // Pre-compute existing group names once for all EntryRows,
        // avoiding N localStorage deserializations for context menus.
        let existing_groups: Vec<String> = {
            let mut groups: Vec<String> = registry
                .values()
                .filter_map(|m| {
                    if let WorkbookGroup::Named(n) = &m.group {
                        Some(n.clone())
                    } else {
                        None
                    }
                })
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            groups.sort();
            groups
        };
        set_existing_groups.set(existing_groups);

        let entries: Vec<DrawerEntry> = registry
            .into_iter()
            .map(|(uuid, meta)| {
                let active = current == Some(uuid);
                DrawerEntry { uuid, meta, active }
            })
            .collect();

        group_entries(entries)
    };

    let assign_group = move |(uuid, group): (WorkbookId, WorkbookGroup)| {
        storage::update_group(&uuid, group);
        app.bump_registry();
    };

    let switch_workbook = move |target_uuid: WorkbookId| {
        execute_workbook(WorkbookAction::Switch(target_uuid), model, &state, app);
    };

    let delete_workbook = move |(uuid, name): (WorkbookId, String)| {
        let safe = storage::sanitize_name(&name);
        let confirmed = leptos::prelude::window()
            .confirm_with_message(&format!("Delete '{safe}'? This cannot be undone."))
            .unwrap_or(false);
        if confirmed {
            execute_workbook(WorkbookAction::Delete(uuid), model, &state, app);
        }
    };

    let create_workbook = move |_| {
        execute_workbook(WorkbookAction::Create, model, &state, app);
    };

    // Toggle group collapse
    let toggle_group = move |group_label: String| {
        app.collapsed_groups.update(|groups| {
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
        <Show when=move || app.sidebar_open.get()>
            <div class="ld">
                <div class="ld-head">
                    <span class="ld-title">"Workbooks"</span>
                    <button
                        class="ld-btn ld-btn-add"
                        title="New workbook"
                        on:click=create_workbook
                    >
                        "+"
                    </button>
                </div>

                <div class="ld-body">
                    {move || {
                        let groups = workbook_groups();
                        let collapsed = app.collapsed_groups.get();
                        let _ = renaming.get();

                        groups.into_iter().map(|group| {
                            let is_collapsed = if let WorkbookGroup::Named(ref l) = group.label {
                                collapsed.contains(l)
                            } else {
                                false
                            };

                            let is_group = matches!(group.label, WorkbookGroup::Named(_));
                            view! {
                                <div class="ld-group">
                                    {if let WorkbookGroup::Named(label) = group.label.clone() {
                                        Some(view! { <GroupHeader label is_collapsed on_toggle /> })
                                    } else {
                                        None
                                    }}
                                    <Show when=move || !is_collapsed>
                                        {group.entries.iter().map(|entry| view! {
                                            <EntryRow
                                                uuid=entry.uuid
                                                name=entry.meta.name.clone()
                                                active=entry.active
                                                current_group=entry.meta.group.clone()
                                                shared_from_link=entry.meta.shared_from_link
                                                existing_groups=existing_groups
                                                on_switch
                                                on_delete
                                                on_group
                                                renaming
                                                set_renaming
                                            />
                                        }).collect::<Vec<_>>()}
                                    </Show>
                                </div>
                                {is_group.then(|| view! { <hr class="ld-sep" /> })}
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
            class="ld-group-hd"
            on:click=move |_| on_toggle.run(label_click.clone())
        >
            <span class="ld-chevron">
                {if is_collapsed { "\u{25b6}" } else { "\u{25bc}" }}
            </span>
            <span>{label}</span>
        </div>
    }
}

#[component]
fn EntryRow(
    uuid: WorkbookId,
    name: String,
    active: bool,
    current_group: WorkbookGroup,
    shared_from_link: bool,
    existing_groups: ReadSignal<Vec<String>>,
    on_switch: Callback<WorkbookId>,
    on_delete: Callback<(WorkbookId, String)>,
    on_group: Callback<(WorkbookId, WorkbookGroup)>,
    renaming: ReadSignal<Option<WorkbookId>>,
    set_renaming: WriteSignal<Option<WorkbookId>>,
) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let app = expect_context::<AppState>();

    let uuid_switch = uuid;
    let uuid_delete = uuid;
    let is_renaming = renaming.get_untracked() == Some(uuid);

    // Pre-clone so the fallback closure can move `name` while the delete
    // button also needs a copy for the confirmation dialog.
    let delete_name = name.clone();

    let (menu_open, set_menu_open) = signal(false);
    let (menu_pos, set_menu_pos) = signal((0i32, 0i32));
    let (group_name, set_group_name) = signal(String::new());

    let existing_groups = existing_groups.get_untracked();

    let has_group = matches!(current_group, WorkbookGroup::Named(_));

    let uuid_for_group = uuid;
    let commit_group = Callback::new(move |name: String| {
        if !name.trim().is_empty() {
            on_group.run((uuid_for_group, WorkbookGroup::Named(name)));
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

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        set_renaming.set(Some(uuid));
    };

    let on_rename_commit = Callback::new(move |new_name: String| {
        if !new_name.trim().is_empty() {
            storage::update_name(&uuid, &new_name);
            // Keep the in-memory model in sync so the next save() won't revert the name.
            if state.current_uuid.get_untracked() == Some(uuid) {
                model.update_value(|m| m.set_name(&new_name));
            }
            // Renaming is an explicit user action — clear the \"shared from link\"
            // quarantine badge. We don't auto-promote on autosave anymore so the
            // user must rename to signal trust.
            storage::promote_from_shared(&uuid);
            app.bump_registry();
        }
        set_renaming.set(None);
    });

    let on_rename_cancel = {
        Callback::new(move |()| {
            set_renaming.set(None);
        })
    };

    view! {
        <div
            class="ld-entry"
            class:active=active
            on:click=move |_| on_switch.run(uuid_switch)
            on:dblclick=on_dblclick

        >
            <ContextMenuButton
                set_open=set_menu_open
                set_pos=set_menu_pos
                class="ld-btn ld-grp"
            >
                "\u{22ee}"
            </ContextMenuButton>

            <Show
                when=move || is_renaming
                fallback={let n = storage::sanitize_name(&delete_name); move || view!{
                    <span class="ld-name">
                        {n.clone()}
                        {if shared_from_link {
                            Some(view! { <span class="ld-shared-badge" title="Shared from link — click to edit and clear this badge">"🔗"</span> })
                        } else {
                            None
                        }}
                    </span>
                }}
            >
                <InlineRenameInput
                    value=name.clone()
                    on_commit=on_rename_commit
                    on_cancel=on_rename_cancel
                    class="ld-rename"
                />
            </Show>

            <button
                class="ld-btn ld-del"
                title="Delete workbook"
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    on_delete.run((uuid_delete, delete_name.clone()));
                }
            >
                "\u{00d7}"
            </button>

            <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
                {has_group.then(|| {
                    let uuid_rm = uuid;
                    view! {
                        <ContextMenuItem on_click=move || on_group.run((uuid_rm, WorkbookGroup::Ungrouped))>
                            "No group"
                        </ContextMenuItem>
                    }
                })}

                {existing_groups.into_iter().map(|group| {
                    let uuid_g = uuid;
                    let g = group.clone();
                    let is_current = current_group == WorkbookGroup::Named(group.clone());
                    view! {
                        <ContextMenuItem on_click=move || on_group.run((uuid_g, WorkbookGroup::Named(g.clone())))>
                            {if is_current { "\u{2713} " } else { "" }}
                            {group}
                        </ContextMenuItem>
                    }
                }).collect::<Vec<_>>()}

                <ContextMenuSeparator />

                <div class="ld-pop-new">
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
