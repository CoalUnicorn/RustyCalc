// CanvasModel - read-only worksheet surface the renderer consumes

use std::rc::Rc;

use crate::geometry::constants::{LAST_COLUMN, LAST_ROW};
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

    /// Conditional-formatting *decoration* for the cell, if any: an optional
    /// data-bar, icon-set, or rating. Despite the `_style` in the name this
    /// returns only the `CellDecoration` — the CF dxf *fill/font* overlay is
    /// delivered separately, already merged into the base style by
    /// `get_cell_styles_in`, not here. `Absent` when no CF decoration applies
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
    /// Returns `Vec<Fetched<T>>`, preserving the `Value`/`Absent`/`BridgeFailed`
    /// split that the single accessors carry. The pane preflight reads the split:
    /// a `BridgeFailed` slot means an in-flight bridge failure, so the frame
    /// holds prior pixels rather than painting the cell blank (symmetric with the
    /// single-cell active-cell repaint). `Absent` and `Value(empty)` still paint
    /// over `cell_bg` identically.
    ///
    /// Default impl loops the per-cell accessor, forwarding each `Fetched`
    /// verbatim; `JsBackedModel` overrides with one batched JS call per range.
    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_style(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch formatted cell values for `range` on `sheet`. Same dense
    /// row-major layout and `Vec<Fetched<T>>` rationale as `get_cell_styles_in`.
    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<String>>,
    ) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_formatted_cell_value(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch cell types for `range` on `sheet`. Same dense layout and
    /// `Vec<Fetched<T>>` rationale as `get_cell_styles_in`. Feeds the text
    /// pass's alignment/colour resolution in `CellTextStyle::resolve`.
    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_type(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch CF decorations for `range` on `sheet`. Same dense
    /// row-major layout and `Vec<Fetched<T>>` rationale as `get_cell_styles_in` —
    /// a per-slot `BridgeFailed` must reach the pane buffer distinctly from
    /// `Absent` so the preflight (and the fingerprint) can tell a transient
    /// bridge failure apart from a legitimately empty cell. Rides the same
    /// pane-cache / blit machinery so decorations stay aligned with
    /// styles/values/types across scrolls.
    fn get_cell_decorations_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<CellDecoration>>,
    ) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_extended_cell_style(sheet, r, c));
            }
        }
    }
}

/// Read-only worksheet surface the renderer consumes; every method is a pure
/// query against the host model. Extends [`CellContentQuery`] (the per-cell
/// content slice) with the sheet-level config and selection accessors.
///
/// `sheet` is an opaque view-scope discriminator, not a spreadsheet concept:
/// the engine echoes `get_selected_sheet()` back into the per-sheet accessors
/// so a multi-surface model can route the query, and equality-compares it
/// across frames (a changed value invalidates the cached frame). It is never
/// interpreted beyond that. Single-surface models (the datagrid impl, for
/// one) return a constant and ignore the parameter.
///
/// The `Option`-returning config/selection accessors use `None` for a transient
/// JS-bridge failure (the next animation frame re-queries), while the
/// `get_*_header_text` overrides use `None` for "no override, fall back to the
/// default."
pub trait CanvasModel: CellContentQuery {
    /// `None` signals a transient JS-bridge failure: the bridge call threw
    /// or the returned shape didn't deserialize. `FrameInputs::capture`
    /// holds the paint attempt rather than substituting sheet `0`.
    fn get_selected_sheet(&self) -> Option<u32>;
    /// `None` signals a transient JS-bridge failure: the bridge call threw
    /// or the returned shape didn't deserialize. The next animation frame
    /// will re-query.
    fn get_selected_view(&self) -> Option<CanvasView>;
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;

    /// Whether the selection (fill, stroke, autofill handle, active-cell
    /// overlay repaint, header highlights) should paint at all. Infallible
    /// and default-`true` — unlike the other accessors here, there is no
    /// "transient bridge failure" reading for this one, so it cannot itself
    /// hold a paint attempt.
    ///
    /// Exists so a deliberately selection-less host (the data-grid adapter
    /// with `show_selection(false)`) can signal that *without* overloading
    /// `get_selected_view() -> None`, which `FrameInputs::capture` treats as
    /// an unconditional hold — a selection-less grid would otherwise retry
    /// forever. The data-grid adapter overrides this and still returns a
    /// real `CanvasView` from `get_selected_view()`.
    fn get_show_selection(&self) -> bool {
        true
    }

