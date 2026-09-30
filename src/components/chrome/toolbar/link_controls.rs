//! "Link" toolbar button and its editor popover.
//!
//! The popover edits the hyperlink of the active cell: destination kind,
//! target or location, tooltip, and the cell label. A link a formula owns
//! (`HYPERLINK`) is read-only — the formula recreates it on every
//! recalculation — so the editor shows the refusal and disables Apply and
//! Delete. A multi-cell selection is refused for a second reason: the engine
//! records one undo entry per cell, so a link applied across a range could not
//! be undone in one step. `execute_link` refuses both actions regardless, so
//! the guard does not depend on this UI.

use leptos::prelude::*;

use crate::components::ui::popover::Popover;
use crate::coord::{CellAddress, CellArea};
use crate::input::link::{
    FORMULA_OWNED, LinkAction, LinkKind, SINGLE_CELL_ONLY, execute_link, label_edit,
};
use crate::input::mouse::{CanvasHandle, with_canvas};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
use crate::util::refocus_workbook;

use ironcalc_base::UserModel;
use ironcalc_base::types::Link;

use super::icon::{Icon, SheetIcon};

/// Editor prefill, re-read from committed and model state every time the
/// popover opens — never kept across commits.
struct LinkSeed {
    kind: LinkKind,
    target: String,
    tooltip: String,
    label: String,
    /// A formula owns the link: the form renders read-only.
    dynamic: bool,
    /// The label becomes cell content, so it is editable for one cell only.
    single_cell: bool,
}

/// Toolbar button opening the hyperlink editor for the active cell.
#[component]
pub fn LinkButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let canvas_handle = expect_context::<CanvasHandle>();

    let (open, set_open) = signal(false);
    let (pos, set_pos) = signal((0i32, 0i32));

    let (kind, set_kind) = signal(LinkKind::External);
    let target = RwSignal::new(String::new());
    let tooltip = RwSignal::new(String::new());
    let label = RwSignal::new(String::new());
    // The text the field showed when the popover opened. Apply compares the
    // field against it, because an unchanged field must not rewrite the cell
    // (the seed is the formatted value, not the cell's input text).
    let seed_label = RwSignal::new(String::new());
    let dynamic = RwSignal::new(false);
    let single_cell = RwSignal::new(true);

    let trigger_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        if !open.get_untracked() {
            let seed = read_seed(model, canvas_handle);
            set_kind.set(seed.kind);
            target.set(seed.target);
            tooltip.set(seed.tooltip);
            seed_label.set(seed.label.clone());
            label.set(seed.label);
            dynamic.set(seed.dynamic);
            single_cell.set(seed.single_cell);
        }
        set_pos.set((ev.client_x(), ev.client_y()));
        set_open.update(|v| *v = !*v);
    };

    let apply = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let action = LinkAction::SetLink {
            kind: kind.get_untracked(),
            target_or_location: target.get_untracked().trim().to_string(),
            tooltip: trimmed_option(&tooltip.get_untracked()),
            label: label_edit(&seed_label.get_untracked(), &label.get_untracked()),
        };
        if let Err(e) = execute_link(&action, model, &state, canvas_handle) {
            state.status.set(Some(StatusMessage::Error(e.to_string())));
            return;
        }
        set_open.set(false);
        refocus_workbook();
    };

    let delete = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        if let Err(e) = execute_link(&LinkAction::DeleteLink, model, &state, canvas_handle) {
            state.status.set(Some(StatusMessage::Error(e.to_string())));
            return;
        }
        set_open.set(false);
        refocus_workbook();
    };

    let locked = move || dynamic.get();
    // A formula owns the link, or the selection is not one cell: either way the
    // action cannot run, so both disable the same controls and show the reason.
    let blocked = move || locked() || !single_cell.get();
    let refusal = move || {
        if locked() {
            FORMULA_OWNED
        } else {
            SINGLE_CELL_ONLY
        }
    };

    view! {
        <div class="tb-link">
            <button
                class="tb-btn"
                title=move || {
                    if locked() { FORMULA_OWNED } else { "Insert or edit the hyperlink" }
                }
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:click=trigger_click
            >
                <Icon icon=SheetIcon::Link />
                "Link"
            </button>
            <Popover open set_open pos class="tb-link-dropdown">
                <div class="lt-form">
                    <Show when=blocked>
                        <p class="lt-owned">{refusal}</p>
                    </Show>
                    <div class="lt-kind">
                        <button
                            class="lt-kind-btn"
                            class:on=move || kind.get() == LinkKind::External
                            disabled=blocked
                            on:click=move |ev: web_sys::MouseEvent| {
                                ev.stop_propagation();
                                set_kind.set(LinkKind::External);
                            }
                        >
                            "URL"
                        </button>
                        <button
                            class="lt-kind-btn"
                            class:on=move || kind.get() == LinkKind::Internal
                            disabled=blocked
                            on:click=move |ev: web_sys::MouseEvent| {
                                ev.stop_propagation();
                                set_kind.set(LinkKind::Internal);
                            }
                        >
                            "This workbook"
                        </button>
                    </div>
                    <input
                        type="text"
                        class="lt-input"
                        placeholder=move || {
                            if kind.get() == LinkKind::External {
                                "https://example.com or mailto:a@b.c"
                            } else {
                                "Sheet1!A1 or a defined name"
                            }
                        }
                        disabled=blocked
                        prop:value=target
                        on:input=move |ev| target.set(event_target_value(&ev))
                    />
                    <input
                        type="text"
                        class="lt-input"
                        placeholder="Tooltip (shown on hover)"
                        disabled=blocked
                        prop:value=tooltip
                        on:input=move |ev| tooltip.set(event_target_value(&ev))
                    />
                    <input
                        type="text"
                        class="lt-input"
                        placeholder="Cell text"
                        title="Cell text becomes the content of the single selected cell"
                        disabled=blocked
                        prop:value=label
                        on:input=move |ev| label.set(event_target_value(&ev))
                    />
                    <div class="lt-actions">
                        <button class="lt-apply" disabled=blocked on:click=apply>
                            "Apply"
                        </button>
                        <button class="lt-delete" disabled=blocked on:click=delete>
                            "Delete"
                        </button>
                    </div>
                </div>
            </Popover>
        </div>
    }
}

