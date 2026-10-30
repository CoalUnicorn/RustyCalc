//! Type conversions between RustyCalc coordinate types and the iron-canvas /
//! ironcalc boundary types.

use iron_canvas_core::{CellCoord, scene_geometry::GridRange};
use ironcalc_base::expressions::parser::DefinedNameS;

use super::types::*;

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

// --- CellArea <-> scene GridRange ---

/// Scene geometry carries no sheet identity and requires ordered endpoints,
/// so a possibly-inverted `CellArea` is normalized on the way in.
impl From<CellArea> for GridRange {
    fn from(c: CellArea) -> Self {
        Self {
            first: CellCoord {
                row: c.r1.min(c.r2),
                col: c.c1.min(c.c2),
            },
            last: CellCoord {
                row: c.r1.max(c.r2),
                col: c.c1.max(c.c2),
            },
        }
    }
}

impl From<GridRange> for CellArea {
    fn from(g: GridRange) -> Self {
        Self {
            r1: g.first.row,
            c1: g.first.col,
            r2: g.last.row,
            c2: g.last.col,
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
