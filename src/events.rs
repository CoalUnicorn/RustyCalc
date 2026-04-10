/*!
# Domain-driven Event System

Typed events representing actual changes in the spreadsheet domain.
Components subscribe to per-category `EventBus` signals and re-render
only when their category fires.

## Event Categories

- **Content**: Cell values, formulas, calculations
- **Format**: Visual styling, colors, layout
- **Structure**: Sheets, rows, columns
- **Navigation**: Selection, scrolling, editing state
- **Theme**: Appearance settings

## Usage

```rust
// Emit a typed event (via WorkbookState)
state.emit_event(SpreadsheetEvent::Format(
    FormatEvent::RangeStyleChanged { area: sa }
));

// Subscribe in an Effect (worksheet.rs pattern)
Effect::new(move |_| {
    let _content = state.events.content.get(); // registers dependency
    // ... render canvas
});
```
*/

use leptos::prelude::*;

use crate::coord::{CellAddress, SheetArea};
use crate::model::CssColor;
use crate::theme::Theme;

/// Per-category event signals.
///
/// Replaces the single `Vec<SpreadsheetEvent>` + 7 `Memo` filters.
/// Each signal holds the events from the most recent `emit_event(s)` call.
/// Contents are REPLACED (not appended) on each emit - never accumulate
/// cross-action history here.
#[derive(Clone, Copy)]
pub struct EventBus {
    pub content: RwSignal<Vec<ContentEvent>>,
    pub format: RwSignal<Vec<FormatEvent>>,
    pub navigation: RwSignal<Vec<NavigationEvent>>,
    pub structure: RwSignal<Vec<StructureEvent>>,
    pub theme: RwSignal<Vec<ThemeEvent>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            content: RwSignal::new(vec![]),
            format: RwSignal::new(vec![]),
            navigation: RwSignal::new(vec![]),
            structure: RwSignal::new(vec![]),
            theme: RwSignal::new(vec![]),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Domain-specific events that represent actual changes in the spreadsheet
#[derive(Clone, PartialEq, Debug)]
pub enum SpreadsheetEvent {
    /// Content of cells changed (values, formulas)
    Content(ContentEvent),
    /// Visual formatting changed (colors, fonts, layout)
    Format(FormatEvent),
    /// Structural changes (sheets, rows, columns)
    Structure(StructureEvent),
    /// Selection and navigation state
    Navigation(NavigationEvent),
    /// Theme and appearance settings
    Theme(ThemeEvent),
}

/// Cell content, formulas, and calculation results changed
#[derive(Clone, PartialEq, Debug)]
pub enum ContentEvent {
    /// A specific cell's content changed. `old_value`/`new_value` are `None`
    /// when the caller doesn't have the previous or next value available.
    CellChanged {
        address: CellAddress,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    /// A range of cells changed (bulk operations)
    RangeChanged { sheet_area: SheetArea },
    /// Formula in a cell was modified
    #[allow(dead_code)]
    FormulaChanged { address: CellAddress },
    /// Calculation chain updated (dependent cells recalculated)
    #[allow(dead_code)]
    CalculationUpdated { affected_sheets: Vec<u32> },
    /// Named range definitions changed
    #[allow(dead_code)]
    NamedRangesChanged,
    /// Generic content change (legacy compatibility)
    GenericChange,
}

/// Visual formatting and styling changes
#[derive(Clone, PartialEq, Debug)]
pub enum FormatEvent {
    /// Cell styling changed (font, colors, borders)
    #[allow(dead_code)]
    CellStyleChanged { address: CellAddress },
    /// Range styling changed (bulk formatting)
    RangeStyleChanged { area: SheetArea },
    /// Column width or row height changed
    LayoutChanged {
        sheet: u32,
        col: Option<i32>,
        row: Option<i32>,
    },
    /// Recent colors list updated
    RecentColorsUpdated { colors: Vec<CssColor> },
    /// Document colors extracted/changed
    #[allow(dead_code)]
    DocumentColorsChanged { colors: Vec<CssColor> },
    /// Conditional formatting rules changed
    #[allow(dead_code)]
    ConditionalFormattingChanged { sheet: u32 },
}

/// The axis being modified in a header operation.
#[derive(Clone, PartialEq, Debug)]
pub enum Dimension {
    /// Row axis. `start` is the 1-based row index where the operation begins.
    Row { start: Option<i32> },
    /// Column axis. `start` is the 1-based column index where the operation begins.
    Column { start: Option<i32> },
}

/// A contiguous span of rows or columns on a single sheet.
#[derive(Clone, PartialEq, Debug)]
pub struct Location {
    /// Sheet index (0-based).
    sheet: u32,
    /// First row or column in the span (1-based).
    start: i32,
    /// Number of rows or columns in the span.
    count: i32,
}

impl Location {
    pub fn new(sheet: u32, start: i32, count: i32) -> Self {
        Self {
            sheet,
            start,
            count,
        }
    }
}

/// A structural change to rows or columns on a single sheet.
#[derive(Clone, PartialEq, Debug)]
pub struct HeaderChange {
    /// Sheet where the change occurred (0-based index).
    pub sheet: u32,
    /// Whether rows/columns were inserted or deleted.
    pub operation: HeaderOperation,
    /// Which axis was affected and at which position.
    pub dimension: Dimension,
    /// Number of rows or columns affected.
    pub count: i32,
}

/// Whether rows or columns are being inserted or deleted.
#[derive(Clone, PartialEq, Debug)]
pub enum HeaderOperation {
    /// New rows or columns are being inserted before `start`.
    Insert,
    /// Existing rows or columns are being removed starting at `start`.
    Delete,
}

#[allow(dead_code)]
impl HeaderChange {
    fn rows(op: HeaderOperation, location: Location) -> Self {
        Self {
            sheet: location.sheet,
            operation: op,
            dimension: Dimension::Row {
                start: Some(location.start),
            },
            count: location.count,
        }
    }

