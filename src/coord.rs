//! Shared coordinate primitives for cell ranges and addresses.
//!
//! All indices are 1-based, matching ironcalc conventions.
//! ironcalc boundary types (`Area`, `ClipboardTuple`, `SelectedView.range`)
//! are converted at the edges via `to_ironcalc_area()`, `as_tuple()`, and
//! `From<[i32; 4]>` — they never leak past the `FrontendModel` trait.

use ironcalc_base::{expressions::types::Area, UserModel};

use ironcalc_base::expressions::parser::stringify::{to_localized_string, to_rc_format};
use ironcalc_base::expressions::parser::Node;
use ironcalc_base::expressions::types::CellReferenceRC;
use ironcalc_base::language::get_language;
use ironcalc_base::locale::get_locale;

use crate::model::ArrowKey;

/// Cell or range reference carried through point-mode state as an ironcalc
/// `Node`. Invariant: `inner` is `Node::ReferenceKind | Node::RangeKind`.
///
/// Construction goes through the `cell` / `range` / `from_cell_area`
/// factories; inner Node access is private so callers can't break the
/// invariant by mutating in place.
#[derive(Clone, Debug, PartialEq)]
pub struct RefNode {
    inner: Node,
}

impl RefNode {
    pub fn cell(
        sheet_index: u32,
        sheet_name: Option<String>,
        row: i32,
        column: i32,
        absolute_row: bool,
        absolute_column: bool,
    ) -> Self {
        Self {
            inner: Node::ReferenceKind {
                sheet_name,
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
            },
        }
    }

    pub fn range(
        sheet_index: u32,
        sheet_name: Option<String>,
        row1: i32,
        column1: i32,
        absolute_row1: bool,
        absolute_column1: bool,
        row2: i32,
        column2: i32,
        absolute_row2: bool,
        absolute_column2: bool,
    ) -> Self {
        Self {
            inner: Node::RangeKind {
                sheet_name,
                sheet_index,
                absolute_row1,
                absolute_column1,
                row1,
                column1,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
            },
        }
    }

    /// Promote a `SheetArea` into point-mode's carrier node.
    ///
    /// Two ironcalc Node encoding rules govern this conversion:
    ///
    /// 1. Relative-ref deltas. Ironcalc stores relative row/column as an
    ///    OFFSET from the stringify ctx, not an absolute coordinate. A
    ///    fresh point-mode click on `A1` with the edited cell at `B2`
    ///    stores `row = 1 - 2 = -1`, `column = 1 - 2 = -1`.
    ///
    /// 2. Sheet qualification. The stringifier emits a `Sheet!` prefix iff
    ///    `sheet_name` is `Some`. Same-sheet points must therefore carry
    ///    `None` — otherwise every bare `A1` click renders as `Sheet1!A1`.
    pub fn from_cell_area(
        area: SheetArea,
        editing: CellAddress,
        sheet_name_of_pointed: &str,
    ) -> Self {
        let sheet_name = if area.sheet == editing.sheet {
            None
        } else {
            Some(sheet_name_of_pointed.to_owned())
        };
        let inner = if area.area.is_single_cell() {
            Node::ReferenceKind {
                sheet_name,
                sheet_index: area.sheet,
                absolute_row: false,
                absolute_column: false,
                row: area.area.r1 - editing.row,
                column: area.area.c1 - editing.column,
            }
        } else {
            Node::RangeKind {
                sheet_name,
                sheet_index: area.sheet,
                absolute_row1: false,
                absolute_column1: false,
                row1: area.area.r1 - editing.row,
                column1: area.area.c1 - editing.column,
                absolute_row2: false,
                absolute_column2: false,
                row2: area.area.r2 - editing.row,
                column2: area.area.c2 - editing.column,
            }
        };
        Self { inner }
    }

    /// A1-style via ironcalc's canonical stringifier. `ctx` is the cell
    /// being edited — drives relative-offset math. Locale/language hard-
    /// coded to "en" until we surface them in AppState.
    pub fn to_localized(&self, ctx: &CellReferenceRC) -> String {
        let locale = get_locale("en").expect("builtin 'en' locale ships with ironcalc");
        let language = get_language("en").expect("builtin 'en' language ships with ironcalc");
        to_localized_string(&self.inner, ctx, locale, language)
    }

