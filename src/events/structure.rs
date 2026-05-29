//! Structure-domain events: sheets, rows, columns.
//!
//! The grammar of structural changes — `Dimension`, `Location`, `HeaderChange`,
//! `HeaderOperation` — lives with `StructureEvent` because it has no other
//! consumer. `StructureEvent::StructureChanged(HeaderChange)` is the only
//! payload that uses them.

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
    // FreezeChanged {
    //     sheet: u32,
    //     frozen_rows: i32,
    //     frozen_cols: i32,
    // },
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