    /// Last addressable row of `sheet`, 1-based inclusive. The slot walks,
    /// the blit-path rebuilds, and the autofill-handle guard clamp here.
    /// The default is Excel's bound; finite models override it so scroll
    /// extents end at their data.
    fn last_row(&self, _sheet: u32) -> i32 {
        LAST_ROW
    }

    /// Column mirror of [`Self::last_row`].
    fn last_column(&self, _sheet: u32) -> i32 {
        LAST_COLUMN
    }

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
        fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>);
        fn get_formatted_cell_values_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<String>>);
        fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>);
        fn get_cell_decorations_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellDecoration>>);
    }
}

impl<T: CanvasModel + ?Sized> CanvasModel for Rc<T> {
    forward_methods! {
        fn get_selected_sheet(&self) -> Option<u32>;
        fn get_selected_view(&self) -> Option<CanvasView>;
        fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
        fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
        fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
        fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
        fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;
        fn get_show_selection(&self) -> bool;
        fn last_row(&self, sheet: u32) -> i32;
        fn last_column(&self, sheet: u32) -> i32;
        fn get_show_row_headers(&self, sheet: u32) -> Option<bool>;
        fn get_show_col_headers(&self, sheet: u32) -> Option<bool>;
        fn get_row_header_text(&self, sheet: u32, row: i32) -> Option<String>;
        fn get_column_header_text(&self, sheet: u32, col: i32) -> Option<String>;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model whose single accessor returns a different `Fetched` variant per
    /// cell, so the default bulk loop's per-slot forwarding is observable.
    struct PerCellOutcomeModel;

    impl CellContentQuery for PerCellOutcomeModel {
        fn get_cell_style(&self, _s: u32, row: i32, col: i32) -> Fetched<CellStyle> {
            match (row, col) {
                (1, 1) => Fetched::BridgeFailed,
                (1, 2) => Fetched::Value(CellStyle::default()),
                _ => Fetched::Absent,
            }
        }
        fn get_cell_type(&self, _s: u32, _row: i32, _col: i32) -> Fetched<CellKind> {
            Fetched::Absent
        }
        fn get_formatted_cell_value(&self, _s: u32, _row: i32, _col: i32) -> Fetched<String> {
            Fetched::Absent
        }
        fn get_extended_cell_style(&self, _s: u32, row: i32, col: i32) -> Fetched<CellDecoration> {
            match (row, col) {
                (1, 1) => Fetched::BridgeFailed,
                (1, 2) => Fetched::Value(CellDecoration::Icon("ArrowUp".to_string())),
                _ => Fetched::Absent,
            }
        }
    }

    // Stage 1 (Fetched bulk channel): the default `get_cell_styles_in` loop must
    // forward each `Fetched` verbatim — a per-cell `BridgeFailed` reaches the
    // pane buffer as `BridgeFailed`, no longer `.value()`-collapsed to the same
    // `None` as a blank cell. This is what lets the Stage 2 preflight tell a
    // transient failure apart from an empty cell.
    #[test]
    fn default_bulk_styles_preserve_bridge_failed_per_slot() {
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };
        let mut out = Vec::new();
        PerCellOutcomeModel.get_cell_styles_in(0, range, &mut out);

        // Row-major: (1,1), (1,2), (2,1), (2,2).
        assert!(matches!(out[0], Fetched::BridgeFailed));
        assert!(matches!(out[1], Fetched::Value(_)));
        assert!(matches!(out[2], Fetched::Absent));
        assert!(matches!(out[3], Fetched::Absent));
    }

    // Acceptance 5: the default `get_cell_decorations_in` loop must forward
    // each `Fetched` verbatim, same as the style bulk path above — a
    // per-cell `BridgeFailed` must not collapse to `Absent`/`None` on its
    // way through the bulk decoration query.
    #[test]
    fn default_bulk_decorations_preserve_bridge_failed_per_slot() {
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };
        let mut out = Vec::new();
        PerCellOutcomeModel.get_cell_decorations_in(0, range, &mut out);

        // Row-major: (1,1), (1,2), (2,1), (2,2).
        assert!(matches!(out[0], Fetched::BridgeFailed));
        assert!(matches!(out[1], Fetched::Value(CellDecoration::Icon(_))));
        assert!(matches!(out[2], Fetched::Absent));
        assert!(matches!(out[3], Fetched::Absent));
    }
}
