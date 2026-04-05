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
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HeaderContextMenu {
    /// Column index (1-based).
    Column(i32),
    /// Row index (1-based).
    Row(i32),
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
    /// UI interaction modes
    Mode(ModeEvent),
    /// Theme and appearance settings
    Theme(ThemeEvent),
}

/// Cell content, formulas, and calculation results changed
#[allow(dead_code)]
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

#[allow(dead_code)]
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

    pub fn dbg_description(&self) -> String {
        match self {
            ContentEvent::CellChanged {
                address, new_value, ..
            } => format!(
                "Content::CellChanged S{}R{}C{} → {:?}",
                address.sheet, address.row, address.column, new_value
            ),
            ContentEvent::RangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            } => {
                format!("Content::RangeChanged S{sheet} {start_col}{start_row}:{end_col}{end_row}")
            }
            ContentEvent::FormulaChanged { address } => format!(
                "Content::FormulaChanged S{}R{}C{}",
                address.sheet, address.row, address.column
            ),
            ContentEvent::CalculationUpdated { affected_sheets } => format!(
                "Content::CalculationUpdated ({} sheets)",
                affected_sheets.len()
            ),
            ContentEvent::NamedRangesChanged => "Content::NamedRangesChanged".into(),
            ContentEvent::GenericChange => "Content::GenericChange".into(),
        }
    }
}

/// Visual formatting and styling changes
#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
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

#[allow(dead_code)]
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

    pub fn dbg_description(&self) -> String {
        match self {
            FormatEvent::CellStyleChanged { address } => format!(
                "Format::CellStyle S{}R{}C{}",
                address.sheet, address.row, address.column
            ),
            FormatEvent::RangeStyleChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            } => format!("Format::RangeStyle S{sheet} {start_col}{start_row}:{end_col}{end_row}"),
            FormatEvent::LayoutChanged { sheet, col, row } => {
                format!("Format::Layout S{sheet} col={col:?} row={row:?}")
            }
            FormatEvent::RecentColorsUpdated { colors } => {
                format!("Format::RecentColors ({} colors)", colors.len())
            }
            FormatEvent::DocumentColorsChanged { colors } => {
                format!("Format::DocColors ({} colors)", colors.len())
            }
            FormatEvent::ConditionalFormattingChanged { sheet } => {
                format!("Format::CondFmt S{sheet}")
            }
        }
    }
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

    //  ((r_min, r_max), (c_min, c_max))
    // pub fn from_selecton_bounds(
    //     sheet: u32,
    //     header: Origin,
    //     bounds: ((i32, i32), (i32, i32)),
    // ) -> Self {
    //     match header {
    //         Origin::Row { .. } => {
    //             let ((start, count), _) = bounds;
    //             Self::new(sheet, start, count - start + 1)
    //         }
    //         Origin::Column { .. } => {
    //             let (_, (start, count)) = bounds;
    //             Self::new(sheet, start, count - start + 1)
    //         }
    //     }
    // }

    // pub fn for_insert(sheet: u32, header: Origin, bounds: ((i32, i32), (i32, i32))) -> Self {
    //     match header {
    //         Origin::Row { .. } => {
    //             let ((start, count), _) = bounds;
    //             Self::new(sheet, start, count - start + 1)
    //         }
    //         Origin::Column { .. } => {
    //             let (_, (start, count)) = bounds;
    //             Self::new(sheet, start, count - start + 1)
    //         }
    //     }
    // }
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
            Dimension::Row { start } => start.expect("Row impossible"),
            Dimension::Column { start } => start.expect("Column impossible"),
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
#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
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
    /// A sheet was hidden. It still exists — not deleted. Use `WorksheetUnhidden` to reverse.
    WorksheetHidden { sheet: u32 },
    /// A previously hidden sheet was made visible again.
    WorksheetUnhidden { sheet: u32, name: String },
}

#[allow(dead_code)]
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
            StructureEvent::WorksheetHidden { sheet } => Some(*sheet),
            StructureEvent::WorksheetUnhidden { sheet, .. } => Some(*sheet),
        }
    }

    pub fn dbg_description(&self) -> String {
        match self {
            StructureEvent::WorksheetAdded { sheet, name } => {
                format!("Structure::SheetAdded S{sheet} {name:?}")
            }
            StructureEvent::WorksheetDeleted { sheet } => {
                format!("Structure::SheetDeleted S{sheet}")
            }
            StructureEvent::WorksheetRenamed {
                sheet,
                old_name,
                new_name,
            } => format!("Structure::SheetRenamed S{sheet} {old_name:?}→{new_name:?}"),
            StructureEvent::WorksheetsReordered => "Structure::Reordered".into(),
            StructureEvent::StructureChanged(c) => format!(
                "Structure::Changed S{} {:?} {:?}",
                c.sheet, c.operation, c.dimension
            ),
            StructureEvent::WorksheetHidden { sheet } => {
                format!("Structure::Hidden(sheet={sheet})")
            }
            StructureEvent::WorksheetUnhidden { sheet, name } => {
                format!("Structure::Unhidden(sheet={sheet}, name={name})")
            }
        }
    }
}

/// Selection, navigation, and editing state changes.
#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub enum NavigationEvent {
    /// The active cell moved to a single new address.
    SelectionChanged { address: CellAddress },
    /// A range selection was extended (Shift-click, Shift-arrow, column/row header click).
    SelectionRangeChanged {
        sheet: u32,
        start_row: i32,
        start_col: i32,
        end_row: i32,
        end_col: i32,
    },
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

