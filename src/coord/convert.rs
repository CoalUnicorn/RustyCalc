//! Type conversions between RustyCalc coordinate types and ironcalc /
//! iron-canvas types.

use iron_canvas_core::types::coord::{FormulaRef, RCRange, SheetArea};
use ironcalc_base::expressions::parser::DefinedNameS;

use super::types::*;

// --- ActiveRef -> FormulaRef ---

impl From<ActiveRef> for FormulaRef {
    fn from(a: ActiveRef) -> Self {
        Self {
            sheet_area: a.sheet_area.into(),
            color_idx: a.color_idx,
            kind: a.kind,
        }
    }
}

// --- SheetRange -> SheetArea ---

impl From<SheetRange> for SheetArea {
    fn from(s: SheetRange) -> Self {
        Self {
            sheet: s.sheet,
            range: s.area.into(),
        }
    }
}

// --- CellArea conversions ---

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

impl From<CellArea> for RCRange {
    fn from(c: CellArea) -> Self {
        Self {
            r1: c.r1,
            c1: c.c1,
            r2: c.r2,
            c2: c.c2,
        }
    }
}

// --- DefinedNameS -> DefinedName ---

impl From<DefinedNameS> for DefinedName {
    fn from((name, scope, formula): DefinedNameS) -> Self {
        Self {
            name,
            scope,
            formula,
        }
    }
}
