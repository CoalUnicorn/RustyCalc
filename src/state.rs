//! Transient UI state and reactive signal primitives.
//!
//! [`WorkbookState`] holds all ephemeral UI state as [`Split<T>`] signal pairs.
//! The model itself lives in a [`ModelStore`] context value, not here.

use gloo_storage::Storage as GlooStorage;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::coord::{CellAddress, CellArea, RefSpan};
use crate::events::*;
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
#[derive(Clone, Copy, Debug, PartialEq)]
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
    /// Formula point-mode: highlighted range + byte span in formula text.
    Pointing { range: CellArea, ref_span: RefSpan },
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

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct WorkbookState {
    pub events: EventBus,
    pub(crate) current_uuid: Split<Option<WorkbookId>>,
    pub(crate) recent_colors: Split<Vec<CssColor>>,
    pub(crate) editing_cell: Split<Option<EditingCell>>,
    pub(crate) formula_input_ref: NodeRef<leptos::html::Input>,
    pub(crate) drag: Split<DragState>,
    pub(crate) context_menu: Split<Option<ContextMenuState>>,
    pub(crate) status: Split<Option<StatusMessage>>,
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
            drag: Split::new(DragState::Idle),
            context_menu: Split::new(None),
            status: Split::new(None),
        }
    }

    /// Active point-mode range, or 1x1 at the current cell if not pointing yet.
    pub(crate) fn effective_point_range(&self, model: ModelStore) -> CellArea {
        if let DragState::Pointing { range, .. } = self.drag.get_untracked() {
            range
        } else {
            model.with_value(|m| {
                let v = m.get_selected_view();
                CellArea::from_cell(v.row, v.column)
            })
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

        // Normalize color (ensure lowercase, with #) and wrap in the domain type.
        let normalized = CssColor::new(if color.starts_with('#') {
            color.to_lowercase()
        } else {
            format!("#{}", color.to_lowercase())
        });

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