    pub fn to_rc(&self) -> String {
        to_rc_format(&self.inner)
    }

    /// Resolve the pointed-at cell(s) as a canonical `SheetArea` for overlay
    /// painting and viewport use.
    ///
    /// `editing` is the cell being edited — required because ironcalc stores
    /// relative row/column fields as *offsets* from the stringify ctx, not as
    /// absolute coordinates. For a `RangeKind`, each corner has its own
    /// absolute flags, so each coordinate must be resolved independently.
    ///
    /// Resolution rule per field:
    ///   absolute=true  → stored field already holds the absolute 1-based coord
    ///   absolute=false → absolute = stored + editing.{row|column}
    pub fn area(&self, editing: &CellAddress) -> SheetArea {
        match &self.inner {
            Node::ReferenceKind {
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
                ..
            } => {
                let r = if *absolute_row {
                    *row
                } else {
                    row + editing.row
                };
                let c = if *absolute_column {
                    *column
                } else {
                    column + editing.column
                };
                SheetArea::from_cell(*sheet_index, r, c)
            }
            Node::RangeKind {
                sheet_index,
                absolute_row1,
                absolute_column1,
                row1,
                column1,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
                ..
            } => {
                let r1 = if *absolute_row1 {
                    *row1
                } else {
                    row1 + editing.row
                };
                let c1 = if *absolute_column1 {
                    *column1
                } else {
                    column1 + editing.column
                };
                let r2 = if *absolute_row2 {
                    *row2
                } else {
                    row2 + editing.row
                };
                let c2 = if *absolute_column2 {
                    *column2
                } else {
                    column2 + editing.column
                };
                SheetArea::new(*sheet_index, r1, c1, r2, c2)
            }
            _ => unreachable!("RefNode invariant: inner is ReferenceKind or RangeKind"),
        }
    }

    /// Plain-arrow move — collapses to a single cell at the trailing corner
    /// plus the arrow delta.
    ///
    /// For a `ReferenceKind` (already one cell) this is just a move. For a
    /// `RangeKind`, the anchor is dropped and the result is a fresh
    /// `ReferenceKind` whose absolute flags are inherited from the trailing
    /// corner — matching Excel's "plain arrow forgets the range selection"
    /// behavior.
    pub fn extend_trailing(&self, key: &ArrowKey) -> Self {
        let (dr, dc) = key.delta();
        let inner = match &self.inner {
            Node::ReferenceKind {
                sheet_name,
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
            } => Node::ReferenceKind {
                sheet_name: sheet_name.clone(),
                sheet_index: *sheet_index,
                absolute_row: *absolute_row,
                absolute_column: *absolute_column,
                row: row + dr,
                column: column + dc,
            },
            Node::RangeKind {
                sheet_name,
                sheet_index,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
                ..
            } => Node::ReferenceKind {
                sheet_name: sheet_name.clone(),
                sheet_index: *sheet_index,
                absolute_row: *absolute_row2,
                absolute_column: *absolute_column2,
                row: row2 + dr,
                column: column2 + dc,
            },
            _ => unreachable!("RefNode invariant: inner is ReferenceKind or RangeKind"),
        };
        Self { inner }
    }

    /// Shift+arrow — anchor corner `(row1, column1)` stays fixed, trailing
    /// corner `(row2, column2)` moves by one cell in the arrow direction.
    ///
    /// A `ReferenceKind` (single cell) promotes to `RangeKind` with the
    /// original cell as both anchor AND trailing-before-move.
    pub fn extend_with_anchor(&self, key: &ArrowKey) -> Self {
        let (dr, dc) = key.delta();
        let inner = match &self.inner {
            Node::ReferenceKind {
                sheet_name,
                sheet_index,
                absolute_row,
                absolute_column,
                row,
                column,
            } => Node::RangeKind {
                sheet_name: sheet_name.clone(),
                sheet_index: *sheet_index,
                absolute_row1: *absolute_row,
                absolute_column1: *absolute_column,
                row1: *row,
                column1: *column,
                absolute_row2: *absolute_row,
                absolute_column2: *absolute_column,
                row2: *row + dr,
                column2: *column + dc,
            },
            Node::RangeKind {
                sheet_name,
                sheet_index,
                absolute_row1,
                absolute_column1,
                row1,
                column1,
                absolute_row2,
                absolute_column2,
                row2,
                column2,
            } => Node::RangeKind {
                sheet_name: sheet_name.clone(),
                sheet_index: *sheet_index,
                absolute_row1: *absolute_row1,
                absolute_column1: *absolute_column1,
                row1: *row1,
                column1: *column1,
                absolute_row2: *absolute_row2,
                absolute_column2: *absolute_column2,
                row2: *row2 + dr,
                column2: *column2 + dc,
            },
            _ => unreachable!("RefNode invariant: inner is ReferenceKind or RangeKind"),
        };
        Self { inner }
    }
}

