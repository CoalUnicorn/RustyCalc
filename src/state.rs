//! Transient UI state and reactive signal primitives.
//!
//! [`WorkbookState`] holds all ephemeral UI state as [`Split<T>`] signal pairs.
//! The model itself lives in a [`ModelStore`] context value, not here.

use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use iron_canvas_web::RefZone;

use crate::coord::{CellAddress, RefNode, SheetRange, TextRef};
use crate::events::*;
use crate::input::formula_analysis::FormulaAnalysis;
use crate::model::CssColor;
use crate::storage::WorkbookId;

pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

/// Zero-cost wrapper around a Leptos `(ReadSignal, WriteSignal)` pair.
pub struct Split<T: Clone + Send + Sync + 'static>(ReadSignal<T>, WriteSignal<T>);

// Manual impls: ReadSignal<T>/WriteSignal<T> are always Copy (arena IDs),
// so Split<T> is Copy for any T - even non-Copy types like String or Vec.
// #[derive(Copy)] would incorrectly add a T: Copy bound.
impl<T: Clone + Send + Sync + 'static> Clone for Split<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Clone + Send + Sync + 'static> Copy for Split<T> {}

impl<T: Clone + Send + Sync + 'static> Split<T> {
    pub fn new(initial: T) -> Self {
        let (r, w) = signal(initial);
        Self(r, w)
    }

    pub fn get(&self) -> T {
        self.0.get()
    }

    pub fn get_untracked(&self) -> T {
        self.0.get_untracked()
    }

    #[allow(dead_code)]
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with(f)
    }

    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.0.with_untracked(f)
    }

    pub fn set(&self, v: T) {
        self.1.set(v);
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.1.update(f);
    }

    #[allow(dead_code)]
    pub fn read(&self) -> ReadSignal<T> {
        self.0
    }

    #[allow(dead_code)]
    pub fn write(&self) -> WriteSignal<T> {
        self.1
    }
}

/// Single enum ensures at most one drag mode is active — illegal
/// combinations (e.g. selecting while resizing) are unrepresentable.
///
/// `Pointing` carries an owned `RefNode` (non-Copy because its inner ironcalc
/// `Node` holds an `Option<String>` sheet name), so the enum is `Clone` only.
#[derive(Clone, Debug, PartialEq)]
pub enum DragState {
    /// No drag in progress.
    Idle,
    /// Mouse button held for a range-drag selection.
    Selecting,
    /// Autofill handle drag: the cell the user is dragging toward.
    Extending { to_row: i32, to_col: i32 },
    /// Column header resize: `(col_1based, current_mouse_x)`.
    ResizingCol { col: i32, x: f64 },
    /// Row header resize: `(row_1based, current_mouse_y)`.
    ResizingRow { row: i32, y: f64 },
    /// Formula point-mode: carries ironcalc's canonical reference Node plus the
    /// byte span of its rendered form in the edited formula text.
    Pointing {
        ref_node: RefNode,
        ref_text: TextRef,
    },
    /// Formula-ref overlay drag. `anchor` is the ref's range at mousedown;
    /// `grab_cell` is the cell under the cursor at mousedown. Mousemove
    /// uses both to compute the new range per `zone` without frame-to-frame
    /// state.
    DraggingFormulaRef {
        ref_idx: usize,
        zone: RefZone,
        anchor: SheetRange,
        grab_cell: CellAddress,
    },
}

/// Cursor style hint derived from the idle hover position. Drives the
/// `class` on `.ws-grid` so the cursor previews the action a mousedown
/// here would start (resize, autofill, ref-drag, …). Drag state wins
/// over this — the view composes both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorHint {
    #[default]
    Cell,
    ColResize,
    RowResize,
    Autofill,
    RefMove,
    RefExtendNS,
    RefExtendEW,
    RefCornerNwse,
    RefCornerNesw,
}

impl CursorHint {
    /// Extra class to append to `.ws-canvas.ws-grid`; empty string for
    /// the default cursor which is already set by `.ws-canvas`.
    pub fn class(self) -> &'static str {
        match self {
            CursorHint::Cell => "",
            CursorHint::ColResize => "resize-col",
            CursorHint::RowResize => "resize-row",
            CursorHint::Autofill => "cur-autofill",
            CursorHint::RefMove => "cur-ref-move",
            CursorHint::RefExtendNS => "cur-ref-ns",
            CursorHint::RefExtendEW => "cur-ref-ew",
            CursorHint::RefCornerNwse => "cur-ref-nwse",
            CursorHint::RefCornerNesw => "cur-ref-nesw",
        }
    }
}

/// Live preview of a formula-ref drag: the ref index and the range the
/// cursor currently resolves to. Mousemove publishes this; the worksheet
/// memo patches `formula_refs[idx].sheet_area` with `range` so the painted
/// outline follows the cursor without rewriting the formula text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefOverride {
    pub idx: usize,
    pub range: SheetRange,
}