/// Read the active cell's link plus its label. Only static links live in the
/// worksheet's link map; a dynamic one is reported by the committed canvas
/// state instead, and switches the form to read-only.
fn read_seed(model: ModelStore, canvas_handle: CanvasHandle) -> LinkSeed {
    let (address, single_cell) = model.with_value(|m| {
        (
            CellAddress::from_view(m),
            CellArea::from_view(m).is_single_cell(),
        )
    });
    let dynamic = with_canvas(canvas_handle, |ic| ic.link_at(address.row, address.column))
        .flatten()
        .is_some_and(|link| link.is_dynamic());
    let stored = model.with_value(|m| {
        m.get_cell_link(address.sheet, address.row, address.column)
            .ok()
            .flatten()
    });
    let (kind, target, tooltip) = match stored {
        Some(Link::External { target, tooltip }) => (LinkKind::External, target, tooltip),
        Some(Link::Internal { location, tooltip }) => (LinkKind::Internal, location, tooltip),
        None => (LinkKind::External, String::new(), None),
    };
    let label = if single_cell {
        model.with_value(|m| cell_label(m, address))
    } else {
        String::new()
    };
    LinkSeed {
        kind,
        target,
        tooltip: tooltip.unwrap_or_default(),
        label,
        dynamic,
        single_cell,
    }
}

/// Displayed text of a cell, empty when the engine cannot report it.
fn cell_label(m: &UserModel<'_>, address: CellAddress) -> String {
    m.get_formatted_cell_value(address.sheet, address.row, address.column)
        .unwrap_or_default()
}

/// Trimmed tooltip text; empty means "no tooltip".
fn trimmed_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
