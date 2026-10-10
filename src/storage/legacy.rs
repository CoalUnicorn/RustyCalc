//! Decoder for RCAL v1, written by IronCalc 93f3407f.
//!
//! Keep field order fixed. Bitcode encodes fields by position.
//! Only the workbook and worksheet containers changed at this engine boundary.

use bitcode::Decode;
use ironcalc_base::cf_types::ConditionalFormatting;
use ironcalc_base::types::*;
use std::collections::HashMap;

#[derive(Decode)]
struct LegacyWorkbook {
    shared_strings: Vec<String>,
    defined_names: Vec<DefinedName>,
    worksheets: Vec<LegacyWorksheet>,
    styles: Styles,
    name: String,
    settings: WorkbookSettings,
    metadata: Metadata,
    tables: HashMap<String, Table>,
    views: HashMap<u32, WorkbookView>,
    theme: Theme,
}

#[derive(Decode)]
struct LegacyWorksheet {
    dimension: String,
    cols: Vec<Col>,
    rows: Vec<Row>,
    name: String,
    sheet_data: HashMap<i32, HashMap<i32, Cell>>,
    shared_formulas: Vec<String>,
    sheet_id: u32,
    state: SheetState,
    color: Color,
    merged_cells: Vec<MergedCell>,
    comments: Vec<Comment>,
    frozen_rows: i32,
    frozen_columns: i32,
    views: HashMap<u32, WorksheetView>,
    show_grid_lines: bool,
    conditional_formatting: Vec<ConditionalFormatting>,
    links: HashMap<(i32, i32), Link>,
}

pub(super) fn decode(bytes: &[u8]) -> Result<Workbook, String> {
    let old: LegacyWorkbook = bitcode::decode(bytes)
        .map_err(|error| format!("Cannot decode legacy workbook: {error}"))?;
    Ok(Workbook {
        shared_strings: old.shared_strings,
        defined_names: old.defined_names,
        worksheets: old.worksheets.into_iter().map(Worksheet::from).collect(),
        styles: old.styles,
        name: old.name,
        settings: old.settings,
        metadata: old.metadata,
        tables: old.tables,
        views: old.views,
        theme: old.theme,
    })
}

impl From<LegacyWorksheet> for Worksheet {
    fn from(old: LegacyWorksheet) -> Self {
        let mut sheet_data = SheetData::new();
        for (row, cells) in old.sheet_data {
            sheet_data.set_row(row, cells);
        }
        Self {
            dimension: old.dimension,
            cols: old.cols,
            rows: old.rows,
            name: old.name,
            sheet_data,
            shared_formulas: old.shared_formulas,
            sheet_id: old.sheet_id,
            state: old.state,
            color: old.color,
            merged_cells: old.merged_cells,
            comments: old.comments,
            frozen_rows: old.frozen_rows,
            frozen_columns: old.frozen_columns,
            views: old.views,
            show_grid_lines: old.show_grid_lines,
            conditional_formatting: old.conditional_formatting,
            links: old.links,
        }
    }
}
