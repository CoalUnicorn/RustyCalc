//! IronCalc engine adapter for `iron-canvas-core`.
//!
//! `IronCalcModel` is a newtype wrapper that implements `CanvasModel` for
//! `ironcalc_base::UserModel`.  Rust's orphan rule prevents implementing a
//! foreign trait (`CanvasModel`) for a foreign type (`UserModel`) outside of
//! the crate that defines the trait, so the direct impl lives here.  This is
//! the IronCalc-specific adapter: constructing an `IronCalcModel` requires
//! naming `ironcalc_base::UserModel`.  The engine-agnostic path is `DataGrid`.

pub mod convert;

use iron_canvas_core::{
    CanvasModel, CanvasView, CellContentQuery, CellDecoration, CellKind, CellStyle, Fetched,
    types::coord::RCRange,
};
use ironcalc_base::UserModel;

use crate::convert::{cell_decoration_from_extended, cell_type_to_kind, style_to_core};

/// Color resolver over a live `UserModel`: `resolve_color` borrows the
/// workbook theme, so resolving costs no theme clone per cell. Pass as
/// `&color_resolver(&model)` to the `convert` functions.
pub fn color_resolver<'a>(
    m: &'a UserModel<'_>,
) -> impl Fn(&ironcalc_base::types::Color) -> Option<String> + 'a {
    move |c| {
        let rgb = m.resolve_color(c);
        (!rgb.is_empty()).then_some(rgb)
    }
}

/// Newtype wrapper that implements `CanvasModel` for `UserModel`.
///
/// Derefs to `UserModel` so callers can still access IronCalc-specific
/// methods directly.  The `CanvasModel` impl is thin delegation.
pub struct IronCalcModel<'a>(pub UserModel<'a>);

impl<'a> std::ops::Deref for IronCalcModel<'a> {
    type Target = UserModel<'a>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> CanvasModel for IronCalcModel<'a> {
    fn get_selected_sheet(&self) -> u32 {
        UserModel::get_selected_sheet(&self.0)
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        let v = UserModel::get_selected_view(&self.0);
        Some(CanvasView {
            sheet: v.sheet,
            row: v.row,
            column: v.column,
            selection: RCRange {
                r1: v.range[0],
                c1: v.range[1],
                r2: v.range[2],
                c2: v.range[3],
            },
            top_row: v.top_row,
            left_column: v.left_column,
        })
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        UserModel::get_frozen_rows_count(&self.0, sheet).ok()
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        UserModel::get_frozen_columns_count(&self.0, sheet).ok()
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        UserModel::get_row_height(&self.0, sheet, row).ok()
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        UserModel::get_column_width(&self.0, sheet, column).ok()
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        UserModel::get_show_grid_lines(&self.0, sheet).ok()
    }
}

impl<'a> CellContentQuery for IronCalcModel<'a> {
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        // Use the dxf-MERGED style so the fingerprint hashes what is painted.
        // A native UserModel error is the only `None` source here, and it maps
        // to `Absent` — there is no JS bridge to fail.
        match UserModel::get_extended_cell_style(&self.0, sheet, row, column)
            .ok()
            .map(|ext| style_to_core(ext.style, &color_resolver(&self.0)))
        {
            Some(s) => Fetched::Value(s),
            None => Fetched::Absent,
        }
    }

    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind> {
        match UserModel::get_cell_type(&self.0, sheet, row, column)
            .ok()
            .map(cell_type_to_kind)
        {
            Some(k) => Fetched::Value(k),
            None => Fetched::Absent,
        }
    }

    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String> {
        match UserModel::get_formatted_cell_value(&self.0, sheet, row, column).ok() {
            Some(v) => Fetched::Value(v),
            None => Fetched::Absent,
        }
    }

    fn get_extended_cell_style(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Fetched<CellDecoration> {
        // Map IronCalc's icon/data_bar/rating to the core decoration. `Absent`
        // when the model errors or no decoration applies — the renderer draws
        // both the same.
        match UserModel::get_extended_cell_style(&self.0, sheet, row, column)
            .ok()
            .and_then(|ext| cell_decoration_from_extended(&ext, &color_resolver(&self.0)))
        {
            Some(d) => Fetched::Value(d),
            None => Fetched::Absent,
        }
    }

    // The bulk `*_in` accessors (styles, types, decorations) inherit the trait
    // default — a per-cell loop over the merged accessors above. There is no JS
    // boundary to amortise here, so an override would only re-spell the default.
}