#[allow(dead_code)]
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

    pub fn dbg_description(&self) -> String {
        match self {
            NavigationEvent::SelectionChanged { address } => format!(
                "Nav::Selection S{}R{}C{}",
                address.sheet, address.row, address.column
            ),
            NavigationEvent::SelectionRangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            } => format!("Nav::RangeSelect S{sheet} {start_col}{start_row}:{end_col}{end_row}"),
            NavigationEvent::ViewportScrolled {
                sheet,
                top_row,
                left_col,
            } => format!("Nav::Scroll S{sheet} top={top_row} left={left_col}"),
            NavigationEvent::ActiveSheetChanged {
                from_sheet,
                to_sheet,
            } => format!("Nav::SheetSwitch S{from_sheet}→S{to_sheet}"),
            NavigationEvent::EditingStarted { address } => format!(
                "Nav::EditStart S{}R{}C{}",
                address.sheet, address.row, address.column
            ),
            NavigationEvent::EditingEnded { address, committed } => format!(
                "Nav::EditEnd S{}R{}C{} committed={committed}",
                address.sheet, address.row, address.column
            ),
        }
    }
}

/// UI interaction mode changes.
#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub enum ModeEvent {
    /// A cell edit session started.
    EditStarted { address: CellAddress },
    /// A cell edit session ended (committed or cancelled).
    EditEnded,
    /// The active [`DragState`] transitioned between modes.
    DragModeChanged {
        from_mode: DragState,
        to_mode: DragState,
    },
    /// Formula point-mode entered or exited.
    ///
    /// `range` is `[r1, c1, r2, c2]` when a range is currently highlighted.
    PointModeChanged {
        active: bool,
        range: Option<[i32; 4]>,
    },
    /// A header context menu was shown or hidden.
    ContextMenuToggled {
        visible: bool,
        target: Option<HeaderContextMenu>,
    },
    /// A modal dialog was shown or hidden.
    DialogToggled { dialog_name: String, visible: bool },
    /// A UI panel was shown or hidden.
    PanelToggled { panel_name: String, visible: bool },
}

impl ModeEvent {
    pub fn dbg_description(&self) -> String {
        match self {
            ModeEvent::EditStarted { address } => format!(
                "Mode::EditStarted S{}R{}C{}",
                address.sheet, address.row, address.column
            ),
            ModeEvent::EditEnded => "Mode::EditEnded".into(),
            ModeEvent::DragModeChanged { from_mode, to_mode } => {
                format!("Mode::Drag {from_mode:?}→{to_mode:?}")
            }
            ModeEvent::PointModeChanged { active, .. } => format!("Mode::Point active={active}"),
            ModeEvent::ContextMenuToggled { visible, target } => {
                format!("Mode::CtxMenu visible={visible} target={target:?}")
            }
            ModeEvent::DialogToggled {
                dialog_name,
                visible,
            } => format!("Mode::Dialog {dialog_name:?} visible={visible}"),
            ModeEvent::PanelToggled {
                panel_name,
                visible,
            } => format!("Mode::Panel {panel_name:?} visible={visible}"),
        }
    }
}

/// Theme and appearance changes.
#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub enum ThemeEvent {
    /// The active theme changed (Auto / Light / Dark cycle).
    ThemeToggled { new_theme: Theme },
    /// The color palette was modified.
    PaletteUpdated,
    /// FIXME: This needs its own place — language/locale changed.
    LocaleChanged { new_locale: String },
}

impl ThemeEvent {
    pub fn dbg_description(&self) -> String {
        match self {
            ThemeEvent::ThemeToggled { new_theme } => format!("Theme::Toggled {new_theme:?}"),
            ThemeEvent::PaletteUpdated => "Theme::PaletteUpdated".into(),
            ThemeEvent::LocaleChanged { new_locale } => format!("Theme::Locale {new_locale:?}"),
        }
    }
}

#[allow(dead_code)]
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
            // Calculation can touch multiple sheets - check the set
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
    // sd '#\[derive\(([^)]+)\)\]' '#[derive($1, Debug, Debug)]' src/events.rs
    // ```
    // Remove
    // ```sh
    // sd ',\s*Debug' '' src/canvas/renderer.rs
    // ```
    pub fn dbg_description(&self) -> String {
        match self {
            SpreadsheetEvent::Content(e) => e.dbg_description(),
            SpreadsheetEvent::Format(e) => e.dbg_description(),
            SpreadsheetEvent::Navigation(e) => e.dbg_description(),
            SpreadsheetEvent::Structure(e) => e.dbg_description(),
            SpreadsheetEvent::Mode(e) => e.dbg_description(),
            SpreadsheetEvent::Theme(e) => e.dbg_description(),
        }
    }
}

// NOTE: Check if still worth the macro
/// Event subscription filters for components
#[allow(dead_code)]
pub trait EventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool;
}

/// Filter for format-related events
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct FormatEventFilter;

impl EventFilter for FormatEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        matches!(event, SpreadsheetEvent::Format(_))
    }
}

/// Filter for theme-related events
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ThemeEventFilter;

impl EventFilter for ThemeEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        matches!(event, SpreadsheetEvent::Theme(_))
    }
}

/// Filter for content changes that affect calculations
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ContentEventFilter;

impl EventFilter for ContentEventFilter {
    fn matches(&self, event: &SpreadsheetEvent) -> bool {
        matches!(event, SpreadsheetEvent::Content(_))
    }
}

/// Filter for events affecting a specific sheet
#[derive(Clone, Debug)]
#[allow(dead_code)]
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
