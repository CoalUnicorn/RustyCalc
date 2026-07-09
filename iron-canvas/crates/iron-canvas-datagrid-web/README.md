# iron-canvas-datagrid-web

The `#[wasm_bindgen]` facade for a standalone canvas data grid — "I just want a fast grid."

## What it does

Exposes `DataGridCanvas` to JavaScript: feed it data, drive a `requestAnimationFrame` loop, and it paints a scrollable, sortable, resizable grid to a `<canvas>` — with zero IronCalc in the bundle. The handle is uniformly 0-based for JS; it is the single seam that translates to the mixed-base pure model underneath.

## Crate role

The shippable artifact for non-spreadsheet consumers. Composes `datagrid` (data), `canvas2d` (paint), and `export` (SVG) behind one wasm class. Built as `cdylib` + `rlib`.

## Key exports

- `DataGridCanvas` (`#[wasm_bindgen]`) — the whole JS API:
  - **data**: `setData`, `setCell`, `appendRows`
  - **layout**: `resize`, `setFrozenHeader`, `setColumnWidth`, `resizeHandleAt`
  - **scroll**: `setScroll`, `scrollBy`
  - **selection / hit-test**: `hitTest`, `selectCell`, `setSelection`
  - **custom overlay**: `setHover`
  - **sort**: `sortByColumn`, `clearSort`, `currentSort`
  - **theme**: `setThemeFromElement`, `setThemeName`
  - **paint / export**: `paintIfDirty`, `exportSvg`

## Dependencies

- `iron-canvas-core`, `iron-canvas-datagrid` — model + renderer
- `iron-canvas-canvas2d` — the on-screen `Painter`
- `iron-canvas-export` — `exportSvg` backing
- `wasm-bindgen`, `js-sys`, `web-sys`, `serde-wasm-bindgen` — JS bindings + wire (de)serialization

## Usage

```js
const grid = new DataGridCanvas(gridCanvas, overlayCanvas);
grid.setData({ columns: [...], rows: [...] });
function frame() { grid.paintIfDirty(); requestAnimationFrame(frame); }
```

## Relationship to sibling crates

The IronCalc-free counterpart to `iron-canvas-web`. Same renderer, same Canvas2D backend — but it swaps the spreadsheet model for `datagrid` and drops `ironcalc_base` entirely, yielding a much smaller wasm payload.
