// CanvasModel - read-only worksheet surface the renderer consumes

use std::rc::Rc;

use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasView {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
    pub selection: RCRange,
    pub top_row: i32,
    pub left_column: i32,
}

/// The cell-content slice of the model: per-cell style, type, value, and CF
/// decoration, plus their bulk (`*_in`) counterparts. Carved out as a
/// supertrait of [`CanvasModel`] so the hottest loop — the cell painter — can
/// state at the type level that it reads *cell content only*: its entry points
/// take `&dyn CellContentQuery`, a `&dyn CanvasModel` upcasting to it at the
/// call site (trait upcasting, stable since Rust 1.86).
///
/// The single accessors (`get_cell_style`, `get_cell_type`,
/// `get_formatted_cell_value`, `get_extended_cell_style`) return [`Fetched<T>`]
/// so a transient bridge failure (`BridgeFailed`) is distinct from a
/// legitimately empty cell (`Absent`). The bulk `*_in` accessors are mutually
/// closed — their defaults loop the single accessors — so the eight methods
/// move as one self-contained unit.
pub trait CellContentQuery {
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String>;

    /// Base cell style plus any conditional-formatting overlay: the CF dxf
    /// fill/font applied on top of the base style, plus optional data-bar,
    /// icon-set, and rating decorations. `Absent` when no CF decoration applies
    /// or when the model doesn't support CF (the renderer draws both the same).
    /// The default returns `Absent` so engines without CF support (and test
    /// stubs) compile unchanged.
    fn get_extended_cell_style(
        &self,
        _sheet: u32,
        _row: i32,
        _column: i32,
    ) -> Fetched<CellDecoration> {
        Fetched::Absent
    }

    /// Bulk-fetch cell styles for `range` on `sheet`. Output is dense,
    /// row-major: `out[(row - r1) * cols + (col - c1)]`.
    ///
    /// Returns `Vec<Option<T>>`, **not** `Vec<Fetched<T>>` like the single
    /// accessors. The pane cache consumes this as take-able scratch
    /// (`Option::take` per slot) and treats every `None` identically — a blank
    /// cell painted over the pre-filled `cell_bg`. No renderer site
    /// distinguishes `Absent` from `BridgeFailed` in bulk, so a `Fetched` here
    /// would ride the buffer unread; the actionable split lives at the fetch
    /// boundary instead (the single-cell active-cell repaint, and the wasm
    /// batch-trust gate).
    ///
    /// Default impl loops the per-cell accessor, collapsing each `Fetched` via
    /// `.value()`; `JsBackedModel` overrides with one batched JS call per range.
    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellStyle>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_style(sheet, r, c).value());
            }
        }
    }

    /// Bulk-fetch formatted cell values for `range` on `sheet`. Same dense
    /// row-major layout and `Vec<Option<T>>` rationale as `get_cell_styles_in`.
    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Option<String>>,
    ) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_formatted_cell_value(sheet, r, c).value());
            }
        }
    }

    /// Bulk-fetch cell types for `range` on `sheet`. Same dense layout and
    /// `Vec<Option<T>>` rationale as `get_cell_styles_in`. Feeds the text
    /// pass's alignment/colour resolution in `CellTextStyle::resolve`.
    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellKind>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_type(sheet, r, c).value());
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
                out.push(self.get_extended_cell_style(sheet, r, c).value());
            }
        }
    }
}

/// Read-only worksheet surface the renderer consumes; every method is a pure
/// query against the host model. Extends [`CellContentQuery`] (the per-cell
/// content slice) with the sheet-level config and selection accessors.
///
/// The `Option`-returning config/selection accessors use `None` for a transient
/// JS-bridge failure (the next animation frame re-queries), while the
/// `get_*_header_text` overrides use `None` for "no override, fall back to the
/// default."
pub trait CanvasModel: CellContentQuery {
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
}

/// Emits forwarding bodies that defer to `(**self).<method>(args)` for each
/// listed signature. Used by the `Rc<T>` blanket impls below — once per trait —
/// without hand-written shims that move together.
macro_rules! forward_methods {
    ($(fn $name:ident(&self $(, $arg:ident: $argty:ty)*) $(-> $ret:ty)?;)*) => {
        $(
            fn $name(&self, $($arg: $argty),*) $(-> $ret)? {
                (**self).$name($($arg),*)
            }
        )*
    };
}

/// Forwarding impls so an `Rc<M>` wrapping any `CanvasModel` is itself a
/// `CanvasModel`. The orchestrator stores `Rc<dyn CanvasModel>` and calls
/// through `Rc::as_ref`, so these are deref-convenience for callers that hold a
/// concrete `Rc<M>` — the `?Sized` arms also cover the `dyn` forms directly.
///
/// Two blocks because supertraits are not auto-derived: `Rc<T>: CanvasModel`
/// requires a real `Rc<T>: CellContentQuery` impl. The `CanvasModel` bound on
/// the second block implies `T: CellContentQuery`, satisfying the first.
impl<T: CellContentQuery + ?Sized> CellContentQuery for Rc<T> {
    forward_methods! {
        fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle>;
        fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind>;
        fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String>;
        fn get_extended_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellDecoration>;
        fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellStyle>>);
        fn get_formatted_cell_values_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<String>>);
        fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellKind>>);
        fn get_cell_decorations_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellDecoration>>);
    }
}

impl<T: CanvasModel + ?Sized> CanvasModel for Rc<T> {
    forward_methods! {
        fn get_selected_sheet(&self) -> u32;
        fn get_selected_view(&self) -> Option<CanvasView>;
        fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
        fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
        fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
        fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
        fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;
        fn get_show_row_headers(&self, sheet: u32) -> Option<bool>;
        fn get_show_col_headers(&self, sheet: u32) -> Option<bool>;
        fn get_row_header_text(&self, sheet: u32, row: i32) -> Option<String>;
        fn get_column_header_text(&self, sheet: u32, col: i32) -> Option<String>;
    }
}
