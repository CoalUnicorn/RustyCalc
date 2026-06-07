// CanvasModel - read-only worksheet surface the renderer consumes

use std::rc::Rc;

use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasView {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
    pub selection: RCRange,
    pub top_row: i32,
    pub left_column: i32,
}

pub trait CanvasModel {
    fn get_selected_sheet(&self) -> u32;
    /// `None` signals a transient JS-bridge failure: the bridge call threw
    /// or the returned shape didn't deserialize. The next animation frame
    /// will re-query.
    fn get_selected_view(&self) -> Option<CanvasView>;
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;

    /// Whether the row-header strip (1, 2, 3...) is visible on `sheet`.
    /// `Some(true)` default; `None` carries the same fetch-failed meaning
    /// as the other accessors. `false` collapses the strip to zero width.
    fn get_show_row_headers(&self, _sheet: u32) -> Option<bool> {
        Some(true)
    }

    /// Whether the column-header strip (A, B, C...) is visible on `sheet`.
    /// `Some(true)` default; `None` carries the same fetch-failed meaning
    /// as the other accessors. `false` collapses the strip to zero height.
    fn get_show_col_headers(&self, _sheet: u32) -> Option<bool> {
        Some(true)
    }

    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<CellStyle>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellKind>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String>;

    /// Base cell style plus any conditional-formatting overlay: the CF dxf
    /// fill/font applied on top of the base style, plus optional data-bar,
    /// icon-set, and rating decorations. `None` when no CF decoration applies
    /// or when the model doesn't support CF. The default returns `None` so
    /// engines without CF support (and test stubs) compile unchanged.
    fn get_extended_cell_style(
        &self,
        _sheet: u32,
        _row: i32,
        _column: i32,
    ) -> Option<CellDecoration> {
        None
    }

    /// Override text for a row header slot. `None` means use the default
    /// numeric label (1, 2, 3...). Implementations that don't support custom
    /// header text omit the override.
    fn get_row_header_text(&self, _sheet: u32, _row: i32) -> Option<String> {
        None
    }

    /// Override text for a column header slot. `None` means use the default
    /// alphabetic label (A, B, C...).
    fn get_column_header_text(&self, _sheet: u32, _col: i32) -> Option<String> {
        None
    }

    /// Bulk-fetch cell styles for `range` on `sheet`. Output is dense,
    /// row-major: `out[(row - r1) * cols + (col - c1)]`. `None` entries
    /// carry the same fetch-failed meaning as `get_cell_style`.
    ///
    /// Default impl loops the per-cell accessor so impls that don't override
    /// keep their existing behaviour; the wasm bridge overrides this with a
    /// single JS round-trip per range.
    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellStyle>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_style(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch formatted cell values for `range` on `sheet`. Same dense
    /// row-major layout and `None`-as-failure semantics as
    /// `get_cell_styles_in`; same default-impl / wasm-override pattern.
    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Option<String>>,
    ) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_formatted_cell_value(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch cell types for `range` on `sheet`. Same layout and
    /// semantics as the other `*_in` accessors. Feeds the text pass's
    /// alignment/colour resolution in `CellTextStyle::resolve`.
    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellKind>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_type(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch CF decorations for `range` on `sheet`. Same dense
    /// row-major layout and `None`-as-absent semantics as the other
    /// `*_in` accessors; rides the same pane-cache / blit machinery so
    /// decorations stay aligned with styles/values/types across scrolls.
    fn get_cell_decorations_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Option<CellDecoration>>,
    ) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_extended_cell_style(sheet, r, c));
            }
        }
    }
}

/// Emits forwarding bodies that defer to `(**self).<method>(args)` for each
/// listed signature. Used once below to populate the `Rc<T>` blanket impl
/// without hand-written shims that move together.
macro_rules! forward_canvas_model {
    ($(fn $name:ident(&self $(, $arg:ident: $argty:ty)*) $(-> $ret:ty)?;)*) => {
        $(
            fn $name(&self, $($arg: $argty),*) $(-> $ret)? {
                (**self).$name($($arg),*)
            }
        )*
    };
}

/// Forwarding impl so an `Rc<M>` wrapping any `CanvasModel` is itself a
/// `CanvasModel`. The orchestrator stores `Rc<dyn CanvasModel>` and
/// calls through `Rc::as_ref`, so this is deref-convenience for callers
/// that hold a concrete `Rc<M>` — the `?Sized` arm also covers
/// `Rc<dyn CanvasModel>` directly.
impl<T: CanvasModel + ?Sized> CanvasModel for Rc<T> {
    forward_canvas_model! {
        fn get_selected_sheet(&self) -> u32;
        fn get_selected_view(&self) -> Option<CanvasView>;
        fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
        fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
        fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
        fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
        fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;
        fn get_show_row_headers(&self, sheet: u32) -> Option<bool>;
        fn get_show_col_headers(&self, sheet: u32) -> Option<bool>;
        fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<CellStyle>;
        fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellKind>;
        fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String>;
        fn get_extended_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<CellDecoration>;
        fn get_row_header_text(&self, sheet: u32, row: i32) -> Option<String>;
        fn get_column_header_text(&self, sheet: u32, col: i32) -> Option<String>;
        fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellStyle>>);
        fn get_formatted_cell_values_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<String>>);
        fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellKind>>);
        fn get_cell_decorations_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellDecoration>>);
    }
}
