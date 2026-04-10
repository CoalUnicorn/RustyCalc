//! Collapsible left sidebar showing all saved workbooks.
//!
//! Lists every workbook stored in the localStorage registry, optionally
//! grouped by `WorkbookMeta::group`.  Clicking a row saves the current
//! workbook and loads the selected one.  The x button deletes a workbook.

use leptos::prelude::*;

use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::state::{DragState, ModelStore, WorkbookState};
use crate::storage::{self, WorkbookMeta};
use std::collections::HashMap;

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
        .map(|(label, entries)| DrawerGroup { label, entries })
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
        storage::delete(&uuid);

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

    // View
    view! {
        <Show when=move || state.sidebar_open.get()>
            <div class="left-drawer">
                // Header
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

                // Scrollable body
                <div class="left-drawer__body">
                    {move || {
                        let groups = workbook_groups();
                        let collapsed = state.collapsed_groups.get();

                        groups.into_iter().map(|group| {
                            let is_named = group.label.is_some();
                            let is_collapsed = group.label.as_ref()
                                .is_some_and(|l| collapsed.contains(l));
                            let _label_for_toggle = group.label.clone();

                            view! {
                                <div class="left-drawer__group">
                                    // Group header (only for named groups)
                                    {is_named.then(|| {
                                        let label = group.label.clone().unwrap_or_default();
                                        let label_click = label.clone();
                                        let toggle = toggle_group;
                                        view! {
                                            <div
                                                class="left-drawer__group-header"
                                                on:click=move |_| toggle(label_click.clone())
                                            >
                                                <span class="left-drawer__chevron">
                                                    {if is_collapsed { "\u{25b6}" } else { "\u{25bc}" }}
                                                </span>
                                                <span>{label}</span>
                                            </div>
                                        }
                                    })}

                                    // Entries (hidden when group is collapsed)
                                    <Show when=move || !is_collapsed>
                                        {
                                            let entries_view: Vec<_> = group.entries.iter().map(|entry| {
                                                let uuid_switch = entry.uuid.clone();
                                                let uuid_delete = entry.uuid.clone();
                                                let name = entry.meta.name.clone();
                                                let active = entry.active;
                                                let switch = switch_workbook;
                                                let delete = delete_workbook;

                                                view! {
                                                    <div
                                                        class="left-drawer__entry"
                                                        class:active=active
                                                        on:click=move |_| switch(uuid_switch.clone())
                                                    >
                                                        <span class="left-drawer__entry-name">
                                                            {name}
                                                        </span>
                                                        <button
                                                            class="left-drawer__btn left-drawer__btn--delete"
                                                            title="Delete workbook"
                                                            on:click=move |ev: web_sys::MouseEvent| {
                                                                ev.stop_propagation();
                                                                delete(uuid_delete.clone());
                                                            }
                                                        >
                                                            "\u{00d7}"
                                                        </button>
                                                    </div>
                                                }
                                            }).collect();
                                            entries_view
                                        }
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
