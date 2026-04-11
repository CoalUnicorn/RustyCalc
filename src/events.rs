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

/// Per-category event signals. Each holds events from the most recent
/// `emit_event(s)` call — replaced (not appended) on each emit.
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

#[derive(Clone, PartialEq, Debug)]
pub enum SpreadsheetEvent {
    Content(ContentEvent),
    Format(FormatEvent),
    Structure(StructureEvent),
    Navigation(NavigationEvent),
    Theme(ThemeEvent),
}

#[derive(Clone, PartialEq, Debug)]
pub enum ContentEvent {
    /// `old_value`/`new_value` are `None` when unavailable at the call site.
    CellChanged {
        address: CellAddress,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    RangeChanged {
        sheet_area: SheetArea,
    },
    #[allow(dead_code)]
    FormulaChanged {
        address: CellAddress,
    },
    #[allow(dead_code)]
    CalculationUpdated {
        affected_sheets: Vec<u32>,
    },
    #[allow(dead_code)]
    NamedRangesChanged,
    GenericChange,
}

#[derive(Clone, PartialEq, Debug)]
pub enum FormatEvent {
    #[allow(dead_code)]
    CellStyleChanged {
        address: CellAddress,
    },
    RangeStyleChanged {
        area: SheetArea,
    },
    LayoutChanged {
        sheet: u32,
        col: Option<i32>,
        row: Option<i32>,
    },
    RecentColorsUpdated {
        colors: Vec<CssColor>,
    },
    #[allow(dead_code)]
    DocumentColorsChanged {
        colors: Vec<CssColor>,
    },
    #[allow(dead_code)]
    ConditionalFormattingChanged {
        sheet: u32,
    },
}

#[derive(Clone, PartialEq, Debug)]
pub enum Dimension {
    Row { start: Option<i32> },
    Column { start: Option<i32> },
}

/// Contiguous span of rows or columns on a sheet. 0-based sheet, 1-based start.
#[derive(Clone, PartialEq, Debug)]
pub struct Location {
    sheet: u32,
    start: i32,
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

#[derive(Clone, PartialEq, Debug)]
pub struct HeaderChange {
    pub sheet: u32,
    pub operation: HeaderOperation,
    pub dimension: Dimension,
    pub count: i32,
}

#[derive(Clone, PartialEq, Debug)]
pub enum HeaderOperation {
    Insert,
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

    pub fn start_position(&self) -> i32 {
        match &self.dimension {
            Dimension::Row { start } => start.unwrap_or(1),
            Dimension::Column { start } => start.unwrap_or(1),
        }
    }

    pub fn affects_rows(&self) -> bool {
        matches!(self.dimension, Dimension::Row { .. })
    }

    pub fn affects_columns(&self) -> bool {
        matches!(self.dimension, Dimension::Column { .. })
    }

    pub fn is_insert(&self) -> bool {
        matches!(self.operation, HeaderOperation::Insert)
    }

    pub fn is_delete(&self) -> bool {
        matches!(self.operation, HeaderOperation::Delete)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum StructureEvent {
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
    WorkbookGroupChanged {
        uuid: String,
    },
    WorkbookRenamed {
        uuid: String,
        old_name: String,
        new_name: String,
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
    StructureChanged(HeaderChange),
    WorksheetHidden {
        sheet: u32,
    },
    WorksheetUnhidden {
        sheet: u32,
        name: String,
    },
    ColumnMoved {
        sheet: u32,
        from_col: i32,
        to_col: i32,
    },
    RowMoved {
        sheet: u32,
        from_row: i32,
        to_row: i32,
    },
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

#[derive(Clone, PartialEq, Debug)]
pub enum NavigationEvent {
    SelectionChanged {
        address: CellAddress,
    },
    /// Shift-click, Shift-arrow, or header click extended the selection.
    SelectionRangeChanged {
        sheet_area: SheetArea,
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

#[derive(Clone, PartialEq, Debug)]
pub enum ThemeEvent {
    ThemeToggled {
        new_theme: Theme,
    },
    #[allow(dead_code)]
    PaletteUpdated,
    #[allow(dead_code)]
    LocaleChanged {
        new_locale: String,
    },
}