    fn columns(op: HeaderOperation, location: Location) -> Self {
        Self {
            sheet: location.sheet,
            operation: op,
            dimension: Dimension::Column {
                start: Some(location.start),
            },
            count: location.count,
        }
    }

    pub fn insert_rows(location: Location) -> Self {
        Self::rows(HeaderOperation::Insert, location)
    }

    pub fn delete_rows(location: Location) -> Self {
        Self::rows(HeaderOperation::Delete, location)
    }

    pub fn insert_columns(location: Location) -> Self {
        Self::columns(HeaderOperation::Insert, location)
    }

    pub fn delete_columns(location: Location) -> Self {
        Self::columns(HeaderOperation::Delete, location)
    }

    /// Get the starting position (row or column index)
    pub fn start_position(&self) -> i32 {
        match &self.dimension {
            Dimension::Row { start } => start.unwrap_or(1),
            Dimension::Column { start } => start.unwrap_or(1),
        }
    }

    /// Check if this change affects rows
    pub fn affects_rows(&self) -> bool {
        matches!(self.dimension, Dimension::Row { .. })
    }

    /// Check if this change affects columns
    pub fn affects_columns(&self) -> bool {
        matches!(self.dimension, Dimension::Column { .. })
    }

    /// Check if this is an insertion operation
    pub fn is_insert(&self) -> bool {
        matches!(self.operation, HeaderOperation::Insert)
    }

    /// Check if this is a deletion operation
    pub fn is_delete(&self) -> bool {
        matches!(self.operation, HeaderOperation::Delete)
    }
}

/// Structural changes to worksheets, rows, columns
#[derive(Clone, PartialEq, Debug)]
pub enum StructureEvent {
    /// Workbook loaded into the model.
    WorkbookSwitched {
        from_uuid: Option<String>,
        to_uuid: String,
    },
    WorkbookDeleted {
        uuid: String,
    },
    WorkbookCreated {
        uuid: String,
        name: String,
    },
    WorksheetAdded {
        sheet: u32,
        name: String,
    },
    WorksheetDeleted {
        sheet: u32,
    },
    WorksheetRenamed {
        sheet: u32,
        old_name: String,
        new_name: String,
    },
    #[allow(dead_code)]
    WorksheetsReordered,
    /// Rows or columns inserted/deleted
    StructureChanged(HeaderChange),
    WorksheetHidden {
        sheet: u32,
    },
    WorksheetUnhidden {
        sheet: u32,
        name: String,
    },
    /// A column was moved to a new position.
    ColumnMoved {
        sheet: u32,
        from_col: i32,
        to_col: i32,
    },
    /// A row was moved to a new position.
    RowMoved {
        sheet: u32,
        from_row: i32,
        to_row: i32,
    },
    /// Frozen pane configuration changed.
    FreezeChanged {
        sheet: u32,
        frozen_rows: i32,
        frozen_cols: i32,
    },
}

impl StructureEvent {
    pub fn rows_inserted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::insert_rows(location))
    }

    pub fn rows_deleted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::delete_rows(location))
    }

    pub fn columns_inserted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::insert_columns(location))
    }

    pub fn columns_deleted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::delete_columns(location))
    }
}

/// Selection, navigation, and editing state changes.
#[derive(Clone, PartialEq, Debug)]
pub enum NavigationEvent {
    /// The active cell moved to a single new address.
    SelectionChanged { address: CellAddress },
    /// A range selection was extended (Shift-click, Shift-arrow, column/row header click).
    SelectionRangeChanged { sheet_area: SheetArea },
    /// The canvas viewport scrolled to a new `top_row` / `left_col` origin.
    ViewportScrolled {
        sheet: u32,
        top_row: i32,
        left_col: i32,
    },
    /// The active sheet changed.
    ActiveSheetChanged { from_sheet: u32, to_sheet: u32 },
    /// A cell edit session started (formula bar focused or printable key pressed).
    EditingStarted { address: CellAddress },
    /// A cell edit session ended. `committed = true` when the value was written to the model.
    EditingEnded {
        address: CellAddress,
        committed: bool,
    },
}

/// Theme and appearance changes.
#[derive(Clone, PartialEq, Debug)]
pub enum ThemeEvent {
    /// The active theme changed (Auto / Light / Dark cycle).
    ThemeToggled { new_theme: Theme },
    /// The color palette was modified.
    #[allow(dead_code)]
    PaletteUpdated,
    #[allow(dead_code)]
    LocaleChanged { new_locale: String },
}
