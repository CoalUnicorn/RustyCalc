//! `iron-canvas-datagrid` — engine-agnostic tabular model that renders
//! through iron-canvas with ZERO IronCalc. Runtime dep: iron-canvas-core.
mod canvas_model;
mod model;

pub use model::{Cell, Column, DataGrid, DataGridBuilder, SortDirection};