/// Byte-offset span within a formula string, marking where the last point-mode
/// reference was spliced — so it can be replaced on the next arrow press or click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRef {
    pub start: usize,
    pub end: usize,
}

impl TextRef {
    /// A zero-length span at `cursor` — the "no previous span" fallback used
    /// when entering point mode for the first time at this formula position.
    pub fn at(cursor: usize) -> Self {
        Self {
            start: cursor,
            end: cursor,
        }
    }
}

// Point-mode move computation

/// Result of a successful point-mode arrow move.
#[derive(Debug, PartialEq)]
pub struct PointingStep {
    /// Formula text with the new reference spliced in.
    pub text: String,
    /// The new pointed-at reference — carries ironcalc's canonical Node form
    /// (absolute flags, sheet qualification) for downstream consumers.
    pub ref_node: RefNode,
    /// Byte span of the spliced reference in `text` (for `DragState::Pointing { ref_span }`).
    pub span: TextRef,
}

/// A cell or range referenced in an editing formula.
///
/// Produced by `formula_analysis::analyze_formula()` and consumed by the canvas
/// renderer to paint colored overlays over referenced cells.
///
/// `color_idx` is a sequential index into `theme::FORMULA_REF_COLORS`, assigned
/// by the parser in token order. The renderer resolves the actual color string —
/// keeping presentation out of the coordinate/analysis layer.
#[derive(Clone, Debug, PartialEq)]
pub struct FormulaRef {
    pub sheet_area: SheetArea,
    /// Sequential color slot (0-based). Renderer maps this to `FORMULA_REF_COLORS[idx % len]`.
    pub color_idx: usize,
    /// Byte span of this token in the formula string (for future cursor-aware
    /// per-token highlighting in the formula bar).
    pub span: TextRef,
}

/// A cell range pinned to a specific sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SheetArea {
    pub sheet: u32,
    pub area: CellArea,
}

impl SheetArea {
    pub fn new(sheet: u32, r1: i32, c1: i32, r2: i32, c2: i32) -> Self {
        Self {
            sheet,
            area: CellArea { r1, c1, r2, c2 },
        }
    }

    pub fn from_cell(sheet: u32, row: i32, col: i32) -> Self {
        Self {
            sheet,
            area: CellArea::from_cell(row, col),
        }
    }

    pub fn from_view(model: &UserModel) -> Self {
        Self {
            sheet: model.get_selected_sheet(),
            area: CellArea::from_view(model),
        }
    }

    // pub fn contains_address(self, addr: CellAddress) -> bool {
    //     if addr.sheet == self.sheet {
    //         return self.area.contains(addr.row, addr.column)
    //     }
    //     false
    // }

    pub fn on_same_sheet(self, other: SheetArea) -> bool {
        self.sheet == other.sheet
    }

    pub fn to_ironcalc_area(self) -> Area {
        self.area.to_area(self.sheet)
    }
}

/// Axis-aligned cell range. 1-based sheet coordinates.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CellArea {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

impl CellArea {
    pub fn height(self) -> i32 {
        self.r2 - self.r1 + 1
    }

    pub fn width(self) -> i32 {
        self.c2 - self.c1 + 1
    }
    pub fn is_single_cell(self) -> bool {
        self.r1 == self.r2 && self.c1 == self.c2
    }

    pub fn rows(self) -> std::ops::Range<i32> {
        self.r1..self.r2 + 1
    }
    pub fn columns(self) -> std::ops::Range<i32> {
        self.c1..self.c2 + 1
    }

