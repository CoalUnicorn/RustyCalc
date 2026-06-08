# iron-canvas-web

The `#[wasm_bindgen]` facade for IronCalc spreadsheets — the full spreadsheet canvas crate.

## What it does

Exposes `IronCanvas` to JavaScript: bind it to a workbook (`IronCalcModel`) and a pair of canvases, and it renders a live spreadsheet with selection, marching ants, formula-reference overlays, autofill handles, and frozen panes. It owns the repaint lifecycle (`requestRepaint` / `markContentDirty` / `paintIfDirty`) and can export the current view to SVG or PDF.

## Crate role

The primary shippable artifact for spreadsheet consumers — RustyCalc's Leptos frontend and any IronCalc web app. Composes `ironcalc` (data), `canvas2d` (paint), and `export` (SVG/PDF), and re-exports `core` + `canvas2d` + `export` so downstream Rust can reach the underlying types. Built as `cdylib` + `rlib`.

## Key exports

- `IronCanvas` (`#[wasm_bindgen]`) — JS API:
  - **lifecycle**: `create`, `setModel`, `resize`, `dispose`
  - **paint**: `requestRepaint`, `markContentDirty`, `paintIfDirty`
  - **theme**: `set_theme_name`, `setThemeFromElement`
  - **export**: `exportSvg`, `exportPdf`
- Rust-only overlay/hit-test API: `set_extend_to`, `set_clipboard`, `set_point_range`, `set_formula_refs`, `set_overlays`, `hit_test`, `cell_rect`, `resize_handle_at`, `autofill_handle`, `canvas_size`

## Dependencies

- `iron-canvas-core`, `iron-canvas-canvas2d`, `iron-canvas-export` — renderer, paint, export
- `ironcalc_base` (via `iron-canvas-ironcalc`) — the workbook engine
- `wasm-bindgen`, `web-sys` — JS bindings
- optional: `iron-canvas-recorder` + `serde_json` (feature `dev-tools`)

## Feature flags

- `dev-tools` — enables recording + replay via `iron-canvas-recorder`
- `pdf` — re-exports the `export` crate's PDF path

## Relationship to sibling crates

The spreadsheet counterpart to `iron-canvas-datagrid-web`. Both are wasm facades over the same core renderer and Canvas2D backend; this one carries the full IronCalc engine and the richer overlay/formula surface, where datagrid-web stays engine-free.