/// Arrow key behavior during a cell edit.
#[derive(Clone, Debug, PartialEq)]
pub enum EditMode {
    /// Arrows commit and navigate. Default from printable keypress.
    Accept,
    /// Arrows move text cursor. Entered via F2 or double-click.
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditFocus {
    Cell,
    FormulaBar,
}

/// In-progress edit of a row in the Manage Named Ranges dialog.
///
/// Slim shape: every field is load-bearing. Compare with [`EditingCell`],
/// which carries `mode` / `focus` / `text_dirty` / a real `address` because
/// it lives inside the canvas's keyboard router. The dialog has none of
/// those concerns (no point-mode, no focus arbitration with the canvas), so
/// those fields would be dead weight here.
///
/// `sync_edit` works for both kinds of edit via the
/// [`crate::input::edit_sync::FormulaEditState`] trait.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingDefinedName {
    /// `None` when creating a new row; `Some((name, scope))` when editing an
    /// existing one. Identifies the row to call `rename_defined_name` against
    /// on save (vs. `create_defined_name` when `None`).
    pub(crate) original: Option<(String, Option<u32>)>,
    pub(crate) name: String,
    pub(crate) scope: Option<u32>,
    /// Formula body without the leading `=`. Stored bare so it round-trips
    /// against ironcalc's `new_defined_name` / `update_defined_name` (both
    /// expect the body, not the `=…` form).
    pub(crate) formula: String,
    pub(crate) cursor: usize,
    pub(crate) formula_analysis: FormulaAnalysis,
    /// Cell whose position interprets relative refs in `formula`. Captured
    /// from the active cell at dialog-open time (Excel's convention) and
    /// frozen for the lifetime of the edit, so toggling sheet tabs behind
    /// the modal can't shift the parser's frame underneath the user.
    pub(crate) context_cell: CellAddress,
}

impl EditingDefinedName {
    /// Formula side of the save gate: an analyzer error, or bare refs under
    /// Workbook scope. Workbook-scoped names need fully-qualified refs
    /// (`Sheet1!A1`) so they round-trip unambiguously regardless of the
    /// active view sheet.
    pub(crate) fn formula_invalid(&self) -> bool {
        self.formula_analysis.has_any_error()
            || (self.scope.is_none() && self.formula_analysis.has_bare_refs())
    }

    /// Full save gate: blank name, or [`Self::formula_invalid`].
    pub(crate) fn save_blockers(&self) -> bool {
        self.name.trim().is_empty() || self.formula_invalid()
    }
}

/// In-progress cell edit not yet committed to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingCell {
    pub(crate) address: CellAddress,
    pub(crate) text: String,
    pub(crate) mode: EditMode,
    pub(crate) focus: EditFocus,
    /// Set on user input (typing, paste); cleared on arrow key consumption.
    /// In `Edit` mode, gates whether arrows enter point-mode — distinguishes
    /// "typed an operator" from "cursor moved through a reference position".
    pub(crate) text_dirty: bool,
    /// Cached result of the last `analyze_formula()` call.
    /// Updated synchronously on each `on_input` event in formula_bar and cell_editor.
    pub(crate) formula_analysis: FormulaAnalysis,
    /// Cursor position (byte offset) in `text`, updated on every input event.
    pub(crate) cursor: usize,
}

/// Right-clicked header identity and the count of selected headers in that axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HeaderContextMenu {
    Column { col: i32, count: i32 },
    Row { row: i32, count: i32 },
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct ContextMenuState {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) target: HeaderContextMenu,
}

/// A user-visible message set by the input pipeline when an engine operation fails.
///
/// Stored on [`WorkbookState`] rather than the EventBus — errors are persistent
/// UI state (shown until dismissed), not fire-and-forget domain events.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusMessage {
    Error(String),
}

/// Non-reactive edge-scroll state: JS interval handle, scroll direction, last
/// mouse position. `StoredValue` (not `Split`) so start/stop never triggers
/// a reactive re-render.
#[derive(Clone, Copy)]
pub struct AutoscrollState {
    pub(crate) id: StoredValue<Option<i32>>,
    pub(crate) dir: StoredValue<(i32, i32)>,
    pub(crate) pos: StoredValue<(f64, f64)>,
}

impl AutoscrollState {
    fn new() -> Self {
        Self {
            id: StoredValue::new(None),
            dir: StoredValue::new((0, 0)),
            pos: StoredValue::new((0.0, 0.0)),
        }
    }

    pub(crate) fn cancel(&self) {
        if let Some(id) = self.id.get_value() {
            leptos::prelude::window().clear_interval_with_handle(id);
            self.id.set_value(None);
        }
        self.dir.set_value((0, 0));
    }
}

#[derive(Clone, Copy)]
pub struct WorkbookState {
    pub events: EventBus,
    pub(crate) current_uuid: Split<Option<WorkbookId>>,
    pub(crate) recent_colors: Split<Vec<CssColor>>,
    pub(crate) editing_cell: Split<Option<EditingCell>>,
    pub(crate) formula_input_ref: NodeRef<leptos::html::Input>,
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
    /// via the shared [`crate::input::edit_sync::sync_edit`] helper.
    pub(crate) editing_named_range: Split<Option<EditingDefinedName>>,
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

#[cfg(test)]
mod tests {
    use crate::Owner;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn status_initializes_to_none() {
        let owner = Owner::new();
        owner.with(|| {
            let state = crate::state::WorkbookState::new(crate::events::EventBus::new());
            assert_eq!(state.status.get_untracked(), None);
        });
    }
}