    pub fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        self.rows()
            .flat_map(move |row| self.columns().map(move |col| (row, col)))
    }

    pub fn from_cell(row: i32, col: i32) -> Self {
        CellArea {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        }
    }

    pub fn contains(self, row: i32, col: i32) -> bool {
        (self.r1..=self.r2).contains(&row) && (self.c1..=self.c2).contains(&col)
    }

    pub fn normalized(self) -> Self {
        Self {
            r1: self.r1.min(self.r2),
            c1: self.c1.min(self.c2),
            r2: self.r1.max(self.r2),
            c2: self.c1.max(self.c2),
        }
    }

    pub fn with_sheet(self, sheet: u32) -> SheetArea {
        SheetArea { sheet, area: self }
    }

    pub fn from_view(model: &UserModel) -> Self {
        Self::from(model.get_selected_view().range)
    }

    /// Move trailing corner one step in the arrow direction. Anchor preserved. Clamps at 1.
    pub fn extend_trailing(self, key: &ArrowKey) -> Self {
        let (r2, c2) = match key {
            ArrowKey::Down => (self.r2 + 1, self.c2),
            ArrowKey::Up => ((self.r2 - 1).max(1), self.c2),
            ArrowKey::Left => (self.r2, (self.c2 - 1).max(1)),
            ArrowKey::Right => (self.r2, self.c2 + 1),
        };
        CellArea {
            r1: self.r1,
            c1: self.c1,
            r2,
            c2,
        }
    }

    /// Returns `(row_tiles, col_tiles)` if `src` tiles exactly into `self`,
    /// or `None` if any dimension has a remainder. A 1x1 source always tiles.
    pub fn tile_reps_of(self, src: CellArea) -> Option<(i32, i32)> {
        let row_reps = self.height() / src.height();
        let col_reps = self.width() / src.width();
        let fills_exactly =
            row_reps * src.height() == self.height() && col_reps * src.width() == self.width();
        let dst_is_larger = row_reps > 1 || col_reps > 1;
        (fills_exactly && dst_is_larger).then_some((row_reps, col_reps))
    }

    pub(crate) fn as_tuple(self) -> (i32, i32, i32, i32) {
        (self.r1, self.c1, self.r2, self.c2)
    }

    pub fn to_area(self, sheet: u32) -> Area {
        Area {
            sheet,
            row: self.r1,
            column: self.c1,
            height: self.r2 - self.r1 + 1,
            width: self.c2 - self.c1 + 1,
        }
    }
}

impl From<(i32, i32, i32, i32)> for CellArea {
    fn from((r1, c1, r2, c2): (i32, i32, i32, i32)) -> Self {
        Self { r1, c1, r2, c2 }
    }
}

impl From<[i32; 4]> for CellArea {
    fn from(range: [i32; 4]) -> Self {
        Self {
            r1: range[0],
            c1: range[1],
            r2: range[2],
            c2: range[3],
        }
    }
}

impl From<CellArea> for [i32; 4] {
    fn from(a: CellArea) -> Self {
        [a.r1, a.c1, a.r2, a.c2]
    }
}

/// Single cell position on a sheet. 1-based indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellAddress {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
}

impl CellAddress {
    pub fn from_view(model: &UserModel<'static>) -> Self {
        let m = model.get_selected_view();
        Self {
            sheet: m.sheet,
            row: m.row,
            column: m.column,
        }
    }

    #[allow(dead_code)]
    pub fn on_sheet(self, sheet: u32) -> bool {
        self.sheet == sheet
    }

    pub fn to_sheet_area(self) -> SheetArea {
        SheetArea {
            sheet: self.sheet,
            area: CellArea::from_cell(self.row, self.column),
        }
    }

