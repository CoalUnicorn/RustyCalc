//! Transient UI state and reactive signal primitives.
//!
//! [`WorkbookState`] holds all ephemeral UI state as [`Split<T>`] signal pairs.
//! The model itself lives in a [`ModelStore`] context value, not here.

mod autoscroll;
mod context_menu;
mod cursor_hint;
mod drag;
mod editing_cell;
mod named_range;
mod split;
mod status;
mod workbook_state;

pub use context_menu::{ContextMenuState, HeaderContextMenu};
pub use cursor_hint::CursorHint;
pub use drag::{DragState, RefOverride};
pub use editing_cell::{EditFocus, EditMode, EditingCell};
pub use named_range::EditingDefinedName;
pub use split::Split;
pub use status::StatusMessage;
pub use workbook_state::{ModelStore, WorkbookState};

