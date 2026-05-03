// JS-side model bridge

use ironcalc_base::types::CellType;
use ironcalc_base::types::Style;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::CanvasModel;
use crate::CanvasView;

#[wasm_bindgen]
extern "C" {
    pub type IronCalcModelHandle;
}

pub struct JsBackedModel {
    handle: IronCalcModelHandle,
}

impl JsBackedModel {
    pub fn new(handle: IronCalcModelHandle) -> Self {
        Self { handle }
    }

    pub fn from_js_value(value: JsValue) -> Self {
        Self::new(value.unchecked_into::<IronCalcModelHandle>())
    }
}

impl CanvasModel for JsBackedModel {
    fn get_selected_sheet(&self) -> u32 {
        let _ = &self.handle;
        todo!("bind IronCalcModelHandle.getSelectedSheet via wasm-bindgen extern method")
    }
    fn get_selected_view(&self) -> CanvasView {
        todo!("bind IronCalcModelHandle.getSelectedView; bridge field-by-field")
    }
    fn get_frozen_rows_count(&self, _sheet: u32) -> Result<i32, String> {
        todo!("bind IronCalcModelHandle.getFrozenRowsCount")
    }
    fn get_frozen_columns_count(&self, _sheet: u32) -> Result<i32, String> {
        todo!("bind IronCalcModelHandle.getFrozenColumnsCount")
    }
    fn get_row_height(&self, _sheet: u32, _row: i32) -> Result<f64, String> {
        todo!("bind IronCalcModelHandle.getRowHeight")
    }
    fn get_column_width(&self, _sheet: u32, _column: i32) -> Result<f64, String> {
        todo!("bind IronCalcModelHandle.getColumnWidth")
    }
    fn get_show_grid_lines(&self, _sheet: u32) -> Result<bool, String> {
        todo!("bind IronCalcModelHandle.getShowGridLines")
    }
    fn get_cell_style(&self, _sheet: u32, _row: i32, _column: i32) -> Result<Style, String> {
        todo!("bind IronCalcModelHandle.getCellStyle; needs serde or per-field bridge")
    }
    fn get_cell_type(&self, _sheet: u32, _row: i32, _column: i32) -> Result<CellType, String> {
        todo!("bind IronCalcModelHandle.getCellType; needs enum-tag bridge")
    }
    fn get_formatted_cell_value(
        &self,
        _sheet: u32,
        _row: i32,
        _column: i32,
    ) -> Result<String, String> {
        todo!("bind IronCalcModelHandle.getFormattedCellValue")
    }
}
