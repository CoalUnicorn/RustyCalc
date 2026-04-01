/*!
# Domain-driven Event System

Replaces the generic `request_redraw()` counter with typed events that represent
actual changes in the spreadsheet domain. Components can subscribe to specific
event types, eliminating unnecessary re-renders.

## Architecture

Instead of all components responding to any change, we model domain events:
- **Content**: Cell values, formulas, calculations
- **Format**: Visual styling, colors, layout
- **Structure**: Sheets, rows, columns
- **Navigation**: Selection, scrolling, editing state
- **Mode**: UI interaction modes
- **Theme**: Appearance settings

Each event carries the minimal data needed to determine if a component should update.

## Usage

```rust
// Emit specific events instead of generic redraw
state.emit_event(SpreadsheetEvent::Format(
    FormatEvent::RecentColorsUpdated { colors: vec!["#ff0000".into()] }
));

// Components subscribe to event types they care about
let format_events = state.subscribe_to_format_events();
```
*/

use crate::{model::CellAddress, theme::Theme};

/// Domain-specific events that represent actual changes in the spreadsheet
#[derive(Clone, Debug, PartialEq)]
pub enum SpreadsheetEvent {
    /// Content of cells changed (values, formulas)
    Content(ContentEvent),
    /// Visual formatting changed (colors, fonts, layout)
    Format(FormatEvent),
    /// Structural changes (sheets, rows, columns)
    Structure(StructureEvent),
    /// Selection and navigation state
    Navigation(NavigationEvent),
    /// UI interaction modes
    Mode(ModeEvent),
    /// Theme and appearance settings
    Theme(ThemeEvent),
}

/// Cell content, formulas, and calculation results changed
#[derive(Clone, Debug, PartialEq)]
pub enum ContentEvent {
    /// A specific cell's value changed
    CellValueChanged { address: CellAddress },
    /// A specific cell's content changed (more detailed)
    CellChanged {
        address: CellAddress,
        old_value: String,
        new_value: String,
    },
    /// A range of cells changed (bulk operations)
    RangeChanged {
        sheet: u32,
        start_row: i32,
        start_col: i32,
        end_row: i32,
        end_col: i32,
    },
    /// Formula in a cell was modified
    FormulaChanged { address: CellAddress },
    /// Calculation chain updated (dependent cells recalculated)
    CalculationUpdated { affected_sheets: Vec<u32> },
    /// Named range definitions changed
    NamedRangesChanged,
    /// Generic content change (legacy compatibility)
    GenericChange,
}

/// Visual formatting and styling changes
#[derive(Clone, Debug, PartialEq)]
pub enum FormatEvent {
    /// Cell styling changed (font, colors, borders)
    CellStyleChanged { address: CellAddress },
    /// Range styling changed (bulk formatting)
    RangeStyleChanged {
        sheet: u32,
        start_row: i32,
        start_col: i32,
        end_row: i32,
        end_col: i32,
    },
    /// Column width or row height changed
    LayoutChanged {
        sheet: u32,
        col: Option<i32>,
        row: Option<i32>,
    },
    /// Recent colors list updated
    RecentColorsUpdated { colors: Vec<String> },
    /// Document colors extracted/changed
    DocumentColorsChanged { colors: Vec<String> },
    /// Conditional formatting rules changed
    ConditionalFormattingChanged { sheet: u32 },
}

/// The type of structural operation
#[derive(Clone, Debug, PartialEq)]
pub enum StructureOperation {
    Insert,
    Delete,
}

/// The dimension being modified
#[derive(Clone, Debug, PartialEq)]
pub enum Dimension {
    Row { start_row: i32 },
    Column { start_col: i32 },
}

/// A structural change to rows or columns
#[derive(Clone, Debug, PartialEq)]
pub struct StructureChange {
    pub sheet: u32,
    pub operation: StructureOperation,
    pub dimension: Dimension,
    pub count: i32,
}

impl StructureChange {
    /// Insert rows starting at the given row
    pub fn insert_rows(sheet: u32, start_row: i32, count: i32) -> Self {
        Self {
            sheet,
            operation: StructureOperation::Insert,
            dimension: Dimension::Row { start_row },
            count,
        }
    }

    /// Delete rows starting at the given row
    pub fn delete_rows(sheet: u32, start_row: i32, count: i32) -> Self {
        Self {
            sheet,
            operation: StructureOperation::Delete,
            dimension: Dimension::Row { start_row },
            count,
        }
    }

    /// Insert columns starting at the given column
    pub fn insert_columns(sheet: u32, start_col: i32, count: i32) -> Self {
        Self {
            sheet,
            operation: StructureOperation::Insert,
            dimension: Dimension::Column { start_col },
            count,
        }
    }

    /// Delete columns starting at the given column
    pub fn delete_columns(sheet: u32, start_col: i32, count: i32) -> Self {
        Self {
            sheet,
            operation: StructureOperation::Delete,
            dimension: Dimension::Column { start_col },
            count,
        }
    }

