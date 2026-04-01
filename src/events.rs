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

/// Active mouse-drag interaction.
///
/// At most one drag mode can be active at a time. Using a single enum
/// instead of parallel `bool` / `Option` signals makes illegal combinations
/// (e.g. selecting while resizing) unrepresentable.
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
    /// Dragging to extend the point-mode range during formula entry.
    Pointing,
}

/// Which header was right-clicked to open the context menu.
#[derive(Clone, Copy, PartialEq)]
pub enum HeaderContextMenu {
    /// Column index (1-based).
    Column(i32),
    /// Row index (1-based).
    Row(i32),
}

/// Domain-specific events that represent actual changes in the spreadsheet
#[derive(Clone, PartialEq)]
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
#[derive(Clone, PartialEq)]
pub enum ContentEvent {
    /// A specific cell's content changed. `old_value`/`new_value` are `None`
    /// when the caller doesn't have the previous or next value available.
    CellChanged {
        address: CellAddress,
        old_value: Option<String>,
        new_value: Option<String>,
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

impl ContentEvent {
    pub fn affected_sheet(&self) -> Option<u32> {
        match self {
            ContentEvent::CellChanged { address, .. } => Some(address.sheet),
            ContentEvent::RangeChanged { sheet, .. } => Some(*sheet),
            ContentEvent::FormulaChanged { address } => Some(address.sheet),
            ContentEvent::CalculationUpdated { .. }
            | ContentEvent::NamedRangesChanged
            | ContentEvent::GenericChange => None,
        }
    }
}

/// Visual formatting and styling changes
#[derive(Clone, PartialEq)]
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

impl FormatEvent {
    pub fn affected_sheet(&self) -> Option<u32> {
        match self {
            FormatEvent::CellStyleChanged { address } => Some(address.sheet),
            FormatEvent::RangeStyleChanged { sheet, .. } => Some(*sheet),
            FormatEvent::LayoutChanged { sheet, .. } => Some(*sheet),
            FormatEvent::ConditionalFormattingChanged { sheet } => Some(*sheet),
            FormatEvent::RecentColorsUpdated { .. } | FormatEvent::DocumentColorsChanged { .. } => {
                None
            }
        }
    }
}

/// The type of header operation
#[derive(Clone, PartialEq)]
pub enum HeaderOperation {
    Insert,
    Delete,
}

/// The dimension being modified
#[derive(Clone, PartialEq)]
pub enum Dimension {
    Row { start_row: i32 },
    Column { start_col: i32 },
}

///
#[derive(Clone, PartialEq)]
pub struct Location {
    sheet: u32,
    start: i32,
    count: i32,
}

impl Location {
    
}

/// A structural change to rows or columns
#[derive(Clone, PartialEq)]
pub struct HeaderChange {
    pub sheet: u32,
    pub operation: HeaderOperation,
    pub dimension: Dimension,
    pub count: i32,
}

impl HeaderChange {
    fn rows(op: HeaderOperation, location: Location) -> Self {
        Self {
            sheet: location.sheet,
            operation: op,
            dimension: Dimension::Row {
                start_row: location.start,
            },
            count: location.count,
        }
    }

    fn columns(op: HeaderOperation, location: Location) -> Self {
        Self {
            sheet: location.sheet,
            operation: op,
            dimension: Dimension::Column {
                start_col: location.start,
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
        matches!(self.operation, HeaderOperation::Insert)
    }

    /// Check if this is a deletion operation
    pub fn is_delete(&self) -> bool {
        matches!(self.operation, HeaderOperation::Delete)
    }
}

/// Structural changes to worksheets, rows, columns
#[derive(Clone, PartialEq)]
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
    StructureChanged(HeaderChange),
}

impl StructureEvent {
    /// Convenience constructor for row insertion
    pub fn rows_inserted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::insert_rows(location))
    }

    /// Convenience constructor for row deletion
    pub fn rows_deleted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::delete_rows(location))
    }

    /// Convenience constructor for column insertion
    pub fn columns_inserted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::insert_columns(location))
    }

    /// Convenience constructor for column deletion
    pub fn columns_deleted(location: Location) -> Self {
        Self::StructureChanged(HeaderChange::delete_columns(location))
    }

    pub fn affected_sheet(&self) -> Option<u32> {
        match self {
            StructureEvent::WorksheetAdded { sheet, .. } => Some(*sheet),
            StructureEvent::WorksheetDeleted { sheet } => Some(*sheet),
            StructureEvent::WorksheetRenamed { sheet, .. } => Some(*sheet),
            StructureEvent::StructureChanged(c) => Some(c.sheet),
            StructureEvent::WorksheetsReordered => None,
        }
    }
}

