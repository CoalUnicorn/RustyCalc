# iron-canvas-datagrid

An engine-agnostic tabular data model: a plain in-memory table that iron-canvas can render.

## What it does

Holds rows, typed columns, and per-cell values/styles in memory, and implements `iron-canvas-core`'s `CanvasModel` so the renderer can paint it directly. No formulas, recalculation, or IronCalc. Supports column sizing/alignment and in-place sorting.

## Crate role

The lightweight data source for consumers who want a fast canvas grid without a spreadsheet engine. Depends only on `iron-canvas-core`; consumed by `iron-canvas-datagrid-web`.

## Key exports

- `DataGrid`: the table model and `CanvasModel` impl
- `DataGridModel`: interior-mutable wrapper for shared renderer ownership
- `DataGridBuilder`: fluent construction of columns + rows
- `Column { header, width, align }`: column definition
- `Cell { value, style }`: a single cell
- `SortDirection`: `Ascending` / `Descending` for `sort_by`

## Dependencies

- `iron-canvas-core` only. No IronCalc, no platform crates.

## Usage

```rust
let grid = DataGrid::builder()
    .column(Column::new("Name").width(160.0).align(HAlign::Left))
    .column(Column::new("Score").width(80.0).align(HAlign::Right))
    .row(vec!["Ada".into(), "99".into()])
    .build();
// grid: impl CanvasModel; hand it to an Orchestrator
```

## Relationship to sibling crates

The non-spreadsheet twin of `iron-canvas-ironcalc`: both satisfy `CanvasModel`, letting the identical core renderer drive either a live workbook or this plain table. `datagrid-web` wraps it in `#[wasm_bindgen]` for JS.