    /// Get the starting position (row or column index)
    pub fn start_position(&self) -> i32 {
        match &self.dimension {
            Dimension::Row { start_row } => *start_row,
            Dimension::Column { start_col } => *start_col,
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
        matches!(self.operation, StructureOperation::Insert)
    }

    /// Check if this is a deletion operation
    pub fn is_delete(&self) -> bool {
        matches!(self.operation, StructureOperation::Delete)
    }
}

/// Structural changes to worksheets, rows, columns
#[derive(Clone, Debug, PartialEq)]
pub enum StructureEvent {
    /// New worksheet added
    WorksheetAdded { sheet: u32, name: String },
    /// Worksheet deleted
    WorksheetDeleted { sheet: u32 },
    /// Worksheet renamed
    WorksheetRenamed {
        sheet: u32,
        old_name: String,
        new_name: String,
    },
    /// Worksheet reordered
    WorksheetsReordered,
    /// Rows or columns inserted/deleted
    StructureChanged(StructureChange),
}

impl StructureEvent {
    /// Convenience constructor for row insertion
    pub fn rows_inserted(sheet: u32, start_row: i32, count: i32) -> Self {
        Self::StructureChanged(StructureChange::insert_rows(sheet, start_row, count))
    }

    /// Convenience constructor for row deletion
    pub fn rows_deleted(sheet: u32, start_row: i32, count: i32) -> Self {
        Self::StructureChanged(StructureChange::delete_rows(sheet, start_row, count))
    }

    /// Convenience constructor for column insertion
    pub fn columns_inserted(sheet: u32, start_col: i32, count: i32) -> Self {
        Self::StructureChanged(StructureChange::insert_columns(sheet, start_col, count))
    }

    /// Convenience constructor for column deletion
    pub fn columns_deleted(sheet: u32, start_col: i32, count: i32) -> Self {
        Self::StructureChanged(StructureChange::delete_columns(sheet, start_col, count))
    }
}

/// Selection, navigation, and editing state changes
#[derive(Clone, Debug, PartialEq)]
pub enum NavigationEvent {
    /// Active selection changed
    SelectionChanged { address: CellAddress },
    /// Selection range changed (drag selection)
    SelectionRangeChanged {
        sheet: u32,
        start_row: i32,
        start_col: i32,
        end_row: i32,
        end_col: i32,
    },
    /// User scrolled the viewport
    ViewportScrolled {
        sheet: u32,
        top_row: i32,
        left_col: i32,
    },
    /// Active worksheet changed
    ActiveSheetChanged { from_sheet: u32, to_sheet: u32 },
    /// Cell editing started
    EditingStarted { address: CellAddress },
    /// Cell editing ended
    EditingEnded {
        address: CellAddress,
        committed: bool,
    },
}

/// UI interaction modes and tool states
#[derive(Clone, Debug, PartialEq)]
pub enum ModeEvent {
    /// Edit mode started for a specific cell
    EditStarted { address: CellAddress },
    /// Edit mode ended (commit or cancel)
    EditEnded,
    /// Drag mode changed (selection, resize, autofill, etc.)
    DragModeChanged {
        from_mode: crate::state::DragState,
        to_mode: crate::state::DragState,
    },
    /// Point mode during formula entry
    PointModeChanged {
        active: bool,
        range: Option<[i32; 4]>,
    },
    /// Context menu shown/hidden
    ContextMenuToggled {
        visible: bool,
        target: Option<crate::state::ContextMenuTarget>,
    },
    /// Modal dialog shown/hidden
    DialogToggled { dialog_name: String, visible: bool },
    /// Panel visibility changed
    PanelToggled { panel_name: String, visible: bool },
}

/// Theme and appearance changes
#[derive(Clone, Debug, PartialEq)]
pub enum ThemeEvent {
    /// Light/dark theme toggled
    ThemeToggled { new_theme: Theme },
    /// Color palette changed or updated
    PaletteUpdated,
    /// FIXE: This needs its own place Language/locale changed
    LocaleChanged { new_locale: String },
}

impl SpreadsheetEvent {
    /// Check if this event affects cell content
    pub fn affects_content(&self) -> bool {
        matches!(self, SpreadsheetEvent::Content(_))
    }

    /// Check if this event affects visual appearance
    pub fn affects_visual(&self) -> bool {
        matches!(
            self,
            SpreadsheetEvent::Format(_) | SpreadsheetEvent::Theme(_)
        )
    }

    /// Check if this event affects layout/structure
    pub fn affects_layout(&self) -> bool {
        matches!(
            self,
            SpreadsheetEvent::Structure(_)
                | SpreadsheetEvent::Format(FormatEvent::LayoutChanged { .. })
        )
    }