/// Selection, navigation, and editing state changes
#[derive(Clone, PartialEq)]
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

impl NavigationEvent {
    pub fn affected_sheet(&self) -> Option<u32> {
        match self {
            NavigationEvent::SelectionChanged { address } => Some(address.sheet),
            NavigationEvent::SelectionRangeChanged { sheet, .. } => Some(*sheet),
            NavigationEvent::ViewportScrolled { sheet, .. } => Some(*sheet),
            // Only the destination sheet is considered affected by a sheet switch.
            NavigationEvent::ActiveSheetChanged { to_sheet, .. } => Some(*to_sheet),
            NavigationEvent::EditingStarted { address } => Some(address.sheet),
            NavigationEvent::EditingEnded { address, .. } => Some(address.sheet),
        }
    }
}

/// UI interaction modes and tool states
#[derive(Clone, PartialEq)]
pub enum ModeEvent {
    /// Edit mode started for a specific cell
    EditStarted { address: CellAddress },
    /// Edit mode ended (commit or cancel)
    EditEnded,
    /// Drag mode changed (selection, resize, autofill, etc.)
    DragModeChanged {
        from_mode: DragState,
        to_mode: DragState,
    },
    /// Point mode during formula entry
    PointModeChanged {
        active: bool,
        range: Option<[i32; 4]>,
    },
    /// Context menu shown/hidden
    ContextMenuToggled {
        visible: bool,
        target: Option<HeaderContextMenu>,
    },
    /// Modal dialog shown/hidden
    DialogToggled { dialog_name: String, visible: bool },
    /// Panel visibility changed
    PanelToggled { panel_name: String, visible: bool },
}

/// Theme and appearance changes
#[derive(Clone, PartialEq)]
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
            // Legacy: treat as affecting all sheets
            SpreadsheetEvent::Content(ContentEvent::GenericChange) => true,
            // Calculation can touch multiple sheets — check the set
            SpreadsheetEvent::Content(ContentEvent::CalculationUpdated { affected_sheets }) => {
                affected_sheets.contains(&sheet)
            }
            // Delegate to per-type method for all other sheet-specific events
            SpreadsheetEvent::Content(e) => e.affected_sheet() == Some(sheet),
            SpreadsheetEvent::Format(e) => e.affected_sheet() == Some(sheet),
            SpreadsheetEvent::Structure(e) => e.affected_sheet() == Some(sheet),
            SpreadsheetEvent::Navigation(e) => e.affected_sheet() == Some(sheet),
            // Sheet-agnostic
            SpreadsheetEvent::Mode(_) | SpreadsheetEvent::Theme(_) => false,
        }
    }

    // /// Get a human-readable description of the event (for debugging)
    // /// Before usage add derive Debug in this file to:
    //  ContextMenuHeader
    //  SpreadsheetEvent
    //  ContentEvent
    //  FormatEvent
    //  StructureOperation
    //  Dimension
    //  StructureChange
    //  StructureEvent
    //  NavigationEvent
    //  ModeEvent
    //  ThemeEvent
    //
    // /// And in `src/state.rs`
    // ContextMenuHeader
    //
    // bash for this file
    // Add
    // ```sh
    // sd '#\[derive\(([^)]+)\)\]' '#[derive($1, Debug)]' src/canvas/renderer.rs
    // ```
    // Remove
    // ```sh
    // sd ',\s*Debug' '' src/canvas/renderer.rs
    // ```
    // pub fn description(&self) -> String {
    //     match self {
    //         SpreadsheetEvent::Content(ContentEvent::CellValueChanged { address }) => {
    //             format!(
    //                 "Cell value changed at {}!{}{}",
    //                 address.sheet, address.column, address.row
    //             )
    //         }
    //         SpreadsheetEvent::Content(ContentEvent::RangeChanged {
    //             sheet,
    //             start_row,
    //             start_col,
    //             end_row,
    //             end_col,
    //         }) => {
    //             format!("Range changed {sheet}!{start_col}{start_row}:{end_col}{end_row}")
    //         }
    //         SpreadsheetEvent::Format(FormatEvent::RecentColorsUpdated { colors }) => {
    //             format!("Recent colors updated ({} colors)", colors.len())
    //         }
    //         SpreadsheetEvent::Theme(ThemeEvent::ThemeToggled { new_theme }) => {
    //             format!("Theme toggled to {:?}", new_theme)
    //         }
    //         // Add more as needed for debugging
    //         _ => format!("{:?}", self),
    //     }
    // }
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
