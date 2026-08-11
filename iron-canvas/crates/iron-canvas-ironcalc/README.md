# iron-canvas-ironcalc

The orphan-rule bridge — teaches iron-canvas how to read an IronCalc workbook.

## What it does

Adapts IronCalc's `UserModel` to `iron-canvas-core`'s `CanvasModel` trait through the local `IronCalcModel` newtype, and converts IronCalc's style/cell types into core's canvas types. Because both `CanvasModel` and `UserModel` are foreign to each other, a direct impl is illegal; the local newtype is what makes the adapter legal under Rust's orphan rule. It also hosts the conditional-formatting bridge: `get_extended_cell_style()` surfaces per-cell CF decorations (data bars, icon sets, ratings) to the paint pipeline as `CellDecoration`s, while color-scale effects arrive through the merged cell style.

## Crate role

The data-source adapter that makes the renderer spreadsheet-aware. Sits between `iron-canvas-core` and `ironcalc_base`; consumed by `iron-canvas-web`. This is the *only* crate besides `web` that pulls in `ironcalc_base`, keeping the engine dependency isolated.

## Key exports

- `IronCalcModel<'a>` — newtype over `UserModel<'a>` implementing `CanvasModel`; `Deref`s to `UserModel` so callers keep full workbook access
- `convert` (module) — `style_to_core`, `cell_type_to_kind`, `cell_decoration_from_extended`: IronCalc → core type mappers

## Dependencies

- `iron-canvas-core` — the `CanvasModel` trait and target types
- `ironcalc_base` (v0.8.3) — the workbook engine being adapted

## Relationship to sibling crates

The IronCalc-specific counterpart to `datagrid`: both implement `CanvasModel`, but this one wraps a live formula engine while `datagrid` wraps a plain in-memory table. Swapping which model the `Orchestrator` holds is what turns the same renderer from a spreadsheet into a data grid.
