//! App host-event projection: one [`SpreadsheetEvent`] to one retained fact.
//!
//! [`summarize`] is the exhaustive mapping from the event bus vocabulary. It
//! has one arm per event variant and no wildcard arm, so adding a variant
//! fails the build here instead of silently recording `None`. The archive
//! calls it and owns everything about whether the fact is retained.

use serde::Serialize;

use crate::coord::{CellAddress, SheetRange};
use crate::events::{
    ContentEvent, FormatEvent, HeaderChange, NavigationEvent, SpreadsheetEvent, StructureEvent,
    ThemeEvent,
};
use iron_canvas_core::RCRange;

use super::records::{HostBatchId, SheetRef};

/// Category of one host event, mirroring the event bus categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HostBatchKind {
    Content,
    Format,
    Structure,
    Navigation,
    Theme,
}

impl HostBatchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Format => "format",
            Self::Structure => "structure",
            Self::Navigation => "navigation",
            Self::Theme => "theme",
        }
    }
}

/// What one host event pointed at.
///
/// Coordinates stay numeric and stay in the app's coordinate types. A1 text is
/// a view concern. Nothing here copies a value, a formula, a color list, or a
/// locale string — the scope and the kind are the whole record.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostScope {
    Cell(CellAddress),
    Range(SheetRange),
    Viewport {
        sheet: u32,
        top_row: i32,
        left_col: i32,
    },
    Sheet {
        sheet: u32,
    },
    Sheets {
        affected: Vec<u32>,
    },
    Layout {
        sheet: u32,
        row: Option<i32>,
        col: Option<i32>,
    },
    Header(HeaderChange),
    RowMoved {
        sheet: u32,
        from_row: i32,
        to_row: i32,
    },
    ColumnMoved {
        sheet: u32,
        from_col: i32,
        to_col: i32,
    },
    /// A color list collapsed to its length.
    Colors {
        count: usize,
    },
}

impl HostScope {
    /// The addressable extent of this scope in canvas coordinates.
    ///
    /// `None` for a scope that names no rectangle: a viewport, a whole sheet,
    /// a set of sheets, a layout change, a header change, a moved row or
    /// column, or a colour list. Coordinates stay numeric — A1 text is a view
    /// concern.
    pub fn range(&self) -> Option<RCRange> {
        match self {
            Self::Cell(cell) => Some(RCRange {
                r1: cell.row,
                c1: cell.column,
                r2: cell.row,
                c2: cell.column,
            }),
            Self::Range(range) => Some(RCRange {
                r1: range.area.r1,
                c1: range.area.c1,
                r2: range.area.r2,
                c2: range.area.c2,
            }),
            _ => None,
        }
    }
}

/// The facts one host event contributes to a batch summary.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchFacts {
    pub kind: HostBatchKind,
    /// `None` when the event has no addressable target.
    pub scope: Option<HostScope>,
    /// Sheet the event acted on, for name resolution at capture time.
    pub sheet: Option<u32>,
}

