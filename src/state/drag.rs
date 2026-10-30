//! Drag-mode enum: at most one pointer gesture is active at a time.

use crate::coord::{CellAddress, RefNode, SheetRange, TextRef};
use iron_canvas::RefZone;

/// Single enum ensures at most one drag mode is active — illegal
/// combinations (e.g. selecting while resizing) are unrepresentable.
///
/// `Pointing` carries an owned `RefNode` (non-Copy because its inner ironcalc
/// `Node` holds an `Option<String>` sheet name), so the enum is `Clone` only.
#[derive(Clone, Debug, PartialEq)]
pub enum DragState {
    /// No drag in progress.
    Idle,
    /// Mouse button held for a range-drag selection.
    Selecting,
    /// Autofill handle drag: the cell the user is dragging toward.
    Extending { to_row: i32, to_col: i32 },
    /// Column header resize. `col` is the dragged boundary's column; `span`
    /// is the inclusive `(first, last)` of columns resized together; `x` is
    /// the current mouse x.
    ResizingCol { col: i32, span: (i32, i32), x: f64 },
    /// Row header resize — mirror of `ResizingCol`.
    ResizingRow { row: i32, span: (i32, i32), y: f64 },
    /// Formula point-mode: carries ironcalc's canonical reference Node plus the
    /// byte span of its rendered form in the edited formula text.
    Pointing {
        ref_node: RefNode,
        ref_text: TextRef,
    },
    /// Drag a direct reference in the formula editor. `preview` drives the
    /// live overlay; `anchor` and `grab_cell` keep the drag math stable.
    DraggingFormulaRef {
        ref_idx: usize,
        zone: RefZone,
        anchor: SheetRange,
        grab_cell: CellAddress,
        preview: SheetRange,
    },
}
