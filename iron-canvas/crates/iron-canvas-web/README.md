# iron-canvas-web

The `#[wasm_bindgen]` facade for IronCalc spreadsheets — the full spreadsheet canvas crate.

## What it does

Exposes `IronCanvas` to JavaScript: bind it to an IronCalc-compatible model handle and a pair of canvases, and it renders a live spreadsheet with selection, marching ants, formula-reference overlays, autofill handles, and frozen panes. It owns the repaint lifecycle (`requestRepaint` / `markContentDirty` / `markRowsDamaged` / `viewChanged` / `paintIfDirty`), always supports SVG export, and exposes PDF export when built with its `pdf` feature.

## Crate role

The primary shippable artifact for spreadsheet consumers: RustyCalc's Leptos frontend and browser hosts that provide an IronCalc-compatible JS model. It composes the core renderer, Canvas2D backend, IronCalc type adapter, and export backends. It re-exports the core API and Canvas2D types, while export types remain in `iron-canvas-export`. Built as `cdylib` + `rlib`.

## JS API — `IronCanvas`

Method names are camelCase, matching the IronCalc wasm API convention (snake_case stays on the Rust side). Payload setters return `Result<(), JsError>`; optional query results are `null` when absent.

- **lifecycle** — `create(gridCanvas, overlayCanvas)` (static), `setModel(model)`, `resize(cssW, cssH, dpr)`, `dispose()`
- **paint** — `requestRepaint`, `markContentDirty`, `markRowsDamaged(sheet, rowStart, rowEnd)`, `viewChanged`, `requestOverlayRepaint`, `paintIfDirty`, `fontsChanged`, `frameTrace`, `recordingSupported` (static)
  - `paintIfDirty` returns `JsPaintResult`: `Idle` / `Painted` / `Retry` / `Playback` — `Retry` means an attempt was held back and the caller should call again next frame with no new signal.
  - `recordingSupported` reports whether the `dev-tools` feature is compiled in, so hosts can hide their Record button on prod builds.
- **theme** — `setThemeName(name)` (`"dark"` recognized, everything else maps to light), `setThemeFromElement(el)` (reads `--palette-*` CSS vars), `setTheme(theme)` (full palette push, every field required), `setThemeVariables(vars)` (partial override with light fallback), `themeChanged()`
- **export** — `exportSvg(cssW, cssH)`; `exportPdf(cssW, cssH)` with feature `pdf`
- **queries** — `hitTest(x, y)`, `cellRect(row, column)`, `resizeHandleAt(x, y, tolerance)`, `autofillHandlePos()`, `pixelToCell(x, y)`, `canvasSize()`, `fitColumnWidth(column, firstRow, lastRow)`
  - Shapes: `hitTest` → object tagged on `kind`; `cellRect` → `{x, y, w, h}`; `autofillHandlePos` → `{x, y}`; `pixelToCell` → `{row, column}`; `canvasSize` → `{w, h}`.
  - `fitColumnWidth` measures a 1-based column over an inclusive row span and returns CSS pixels, or `undefined` when there is no content to fit. The host applies the result through its workbook model and requests a repaint.
- **overlays** — `setExtendTo(target|null)`, `setClipboard(area|null)`, `setPointRange(range|null)`, `setFormulaRefs(refs)`, `setOverlays(overlays)` (all camelCase field objects; see `wire.rs` for exact shapes)

### dev-tools (feature `dev-tools`)

- **recording** — `startRecording(opts)`, `stopRecording()`, `setFrameDiagnosticsEnabled(b)`, `setFrameDiagnosticsProbe(r1, c1, r2, c2)`, `frameDiagnostics()`, `recordingCurrentAttempt()`
- **playback** — `loadRecording(bytes)`, `seekRecording(frameIdx)`, `playRecording(nowMs)`, `pauseRecording()`, `isPlaying()`, `tickPlayback(nowMs)`, `exitPlayback()`, `playbackActive()`, `recordingFrameCount()`, `recordingCurrentFrame()`
- **free functions** — `icrReplayGridOps(ctx, opsJson)`, `icrReplayOverlayOps(ctx, opsJson)`

## JS model contract — `setModel`

`setModel` adopts an IronCalc wasm `Model` handle after a structural duck-test (module-agnostic, not `instanceof`). Required methods — all part of the IronCalc wasm API:

`getSelectedView`, `getSelectedSheet`, `getFrozenRowsCount`, `getFrozenColumnsCount`, `getRowHeight`, `getColumnWidth`, `getShowGridLines`, `getCellStyle`, `getCellType`, `getFormattedCellValue`

Optional extensions, probed once at bind time; absence degrades gracefully:

- **bulk fetch** — `getCellStylesIn(sheet, r1, c1, r2, c2)`, `getFormattedCellValuesIn(...)`, `getCellTypesIn(...)`, returning dense row-major arrays. Without them the engine falls back to per-cell calls. These are **not** in the upstream IronCalc wasm API — hosts install them on the handle (see `web-test/index.html`).
- `getTheme()` — workbook theme for `Color::Theme(idx, tint)` resolution; absent → Office default.
- `getShowRowHeaders(sheet)` / `getShowColHeaders(sheet)` — absent → headers assumed visible.

`getCellStyle` may return the CF-merged `ExtendedCellStyle` wrapper (`{style, icon, data_bar, rating}`) or a bare `Style`; both deserialize.

**Theme host contract**: after `model.setTheme(...)`, call `ironCanvas.themeChanged()` — it drops the bridge's cached theme and marks content dirty. Without it the stale cache silently misrenders theme colors (host bug, no error).

## Theme shapes

`IronCanvas.setTheme` takes the canvas palette shape — 14 camelCase color slots (`gridColor`, `headerBg`, `headerTextColor`, `selectionColor`, ...). This is a *different* object from IronCalc's `IronCalcTheme` (`name`, `dk1`, `lt1`, `accent1..6`, `hlink`, `folHlink`). A host maps between them — typically via `--palette-*` CSS variables and `setThemeFromElement`, or by hand.

## Rust-only API

Counterpart methods for Rust hosts (RustyCalc's Leptos frontend drives these directly, never through the JS wire shapes): `set_model`, `set_theme`, `set_theme_variables`, `set_overlays`, `set_extend_to`, `set_clipboard`, `set_point_range`, `set_formula_refs`, `view_changed`, `request_overlay_repaint`, `hit_test`, `cell_rect`, `pixel_to_cell`, `resize_handle_at`, `autofill_handle`, `canvas_size`, `scroll_pane_rect`, `legal_scroll_origin`, `scroll_to_show`, `fit_column_width`, `fit_row_height`.

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