    /// Check if this event affects a specific sheet
    pub fn affects_sheet(&self, sheet: u32) -> bool {
        match self {
            // Events carrying a `sheet` field — compare directly
            SpreadsheetEvent::Content(ContentEvent::CellValueChanged {
                address: CellAddress { sheet: s, .. },
            })
            | SpreadsheetEvent::Content(ContentEvent::CellChanged {
                address: CellAddress { sheet: s, .. },
                ..
            })
            | SpreadsheetEvent::Content(ContentEvent::RangeChanged { sheet: s, .. })
            | SpreadsheetEvent::Content(ContentEvent::FormulaChanged {
                address: CellAddress { sheet: s, .. },
            })
            | SpreadsheetEvent::Format(FormatEvent::CellStyleChanged {
                address: CellAddress { sheet: s, .. },
            })
            | SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged { sheet: s, .. })
            | SpreadsheetEvent::Format(FormatEvent::LayoutChanged { sheet: s, .. })
            | SpreadsheetEvent::Format(FormatEvent::ConditionalFormattingChanged { sheet: s })
            | SpreadsheetEvent::Structure(StructureEvent::WorksheetAdded { sheet: s, .. })
            | SpreadsheetEvent::Structure(StructureEvent::WorksheetDeleted { sheet: s })
            | SpreadsheetEvent::Structure(StructureEvent::WorksheetRenamed { sheet: s, .. })
            | SpreadsheetEvent::Structure(StructureEvent::StructureChanged(StructureChange {
                sheet: s,
                ..
            }))
            | SpreadsheetEvent::Navigation(NavigationEvent::SelectionChanged {
                address: CellAddress { sheet: s, .. },
            })
            | SpreadsheetEvent::Navigation(NavigationEvent::SelectionRangeChanged {
                sheet: s,
                ..
            })
            | SpreadsheetEvent::Navigation(NavigationEvent::ViewportScrolled {
                sheet: s, ..
            })
            | SpreadsheetEvent::Navigation(NavigationEvent::EditingStarted {
                address: CellAddress { sheet: s, .. },
            })
            | SpreadsheetEvent::Navigation(NavigationEvent::EditingEnded {
                address: CellAddress { sheet: s, .. },
                ..
            }) => *s == sheet,

            // Calculation can touch multiple sheets — check the set
            SpreadsheetEvent::Content(ContentEvent::CalculationUpdated { affected_sheets }) => {
                affected_sheets.contains(&sheet)
            }

            // Sheet-switch: only the destination sheet is affected
            SpreadsheetEvent::Navigation(NavigationEvent::ActiveSheetChanged {
                to_sheet, ..
            }) => *to_sheet == sheet,

            // Legacy: treat as affecting all sheets
            SpreadsheetEvent::Content(ContentEvent::GenericChange) => true,

            // Sheet-agnostic events
            SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged)
            | SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated { .. })
            | SpreadsheetEvent::Format(FormatEvent::DocumentColorsChanged { .. })
            | SpreadsheetEvent::Structure(StructureEvent::WorksheetsReordered)
            | SpreadsheetEvent::Mode(_)
            | SpreadsheetEvent::Theme(_) => false,
        }
    }

    /// Get a human-readable description of the event (for debugging)
    pub fn description(&self) -> String {
        match self {
            SpreadsheetEvent::Content(ContentEvent::CellValueChanged { address }) => {
                format!(
                    "Cell value changed at {}!{}{}",
                    address.sheet, address.column, address.row
                )
            }
            SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }) => {
                format!("Range changed {sheet}!{start_col}{start_row}:{end_col}{end_row}")
            }
            SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated { colors }) => {
                format!("Recent colors updated ({} colors)", colors.len())
            }
            SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled { new_theme }) => {
                format!("Theme toggled to {:?}", new_theme)
            }
            // Add more as needed for debugging
            _ => format!("{:?}", self),
        }
    }
}

/// Event subscription filters for components
pub trait EventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool;
}

/// Filter for format-related events
#[derive(Clone)]
pub struct FormatEventFilter;

impl EventFilter for FormatEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        matches!(event, SpreadsheetEvent::Format(_))
    }
}

/// Filter for theme-related events
#[derive(Clone)]
pub struct ThemeEventFilter;

impl EventFilter for ThemeEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        matches!(event, SpreadsheetEvent::Theme(_))
    }
}

/// Filter for content changes that affect calculations
#[derive(Clone)]
pub struct ContentEventFilter;

impl EventFilter for ContentEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        matches!(event, SpreadsheetEvent::Content(_))
    }
}

/// Filter for events affecting a specific sheet
#[derive(Clone)]
pub struct SheetEventFilter {
    pub sheet: u32,
}

impl EventFilter for SheetEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        event.affects_sheet(self.sheet)
    }
}

/// Convenience macro for creating event filters
#[macro_export]
macro_rules! event_filter {
    (format) => {
        $crate::events::FormatEventFilter
    };
    (theme) => {
        $crate::events::ThemeEventFilter
    };
    (content) => {
        $crate::events::ContentEventFilter
    };
    (sheet $sheet:expr) => {
        $crate::events::SheetEventFilter { sheet: $sheet }
    };
}
