//! Shared coordinate types — cell addresses, ranges, and formula references.
//!
//! All indices are 1-based, matching ironcalc conventions.  Pure data structs
//! and their inherent impls; `From` conversions live in `super::convert`.

use ironcalc_base::UserModel;
use ironcalc_base::expressions::parser::Node;
use ironcalc_base::expressions::parser::stringify::{to_localized_string, to_rc_format};
use ironcalc_base::expressions::types::CellReferenceRC;
use ironcalc_base::language::get_language;
use ironcalc_base::locale::get_locale;

pub use iron_canvas_core::types::coord::FormulaRefKind;

use crate::model::ArrowKey;

/// Cell or range reference carried through point-mode state as an ironcalc
/// `Node`. Invariant: `inner` is `Node::ReferenceKind | Node::RangeKind`.
///
/// Construction goes through the `cell` / `range` / `from_cell_area`
/// factories; inner Node access is private so callers can't break the
/// invariant by mutating in place.
#[derive(Clone, Debug, PartialEq)]
pub struct RefNode {
    pub(crate) inner: Node,
}

/// Named pair of absolute flags — prevents swapping row/column booleans
/// at call sites (`cell(…, true, false)` was ambiguous).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Absolute {
    pub row: bool,
    pub column: bool,
}

impl RefNode {
    pub fn cell(
        sheet_index: u32,
        sheet_name: Option<String>,
        row: i32,
        column: i32,
        absolute: Absolute,
    ) -> Self {
        Self {
            inner: Node::ReferenceKind {
                sheet_name,
                sheet_index,
                absolute_row: absolute.row,
                absolute_column: absolute.column,
                row,
                column,
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn range(
        sheet_index: u32,
        sheet_name: Option<String>,
        row1: i32,
        column1: i32,
        absolute1: Absolute,
        row2: i32,
        column2: i32,
        absolute2: Absolute,
    ) -> Self {
        Self {
            inner: Node::RangeKind {
                sheet_name,
                sheet_index,
                absolute_row1: absolute1.row,
                absolute_column1: absolute1.column,
                row1,
                column1,
                absolute_row2: absolute2.row,
                absolute_column2: absolute2.column,
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
        area: SheetRange,
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
        let locale = get_locale("en").unwrap_or_else(|_| panic!("builtin 'en' locale missing"));
        let language =
            get_language("en").unwrap_or_else(|_| panic!("builtin 'en' language missing"));
        to_localized_string(&self.inner, ctx, locale, language)
    }

    pub fn to_rc(&self) -> String {
        to_rc_format(&self.inner)
    }

    /// Rewrite this ref's coordinates to `new` while preserving the
    /// user-visible identity: sheet qualification (`Sheet!` prefix) and
    /// per-axis absolute (`$`) flags. The dragged ref's text must keep
    /// its `$A$1` / `Sheet2!` markup so the user's intent survives the
    /// drag.
    ///
    /// Encoding rules mirror [`Self::from_cell_area`]:
    /// - Relative axes store `coord - editing.*` so the stringifier emits
    ///   the address against the editing cell's RC ctx.
    /// - Absolute axes store the absolute 1-based coord unchanged.
    /// - `sheet_name` is preserved from `self`; cross-sheet drag isn't
    ///   supported, so `self.sheet_name.is_some()` implies the new ref
    ///   is on the same other sheet.
    ///
    /// Cell↔range transitions: a single-cell self promotes to `RangeKind`
    /// when `new` is multi-cell (duplicating both absolute flags onto the
    /// new endpoint); a range self collapses to `ReferenceKind` when `new`
    /// is a single cell (endpoint 1's flags win — TopLeft is the canonical
    /// surviving corner).
    pub fn with_area(&self, new: SheetRange, editing: CellAddress) -> Self {
        let encode =
            |abs: bool, coord: i32, base: i32| -> i32 { if abs { coord } else { coord - base } };
        let (sheet_name, abs_r1, abs_c1, abs_r2, abs_c2) = match &self.inner {
            Node::ReferenceKind {
                sheet_name,
                absolute_row,
                absolute_column,
                ..
            } => (
                sheet_name.clone(),
                *absolute_row,
                *absolute_column,
                *absolute_row,
                *absolute_column,
            ),
            Node::RangeKind {
                sheet_name,
                absolute_row1,
                absolute_column1,
                absolute_row2,
                absolute_column2,
                ..
            } => (
                sheet_name.clone(),
                *absolute_row1,
                *absolute_column1,
                *absolute_row2,
                *absolute_column2,
            ),
            _ => (None, false, false, false, false),
        };

        let a = new.area;
        let inner = if a.is_single_cell() {
            Node::ReferenceKind {
                sheet_name,
                sheet_index: new.sheet,
                absolute_row: abs_r1,
                absolute_column: abs_c1,
                row: encode(abs_r1, a.r1, editing.row),
                column: encode(abs_c1, a.c1, editing.column),
            }
        } else {
            Node::RangeKind {
                sheet_name,
                sheet_index: new.sheet,
                absolute_row1: abs_r1,
                absolute_column1: abs_c1,
                row1: encode(abs_r1, a.r1, editing.row),
                column1: encode(abs_c1, a.c1, editing.column),
                absolute_row2: abs_r2,
                absolute_column2: abs_c2,
                row2: encode(abs_r2, a.r2, editing.row),
                column2: encode(abs_c2, a.c2, editing.column),
            }
        };
        Self { inner }
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
    ///   absolute=true  -> stored field already holds the absolute 1-based coord
    ///   absolute=false -> absolute = stored + editing.{row|column}
    pub fn area(&self, editing: &CellAddress) -> SheetRange {
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
                SheetRange::from_cell(*sheet_index, r, c)
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
                SheetRange::new(*sheet_index, r1, c1, r2, c2)
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

    /// Produce a single-cell `RefNode` at absolute `(abs_row, abs_col)` on
    /// `view_sheet`, preserving this RefNode's `absolute_row` /
    /// `absolute_column` flags. A `RangeKind` collapses to a cell inheriting
    /// the trailing corner's flags — matches `extend_trailing`'s "click kills
    /// the range selection" semantics.
    ///
    /// This is the click-to-replace primitive: when the caret sits on
    /// `$A$1` and the user clicks B5, the result is `$B$5`. Flag inheritance
    /// is what makes it Excel-parity instead of "drop to bare relative".
    ///
    /// The clicked cell's sheet wins (Excel parity: replace means replace) —
    /// the old ref's qualification is NOT preserved. Qualification is
    /// re-derived against the edit origin, mirroring [`Self::from_cell_area`]:
    /// `Some(view_sheet_name)` iff `view_sheet != editing.sheet`.
    pub fn relocate_to(
        &self,
        abs_row: i32,
        abs_col: i32,
        editing: &CellAddress,
        view_sheet: u32,
        view_sheet_name: &str,
    ) -> Self {
        let (abs_r_flag, abs_c_flag) = match &self.inner {
            Node::ReferenceKind {
                absolute_row,
                absolute_column,
                ..
            } => (*absolute_row, *absolute_column),
            Node::RangeKind {
                absolute_row2,
                absolute_column2,
                ..
            } => (*absolute_row2, *absolute_column2),
            _ => unreachable!("RefNode invariant: inner is ReferenceKind or RangeKind"),
        };
        let sheet_name = if view_sheet == editing.sheet {
            None
        } else {
            Some(view_sheet_name.to_owned())
        };
        let row = if abs_r_flag {
            abs_row
        } else {
            abs_row - editing.row
        };
        let column = if abs_c_flag {
            abs_col
        } else {
            abs_col - editing.column
        };
        Self::cell(
            view_sheet,
            sheet_name,
            row,
            column,
            Absolute {
                row: abs_r_flag,
                column: abs_c_flag,
            },
        )
    }

    /// F4 toggle: advance this ref's absolute (`$`) flags one step through the
    /// Excel cycle, re-encoding coordinates against `editing` so the resolved
    /// target is unchanged.
    ///
    /// Cell cycle, as `(row_abs, col_abs)`:
    ///   `(F,F)` `A1` → `(T,T)` `$A$1` → `(T,F)` `A$1` → `(F,T)` `$A1` → `(F,F)`
    /// Range endpoints advance independently, preserving mixed endpoint states.
    ///
    /// Re-encoding is the trap: a relative axis stores `coord - editing`, an
    /// absolute axis stores the literal `coord` (mirroring [`Self::area`] /
    /// [`Self::with_area`]). Flipping a flag therefore must add or subtract
    /// `editing.{row|column}`, not just toggle the boolean.
    pub fn cycle_absolute(&self, editing: &CellAddress) -> Self {
        // Excel's cycle in (row_abs, col_abs): (F,F)→(T,T)→(T,F)→(F,T)→(F,F).
        let next = |row_abs: bool, col_abs: bool| match (row_abs, col_abs) {
            (false, false) => (true, true),
            (true, true) => (true, false),
            (true, false) => (false, true),
            (false, true) => (false, false),
        };
        // Re-encode an axis to the new flag: absolute stores the literal coord,
        // relative stores the offset from the editing cell.
        let encode = |abs: bool, coord: i32, base: i32| if abs { coord } else { coord - base };

        // Resolve to absolute coords first, then rebuild against the new flags.
        let resolved = self.area(editing).area;
        match &self.inner {
            Node::ReferenceKind {
                sheet_name,
                sheet_index,
                absolute_row,
                absolute_column,
                ..
            } => {
                let (row_abs, col_abs) = next(*absolute_row, *absolute_column);
                Self {
                    inner: Node::ReferenceKind {
                        sheet_name: sheet_name.clone(),
                        sheet_index: *sheet_index,
                        absolute_row: row_abs,
                        absolute_column: col_abs,
                        row: encode(row_abs, resolved.r1, editing.row),
                        column: encode(col_abs, resolved.c1, editing.column),
                    },
                }
            }
            Node::RangeKind {
                sheet_name,
                sheet_index,
                absolute_row1,
                absolute_column1,
                absolute_row2,
                absolute_column2,
                ..
            } => {
                let (row1_abs, col1_abs) = next(*absolute_row1, *absolute_column1);
                let (row2_abs, col2_abs) = next(*absolute_row2, *absolute_column2);
                Self {
                    inner: Node::RangeKind {
                        sheet_name: sheet_name.clone(),
                        sheet_index: *sheet_index,
                        absolute_row1: row1_abs,
                        absolute_column1: col1_abs,
                        row1: encode(row1_abs, resolved.r1, editing.row),
                        column1: encode(col1_abs, resolved.c1, editing.column),
                        absolute_row2: row2_abs,
                        absolute_column2: col2_abs,
                        row2: encode(row2_abs, resolved.r2, editing.row),
                        column2: encode(col2_abs, resolved.c2, editing.column),
                    },
                }
            }
            _ => unreachable!("RefNode invariant: inner is ReferenceKind or RangeKind"),
        }
    }
}

// A workbook- or sheet-scoped name that resolves to a formula.
//
// Wraps ironcalc's `DefinedNameS = (name, scope, formula)` tuple with named
// fields at our API boundary. The parser takes the tuple form —
// `into_ironcalc()` converts only at that single call site.
#[derive(Clone, Debug, PartialEq)]
pub struct DefinedName {
    pub name: String,
    /// `None` = workbook-scoped. `Some(sheet_index)` = visible only on that sheet.
    pub scope: Option<u32>,
    pub formula: String,
}

impl DefinedName {
    pub(crate) fn into_ironcalc(self) -> DefinedNameS {
        (self.name, self.scope, self.formula)
    }
}

use ironcalc_base::expressions::parser::DefinedNameS;

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
/// Produced by `formula_analysis::analyze_formula()`. Two consumers read it:
/// the canvas renderer paints colored overlays (needs `sheet_area` + `color_idx`),
/// and editing-grade features — "fix this ref", point-mode replacement, circular-
/// dep detection — read `ref_node` to preserve the user's `$`-prefix and sheet-
/// qualification intent through edits.
///
/// `sheet_area` is a precomputed projection of `ref_node` via `RefNode::area`,
/// cached at analysis time so per-frame paint does not re-resolve relative
/// offsets. `color_idx` is a sequential index into `theme::FORMULA_REF_COLORS`,
/// assigned in token order by reference identity (same target -> same slot).
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveRef {
    /// Full ironcalc Node identity — `ReferenceKind | RangeKind` with
    /// `absolute_row` / `absolute_column` / `sheet_name` preserved.
    pub ref_node: RefNode,
    /// Precomputed projection of `ref_node` for the renderer hot path.
    pub sheet_area: SheetRange,
    /// Sequential color slot (0-based). Renderer maps this to `FORMULA_REF_COLORS[idx % len]`.
    pub color_idx: usize,
    /// Byte span of this token in the formula string — drives cursor-aware
    /// per-token highlighting via `FormulaAnalysis::refs_at_cursor`.
    pub span: TextRef,
    /// Emission origin — `Direct` for `RefLeaf::Resolved`, `DefinedName`
    /// when a name expands to a Reference/Range. Drag-edit gating reads
    /// this via `matches!(kind, FormulaRefKind::Direct)`.
    pub kind: FormulaRefKind,
}

/// A cell range pinned to a specific sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SheetRange {
    pub sheet: u32,
    pub area: CellArea,
}

impl SheetRange {
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

    pub fn on_same_sheet(self, other: SheetRange) -> bool {
        self.sheet == other.sheet
    }

    pub fn to_ironcalc_area(self) -> Area {
        self.area.to_area(self.sheet)
    }
}

use ironcalc_base::expressions::types::Area;

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

    pub fn with_sheet(self, sheet: u32) -> SheetRange {
        SheetRange { sheet, area: self }
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

    pub fn to_sheet_area(self) -> SheetRange {
        SheetRange {
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

// ==============================================================================
// Selection → A1 text (for grid range-picking in drawers)
// ==============================================================================
//
// The CF and Named-Range drawers fill a range field from the live grid
// selection. Each consumer wants a different A1 shape, so we expose two
// formatters built on the same `RefNode` stringifier the formula bar uses —
// guaranteeing the output is exactly what the engine round-trips.

/// The current selection as a **sheet-relative** A1 string (`B2:D8`) — no
/// sheet prefix, no `$`. This is the shape a CF sqref wants.
///
/// We reuse point-mode's relative encoding: `from_cell_area` stores each
/// coordinate as an offset from the active cell, and `to_localized` against
/// that same anchor reconstructs the real address. Because the selection is
/// always on the active sheet, the node carries `sheet_name: None`, so no
/// `Sheet!` prefix is emitted.
pub fn selection_a1_relative(model: &UserModel<'static>) -> String {
    let selection = SheetRange::from_view(model);
    let active = CellAddress::from_view(model);
    RefNode::from_cell_area(selection, active, "").to_localized(&active.as_stringify_ctx())
}

/// The current selection as a **sheet-qualified absolute** A1 string
/// (`Sheet1!$B$2:$D$8`) — the shape a Named Range "refers to".
///
/// Absolute refs store the 1-based coordinate verbatim (no offset), so the
/// stringify anchor is irrelevant; `Some(sheet_name)` forces the `Sheet!`
/// qualifier.
pub fn selection_a1_qualified_absolute(model: &UserModel<'static>) -> String {
    let selection = SheetRange::from_view(model);
    let sheet_name = model
        .get_worksheets_properties()
        .get(selection.sheet as usize)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let abs = Absolute {
        row: true,
        column: true,
    };
    let area = selection.area;
    let node = if area.is_single_cell() {
        RefNode::cell(selection.sheet, Some(sheet_name), area.r1, area.c1, abs)
    } else {
        RefNode::range(
            selection.sheet,
            Some(sheet_name),
            area.r1,
            area.c1,
            abs,
            area.r2,
            area.c2,
            abs,
        )
    };
    node.to_localized(&CellAddress::from_view(model).as_stringify_ctx())
}

#[cfg(test)]
mod tests {
    use super::*;

    // F4 walks a single cell through all four `$`-states and back to the start.
    // The ref resolves to A1 throughout — only the markup changes — so the
    // editing-cell offset re-encoding must hold the target steady.
    #[test]
    fn f4_cycles_cell_through_four_states() {
        let editing = CellAddress {
            sheet: 0,
            row: 2,
            column: 2,
        };
        let ctx = editing.as_stringify_ctx();
        let r0 = RefNode::from_cell_area(SheetRange::from_cell(0, 1, 1), editing, "");
        assert_eq!(r0.to_localized(&ctx), "A1");

        let r1 = r0.cycle_absolute(&editing);
        assert_eq!(r1.to_localized(&ctx), "$A$1");

        let r2 = r1.cycle_absolute(&editing);
        assert_eq!(r2.to_localized(&ctx), "A$1");

        let r3 = r2.cycle_absolute(&editing);
        assert_eq!(r3.to_localized(&ctx), "$A1");

        let r4 = r3.cycle_absolute(&editing);
        assert_eq!(r4.to_localized(&ctx), "A1");
    }

    // A range cycles both corners together: A1:B2 → $A$1:$B$2.
    #[test]
    fn f4_cycles_range_both_endpoints() {
        let editing = CellAddress {
            sheet: 0,
            row: 1,
            column: 1,
        };
        let ctx = editing.as_stringify_ctx();
        let r0 = RefNode::from_cell_area(SheetRange::new(0, 1, 1, 2, 2), editing, "");
        assert_eq!(r0.to_localized(&ctx), "A1:B2");

        let r1 = r0.cycle_absolute(&editing);
        assert_eq!(r1.to_localized(&ctx), "$A$1:$B$2");
    }

    #[test]
    fn f4_cycles_mixed_range_endpoints_independently() {
        let editing = CellAddress {
            sheet: 0,
            row: 3,
            column: 3,
        };
        let ctx = editing.as_stringify_ctx();
        let r0 = RefNode::range(
            0,
            None,
            -2,
            -2,
            Absolute {
                row: false,
                column: false,
            },
            2,
            2,
            Absolute {
                row: true,
                column: true,
            },
        );
        assert_eq!(r0.to_localized(&ctx), "A1:$B$2");

        let r1 = r0.cycle_absolute(&editing);
        assert_eq!(r1.to_localized(&ctx), "$A$1:B$2");
        assert_eq!(r1.area(&editing), r0.area(&editing));
    }
}
