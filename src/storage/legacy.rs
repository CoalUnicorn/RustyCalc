//! Decoders for RCAL v1 (IronCalc 93f3407f) and v2 (IronCalc 7e6ae21c).
//!
//! Keep field order fixed. Bitcode encodes fields by position.
//! Only the workbook and worksheet containers changed at this engine boundary.

use bitcode::Decode;
use ironcalc_base::cf_types::ConditionalFormatting;
use ironcalc_base::types::*;
use std::collections::HashMap;

// IronCalc no longer serializes views. These layouts belong to RCAL v1 and v2.
#[derive(Decode)]
struct LegacyWorkbookView {
    _sheet: u32,
    _window_width: i64,
    _window_height: i64,
}

#[derive(Decode)]
struct LegacyWorksheetView {
    _row: i32,
    _column: i32,
    _range: [i32; 4],
    _focus_row: i32,
    _focus_column: i32,
    _top_row: i32,
    _left_column: i32,
}

#[derive(Decode)]
struct LegacyWorkbook<C> {
    shared_strings: Vec<String>,
    defined_names: Vec<DefinedName>,
    worksheets: Vec<LegacyWorksheet<C>>,
    styles: Styles,
    name: String,
    settings: WorkbookSettings,
    metadata: Metadata,
    tables: HashMap<String, Table>,
    _views: HashMap<u32, LegacyWorkbookView>,
    theme: Theme,
}

#[derive(Decode)]
struct LegacyWorksheet<C> {
    dimension: String,
    cols: Vec<Col>,
    rows: Vec<Row>,
    name: String,
    sheet_data: C,
    shared_formulas: Vec<String>,
    sheet_id: u32,
    state: SheetState,
    color: Color,
    merged_cells: Vec<MergedCell>,
    comments: Vec<Comment>,
    frozen_rows: i32,
    frozen_columns: i32,
    _views: HashMap<u32, LegacyWorksheetView>,
    show_grid_lines: bool,
    conditional_formatting: Vec<ConditionalFormatting>,
    links: HashMap<(i32, i32), Link>,
}

#[derive(Decode)]
struct LegacySheetData(HashMap<i32, HashMap<i32, Cell>>);

pub(super) fn decode_v1(bytes: &[u8]) -> Result<Workbook, String> {
    bitcode::decode::<LegacyWorkbook<LegacySheetData>>(bytes)
        .map(Workbook::from)
        .map_err(|error| format!("Cannot decode RCAL v1 workbook: {error}"))
}

pub(super) fn decode_v2(bytes: &[u8]) -> Result<Workbook, String> {
    bitcode::decode::<LegacyWorkbook<SheetData>>(bytes)
        .map(Workbook::from)
        .map_err(|error| format!("Cannot decode RCAL v2 workbook: {error}"))
}

impl From<LegacySheetData> for SheetData {
    fn from(old: LegacySheetData) -> Self {
        let mut sheet_data = SheetData::new();
        for (row, cells) in old.0 {
            sheet_data.set_row(row, cells);
        }
        sheet_data
    }
}

impl<C: Into<SheetData>> From<LegacyWorkbook<C>> for Workbook {
    fn from(old: LegacyWorkbook<C>) -> Self {
        Self {
            shared_strings: old.shared_strings,
            defined_names: old.defined_names,
            worksheets: old.worksheets.into_iter().map(Worksheet::from).collect(),
            styles: old.styles,
            name: old.name,
            settings: old.settings,
            metadata: old.metadata,
            tables: old.tables,
            theme: old.theme,
        }
    }
}

impl<C: Into<SheetData>> From<LegacyWorksheet<C>> for Worksheet {
    fn from(old: LegacyWorksheet<C>) -> Self {
        Self {
            dimension: old.dimension,
            cols: old.cols,
            rows: old.rows,
            name: old.name,
            sheet_data: old.sheet_data.into(),
            shared_formulas: old.shared_formulas,
            sheet_id: old.sheet_id,
            state: old.state,
            color: old.color,
            merged_cells: old.merged_cells,
            comments: old.comments,
            frozen_rows: old.frozen_rows,
            frozen_columns: old.frozen_columns,
            show_grid_lines: old.show_grid_lines,
            conditional_formatting: old.conditional_formatting,
            links: old.links,
        }
    }
}
