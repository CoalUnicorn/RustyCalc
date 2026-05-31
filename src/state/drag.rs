//! Drag-mode enum and the formula-ref ghost-range overlay.

use iron_canvas_web::RefZone;

use crate::coord::{CellAddress, RefNode, SheetRange, TextRef};

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
    /// Formula-ref overlay drag. `anchor` is the ref's range at mousedown;
    /// `grab_cell` is the cell under the cursor at mousedown. Mousemove
    /// uses both to compute the new range per `zone` without frame-to-frame
    /// state.
    DraggingFormulaRef {
        ref_idx: usize,
        zone: RefZone,
        anchor: SheetRange,
        grab_cell: CellAddress,
    },
}

/// Live preview of a formula-ref drag: the ref index and the range the
/// cursor currently resolves to. Mousemove publishes this; the worksheet
/// memo patches `formula_refs[idx].sheet_area` with `range` so the painted
/// outline follows the cursor without rewriting the formula text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RefOverride {
    pub idx: usize,
    pub range: SheetRange,
}
