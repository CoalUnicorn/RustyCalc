//! Navigation-domain events: selection, scrolling, edit-mode transitions.

use crate::coord::{CellAddress, SheetRange};

#[derive(Clone, PartialEq, Debug)]
pub enum NavigationEvent {
    SelectionChanged {
        address: CellAddress,
    },
    /// Shift-click, Shift-arrow, or header click extended the selection.
    SelectionRangeChanged {
        sheet_area: SheetRange,
    },
    ViewportScrolled {
        sheet: u32,
        top_row: i32,
        left_col: i32,
    },
    ActiveSheetChanged {
        from_sheet: u32,
        to_sheet: u32,
    },
    EditingStarted {
        address: CellAddress,
    },
    EditingEnded {
        address: CellAddress,
        committed: bool,
    },
}