    /// Ironcalc stringify ctx anchored at this cell. Sheet name is empty —
    /// for same-sheet point-mode, Nodes carry `sheet_name: None` and the
    /// stringifier ignores `ctx.sheet`. Cross-sheet point-mode will need a
    /// real sheet name threaded through.
    pub fn as_stringify_ctx(self) -> CellReferenceRC {
        CellReferenceRC {
            sheet: String::new(),
            row: self.row,
            column: self.column,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_includes_corners() {
        let a = CellArea {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 3,
        };
        assert!(a.contains(1, 1), "top-left");
        assert!(a.contains(3, 3), "bottom-right");
        assert!(!a.contains(4, 1), "outside");
    }

    #[test]
    fn contains_single_cell_area() {
        let a = CellArea::from_cell(5, 7);
        assert!(a.contains(5, 7));
        assert!(!a.contains(5, 8));
    }

    #[test]
    fn normalized_swaps_inverted_coords() {
        let a = CellArea {
            r1: 4,
            c1: 3,
            r2: 1,
            c2: 1,
        };
        assert_eq!(
            a.normalized(),
            CellArea {
                r1: 1,
                c1: 1,
                r2: 4,
                c2: 3
            }
        );
    }

    #[test]
    fn to_sheet_area_produces_single_cell() {
        let addr = CellAddress {
            sheet: 2,
            row: 4,
            column: 6,
        };
        let sa = addr.to_sheet_area();
        assert_eq!(sa.sheet, 2);
        assert_eq!(sa.area, CellArea::from_cell(4, 6));
        assert!(sa.area.is_single_cell());
    }

    fn ctx_a1() -> CellReferenceRC {
        CellReferenceRC {
            sheet: "Sheet1".into(),
            row: 1,
            column: 1,
        }
    }

    fn editing_a1() -> CellAddress {
        CellAddress {
            sheet: 0,
            row: 1,
            column: 1,
        }
    }

    // Relative Node fields store offsets from ctx: zero-offset from A1 → "A1".
    #[test]
    fn refnode_a1_roundtrip() {
        let n = RefNode::cell(0, None, 0, 0, false, false);
        assert_eq!(n.to_localized(&ctx_a1()), "A1");
    }

    // Absolute Node fields store the final 1-based coordinate directly.
    #[test]
    fn refnode_absolute_a1_roundtrip() {
        let n = RefNode::cell(0, None, 1, 1, true, true);
        assert_eq!(n.to_localized(&ctx_a1()), "$A$1");
    }

    #[test]
    fn refnode_cross_sheet_range() {
        let n = RefNode::range(
            1,
            Some("Sheet2".into()),
            0,
            0,
            false,
            false,
            2,
            1,
            false,
            false,
        );
        assert_eq!(n.to_localized(&ctx_a1()), "Sheet2!A1:B3");
    }

    #[test]
    fn refnode_quoted_sheet_name() {
        let n = RefNode::cell(1, Some("Space Sheet".into()), 0, 0, false, false);
        assert_eq!(n.to_localized(&ctx_a1()), "'Space Sheet'!A1");
    }

    #[test]
    fn refnode_rc_format_absolute_is_r1c1() {
        let n = RefNode::cell(0, None, 1, 1, true, true);
        assert_eq!(n.to_rc(), "R1C1");
    }

    // Relative ref: stored fields are deltas; area() must add editing coords.
    #[test]
    fn refnode_area_relative_resolves_with_editing() {
        let n = RefNode::cell(3, None, 4, 6, false, false);
        let resolved = n.area(&editing_a1());
        assert_eq!(resolved, SheetArea::from_cell(3, 5, 7));
    }

    // Absolute ref: stored fields are already absolute; editing is ignored.
    #[test]
    fn refnode_area_absolute_ignores_editing() {
        let n = RefNode::cell(3, None, 5, 7, true, true);
        let editing_far_away = CellAddress {
            sheet: 0,
            row: 100,
            column: 100,
        };
        assert_eq!(n.area(&editing_far_away), SheetArea::from_cell(3, 5, 7));
    }

    // Range: each corner resolved independently via its own absolute flags.
    #[test]
    fn refnode_area_range_mixed_flags() {
        // Anchor absolute at A1, trailing relative at delta (2,1) from editing.
        let n = RefNode::range(2, None, 1, 1, true, true, 2, 1, false, false);
        let editing = CellAddress {
            sheet: 0,
            row: 1,
            column: 1,
        };
        assert_eq!(n.area(&editing), SheetArea::new(2, 1, 1, 3, 2));
    }

    #[test]
    fn from_cell_area_same_sheet_omits_name() {
        let area = SheetArea::from_cell(0, 1, 1);
        let n = RefNode::from_cell_area(area, editing_a1(), "Sheet1");
        assert_eq!(n.to_localized(&ctx_a1()), "A1");
    }

    #[test]
    fn from_cell_area_cross_sheet_qualifies() {
        let area = SheetArea::from_cell(1, 1, 1);
        let n = RefNode::from_cell_area(area, editing_a1(), "Sheet2");
        assert_eq!(n.to_localized(&ctx_a1()), "Sheet2!A1");
    }

    #[test]
    fn from_cell_area_relative_offset() {
        let area = SheetArea::from_cell(0, 5, 3);
        let editing = CellAddress {
            sheet: 0,
            row: 2,
            column: 2,
        };
        let ctx = CellReferenceRC {
            sheet: "Sheet1".into(),
            row: 2,
            column: 2,
        };
        let n = RefNode::from_cell_area(area, editing, "Sheet1");
        assert_eq!(n.to_localized(&ctx), "C5");
    }

    // Plain arrow: whole reference moves. Relative Node fields store deltas
    // from ctx A1, so +1 to the row field shifts the resolved coord one row
    // down — matching ironcalc's stringify semantics.
    #[test]
    fn extend_trailing_single_cell_arrow_down() {
        let n = RefNode::cell(0, None, 0, 0, false, false);
        let moved = n.extend_trailing(&ArrowKey::from_str("ArrowDown").unwrap());
        assert_eq!(moved.to_localized(&ctx_a1()), "A2");
    }

    // Absolute flags survive the shift; stored coord increments directly.
    #[test]
    fn extend_trailing_preserves_absolute() {
        let n = RefNode::cell(0, None, 1, 1, true, true);
        let moved = n.extend_trailing(&ArrowKey::from_str("ArrowDown").unwrap());
        assert_eq!(moved.to_localized(&ctx_a1()), "$A$2");
    }

    // Range variant: plain arrow drops the anchor, leaving a single cell at
    // trailing + delta. Matches Excel's "plain arrow forgets the range".
    #[test]
    fn extend_trailing_range_collapses_to_trailing() {
        let n = RefNode::range(0, None, 0, 0, false, false, 1, 1, false, false);
        let moved = n.extend_trailing(&ArrowKey::from_str("ArrowRight").unwrap());
        assert_eq!(moved.to_localized(&ctx_a1()), "C2");
    }

    // Sheet qualification is part of the preserved metadata.
    #[test]
    fn extend_trailing_preserves_sheet_name() {
        let n = RefNode::cell(1, Some("Sheet2".into()), 0, 0, false, false);
        let moved = n.extend_trailing(&ArrowKey::from_str("ArrowDown").unwrap());
        assert_eq!(moved.to_localized(&ctx_a1()), "Sheet2!A2");
    }

    // Shift+arrow on a single cell: anchor pinned at A1, trailing grows to A2.
    #[test]
    fn extend_with_anchor_promotes_single_cell() {
        let n = RefNode::cell(0, None, 0, 0, false, false);
        let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowDown").unwrap());
        assert_eq!(grown.to_localized(&ctx_a1()), "A1:A2");
    }

    // Absolute flags mirror from the source onto the promoted trailing corner.
    #[test]
    fn extend_with_anchor_mirrors_absolute_flags() {
        let n = RefNode::cell(0, None, 1, 1, true, true);
        let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowDown").unwrap());
        assert_eq!(grown.to_localized(&ctx_a1()), "$A$1:$A$2");
    }

    // Existing range: anchor pinned at B3, trailing extends from C4 → D4.
    #[test]
    fn extend_with_anchor_extends_range_trailing_corner() {
        let n = RefNode::range(0, None, 2, 1, false, false, 3, 2, false, false);
        let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowRight").unwrap());
        assert_eq!(grown.to_localized(&ctx_a1()), "B3:D4");
    }

    // Sheet qualification carries through promotion.
    #[test]
    fn extend_with_anchor_preserves_sheet_on_promotion() {
        let n = RefNode::cell(1, Some("Sheet2".into()), 0, 0, false, false);
        let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowDown").unwrap());
        assert_eq!(grown.to_localized(&ctx_a1()), "Sheet2!A1:A2");
    }
}
