//! The top-level transient UI state struct shared across all components.

use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::coord::{CellAddress, RefNode, SheetRange};
use crate::events::*;
use crate::model::CssColor;
use crate::storage::WorkbookId;

use super::autoscroll::AutoscrollState;
use super::context_menu::ContextMenuState;
use super::cursor_hint::CursorHint;
use super::drag::{DragState, RefOverride};
use super::editing_cell::{EditFocus, EditingCell};
use super::named_range::EditingDefinedName;
use super::split::Split;
use super::status::StatusMessage;

pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

#[derive(Clone, Copy)]
pub struct WorkbookState {
    pub events: EventBus,
    pub(crate) current_uuid: Split<Option<WorkbookId>>,
    pub(crate) recent_colors: Split<Vec<CssColor>>,
    pub(crate) editing_cell: Split<Option<EditingCell>>,
    pub(crate) formula_input_ref: NodeRef<leptos::html::Textarea>,
    pub(crate) cell_editor_ref: NodeRef<leptos::html::Textarea>,
    pub(crate) drag: Split<DragState>,
    /// Idle-hover cursor style hint; written by `handle_mousemove`'s
    /// `buttons() == 0` branch after a `resize_handle_at` + `hit_test`
    /// probe. The worksheet `class=` memo composes this with `drag`.
    pub(crate) hover_cursor: Split<CursorHint>,
    /// Ghost-range published by `DragState::DraggingFormulaRef` mousemoves.
    /// Cleared on mouseup, on Escape, and on the mouseup-missed bail-out.
    pub(crate) dragged_ref_override: Split<Option<RefOverride>>,
    pub(crate) context_menu: Split<Option<ContextMenuState>>,
    pub(crate) status: Split<Option<StatusMessage>>,
    pub(crate) autoscroll: AutoscrollState,
    /// Whether the "Manage Named Ranges" modal is mounted.
    /// Toggled by the toolbar `Names` button and the dialog's close handlers.
    pub(crate) named_ranges_modal_open: Split<bool>,
    /// Selected / in-progress row in the Manage Named Ranges dialog.
    /// `None` while no row is being edited (initial state, or after Save /
    /// Cancel). The dialog's `<FormulaInput>` reads/writes through this signal
    /// via the shared [`crate::input::formula::sync_edit`] helper.
    pub(crate) editing_named_range: Split<Option<EditingDefinedName>>,
    /// Combined row+column header visibility (View → Show Headers). App-level
    /// view preference; not persisted in the workbook. Read untracked by the
    /// canvas adapter; the toggle emits FormatEvent::LayoutChanged to repaint.
    pub(crate) show_headers: Split<bool>,
}

impl WorkbookState {
    pub fn new(events: EventBus) -> Self {
        // Load recent colors from localStorage (CssColor is serde-transparent, same JSON as String)
        let recent_colors: Vec<CssColor> =
            <gloo_storage::LocalStorage as GlooStorage>::get("rustycalc_recent_colors")
                .unwrap_or_default();

        Self {
            events,
            current_uuid: Split::new(None),
            recent_colors: Split::new(recent_colors),
            editing_cell: Split::new(None),
            formula_input_ref: NodeRef::new(),
            cell_editor_ref: NodeRef::new(),
            drag: Split::new(DragState::Idle),
            hover_cursor: Split::new(CursorHint::default()),
            dragged_ref_override: Split::new(None),
            context_menu: Split::new(None),
            status: Split::new(None),
            autoscroll: AutoscrollState::new(),
            named_ranges_modal_open: Split::new(false),
            editing_named_range: Split::new(None),
            show_headers: Split::new(true),
        }
    }

    /// Active point-mode reference as a `RefNode`, or a 1x1 reference at the
    /// current cell when point-mode hasn't started yet.
    ///
    /// Returning `RefNode` (not `CellArea`) preserves absolute-flag and
    /// sheet-qualification state end-to-end: `try_point_move` never has to
    /// rebuild what the drag state already carries.
    pub(crate) fn effective_point_ref(&self, model: ModelStore) -> RefNode {
        if let DragState::Pointing { ref_node, .. } = self.drag.get_untracked() {
            ref_node
        } else {
            let editing = model.with_value(CellAddress::from_view);
            let area = SheetRange::from_cell(editing.sheet, editing.row, editing.column);
            RefNode::from_cell_area(area, editing, "")
        }
    }

    //  Event System (delegates to EventBus)

    pub fn emit_event(&self, event: SpreadsheetEvent) {
        self.events.emit_event(event);
    }

    pub fn emit_events(&self, new_events: impl IntoIterator<Item = SpreadsheetEvent>) {
        self.events.emit_events(new_events);
    }

    /// Add a color to the recent colors list
    ///
    /// - Moves color to front if already exists
    /// - Limits list to 16 colors maximum
    /// - Persists to localStorage
    /// - Ignores colors already in COLOR_PALETTE
    pub fn add_recent_color(&self, color: &str) {
        use crate::theme::COLOR_PALETTE;

        // Don't add colors that are already in the standard palette
        if COLOR_PALETTE.contains(&color) {
            return;
        }

        // Normalize color.
        let normalized = CssColor::new(color);

        self.recent_colors.update(|colors| {
            // Remove if already exists
            colors.retain(|c| c != &normalized);

            // Add to front
            colors.insert(0, normalized);

            // Limit to 16 colors
            colors.truncate(16);
        });

        // Convert to Vec<CssColor> for storage and event (same JSON representation)
        let string_colors: Vec<CssColor> = self
            .recent_colors
            .with_untracked(|colors| colors.iter().map(|c| c.to_owned()).collect());
        <gloo_storage::LocalStorage as GlooStorage>::set("rustycalc_recent_colors", &string_colors)
            .ok();

        // Emit event for reactive subscribers
        self.emit_event(SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated {
            colors: string_colors,
        }));
    }

    /// Restore keyboard focus to whichever formula input the user was editing.
    ///
    /// Called after point-mode mouse drags so the user can continue typing
    /// the formula without clicking again.
    pub fn refocus_formula_input(&self) {
        use wasm_bindgen::JsCast;
        let Some(edit) = self.editing_cell.get_untracked() else {
            return;
        };
        match edit.focus {
            EditFocus::FormulaBar => {
                if let Some(el) = self.formula_input_ref.get_untracked() {
                    el.focus().ok();
                }
            }
            EditFocus::Cell => {
                if let Some(el) = self.cell_editor_ref.get_untracked() {
                    el.unchecked_into::<web_sys::HtmlElement>().focus().ok();
                }
            }
        }
    }
}
