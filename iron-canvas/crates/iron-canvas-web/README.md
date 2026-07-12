# iron-canvas-web

The `#[wasm_bindgen]` facade for IronCalc spreadsheets — the full spreadsheet canvas crate.

## What it does

Exposes `IronCanvas` to JavaScript: bind it to an IronCalc-compatible model handle and a pair of canvases, and it renders a live spreadsheet with selection, marching ants, formula-reference overlays, autofill handles, and frozen panes. It owns the repaint lifecycle (`requestRepaint` / `markContentDirty` / `markRowsDamaged` / `paintIfDirty`), always supports SVG export, and exposes PDF export when built with its `pdf` feature.

## Crate role

The primary shippable artifact for spreadsheet consumers: RustyCalc's Leptos frontend and browser hosts that provide an IronCalc-compatible JS model. It composes the core renderer, Canvas2D backend, IronCalc type adapter, and export backends. It re-exports the core API and Canvas2D types, while export types remain in `iron-canvas-export`. Built as `cdylib` + `rlib`.

## Key exports

- `IronCanvas` (`#[wasm_bindgen]`) — JS API:
  - **lifecycle**: `create`, `setModel`, `resize`, `dispose`
  - **paint**: `requestRepaint`, `markContentDirty`, `markRowsDamaged`, `paintIfDirty`
  - **theme**: `set_theme_name`, `setThemeFromElement`, `themeChanged`
  - **export**: `exportSvg`; `exportPdf` with feature `pdf`
  - **queries**: `hitTest`, `cellRect`, `resizeHandleAt`, `autofillHandlePos`
- Rust-only overlay/hit-test API: `set_extend_to`, `set_clipboard`, `set_point_range`, `set_formula_refs`, `set_overlays`, `hit_test`, `cell_rect`, `resize_handle_at`, `autofill_handle`, `canvas_size`

## Dependencies

- `iron-canvas-core`, `iron-canvas-canvas2d`, `iron-canvas-export` — renderer, paint, export
- `iron-canvas-ironcalc`, `ironcalc_base` — IronCalc conversion and model types
- `wasm-bindgen`, `web-sys` — JS bindings
- optional: `iron-canvas-recorder` + `serde_json` (feature `dev-tools`)

## Feature flags

- `dev-tools` — enables recording + replay via `iron-canvas-recorder`
- `pdf` — exposes `IronCanvas.exportPdf`; export crate types are not re-exported

## Relationship to sibling crates

The spreadsheet counterpart to `iron-canvas-datagrid-web`. Both are wasm facades over the same core renderer and Canvas2D backend; this one bridges an external IronCalc model handle and exposes the richer spreadsheet overlay/formula surface, while datagrid-web owns a plain in-memory table.