/// Summarize one host event.
///
/// One arm per [`SpreadsheetEvent`] variant with no wildcard arm, so adding a
/// variant fails the build here instead of silently recording `None`.
pub fn summarize(event: &SpreadsheetEvent) -> BatchFacts {
    let (kind, scope, sheet) = match event {
        SpreadsheetEvent::Content(event) => match event {
            ContentEvent::CellChanged { address, .. } => (
                HostBatchKind::Content,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            ContentEvent::RangeChanged { sheet_area } => (
                HostBatchKind::Content,
                Some(HostScope::Range(*sheet_area)),
                Some(sheet_area.sheet),
            ),
            ContentEvent::FormulaChanged { address } => (
                HostBatchKind::Content,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            ContentEvent::CalculationUpdated { affected_sheets } => (
                HostBatchKind::Content,
                Some(HostScope::Sheets {
                    affected: affected_sheets.clone(),
                }),
                None,
            ),
            ContentEvent::NamedRangesChanged => (HostBatchKind::Content, None, None),
        },
        SpreadsheetEvent::Format(event) => match event {
            FormatEvent::CellStyleChanged { address } => (
                HostBatchKind::Format,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            FormatEvent::RangeStyleChanged { area } => (
                HostBatchKind::Format,
                Some(HostScope::Range(*area)),
                Some(area.sheet),
            ),
            FormatEvent::LayoutChanged { sheet, col, row } => (
                HostBatchKind::Format,
                Some(HostScope::Layout {
                    sheet: *sheet,
                    row: *row,
                    col: *col,
                }),
                Some(*sheet),
            ),
            FormatEvent::RecentColorsUpdated { colors } => (
                HostBatchKind::Format,
                Some(HostScope::Colors {
                    count: colors.len(),
                }),
                None,
            ),
            FormatEvent::DocumentColorsChanged { colors } => (
                HostBatchKind::Format,
                Some(HostScope::Colors {
                    count: colors.len(),
                }),
                None,
            ),
            FormatEvent::ConditionalFormattingChanged { sheet } => (
                HostBatchKind::Format,
                Some(HostScope::Sheet { sheet: *sheet }),
                Some(*sheet),
            ),
        },
        SpreadsheetEvent::Structure(event) => match event {
            StructureEvent::WorksheetAdded { sheet, .. }
            | StructureEvent::WorksheetDeleted { sheet }
            | StructureEvent::WorksheetRenamed { sheet, .. }
            | StructureEvent::WorksheetHidden { sheet }
            | StructureEvent::WorksheetUnhidden { sheet, .. } => (
                HostBatchKind::Structure,
                Some(HostScope::Sheet { sheet: *sheet }),
                Some(*sheet),
            ),
            StructureEvent::WorksheetsReordered => (HostBatchKind::Structure, None, None),
            StructureEvent::StructureChanged(change) => (
                HostBatchKind::Structure,
                Some(HostScope::Header(change.clone())),
                Some(change.sheet),
            ),
            StructureEvent::ColumnMoved {
                sheet,
                from_col,
                to_col,
            } => (
                HostBatchKind::Structure,
                Some(HostScope::ColumnMoved {
                    sheet: *sheet,
                    from_col: *from_col,
                    to_col: *to_col,
                }),
                Some(*sheet),
            ),
            StructureEvent::RowMoved {
                sheet,
                from_row,
                to_row,
            } => (
                HostBatchKind::Structure,
                Some(HostScope::RowMoved {
                    sheet: *sheet,
                    from_row: *from_row,
                    to_row: *to_row,
                }),
                Some(*sheet),
            ),
            StructureEvent::DocumentReset => (HostBatchKind::Structure, None, None),
        },
        SpreadsheetEvent::Navigation(event) => match event {
            NavigationEvent::SelectionChanged { address } => (
                HostBatchKind::Navigation,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            NavigationEvent::SelectionRangeChanged { sheet_area } => (
                HostBatchKind::Navigation,
                Some(HostScope::Range(*sheet_area)),
                Some(sheet_area.sheet),
            ),
            NavigationEvent::ViewportScrolled {
                sheet,
                top_row,
                left_col,
            } => (
                HostBatchKind::Navigation,
                Some(HostScope::Viewport {
                    sheet: *sheet,
                    top_row: *top_row,
                    left_col: *left_col,
                }),
                Some(*sheet),
            ),
            NavigationEvent::ActiveSheetChanged {
                from_sheet,
                to_sheet,
            } => (
                HostBatchKind::Navigation,
                Some(HostScope::Sheets {
                    affected: vec![*from_sheet, *to_sheet],
                }),
                Some(*to_sheet),
            ),
            NavigationEvent::EditingStarted { address } => (
                HostBatchKind::Navigation,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            NavigationEvent::EditingEnded { address, .. } => (
                HostBatchKind::Navigation,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
        },
        SpreadsheetEvent::Theme(event) => match event {
            ThemeEvent::ThemeToggled { .. }
            | ThemeEvent::PaletteUpdated
            | ThemeEvent::LocaleChanged { .. } => (HostBatchKind::Theme, None, None),
        },
    };
    BatchFacts { kind, scope, sheet }
}

/// One retained host event, with the sheet identity resolved at capture time.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostBatchSummary {
    pub batch_id: HostBatchId,
    /// Position of this event inside its batch.
    pub index: u32,
    pub at_ms: f64,
    pub kind: HostBatchKind,
    pub scope: Option<HostScope>,
    /// Sheet identity for a scoped event, resolved at capture time.
    pub sheet: Option<SheetRef>,
}
