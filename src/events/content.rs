//! Content-domain events: cell values, formulas, calculations.

use crate::coord::{CellAddress, SheetRange};

#[derive(Clone, PartialEq, Debug)]
pub enum ContentEvent {
    /// `old_value`/`new_value` are `None` when unavailable at the call site.
    CellChanged {
        address: CellAddress,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    RangeChanged {
        sheet_area: SheetRange,
    },
    FormulaChanged {
        address: CellAddress,
    },
    CalculationUpdated {
        affected_sheets: Vec<u32>,
    },
    NamedRangesChanged,
}
