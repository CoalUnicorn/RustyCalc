# iron-canvas-canvas2d

The HTML5 Canvas2D backend — `iron-canvas-core`'s `Painter` impl for the browser.

## What it does

Turns abstract paint calls into `CanvasRenderingContext2d` operations. `CanvasPainter` caches ctx-state setters (fill style, font, stroke) so redundant property writes never cross the JS boundary, and `WebSurface` wraps an `HtmlCanvasElement` as a core `Surface`. A small CSS-variable bridge reads theme tokens straight off a DOM element.

## Crate role

The platform layer for live, on-screen rendering. It was split out of `iron-canvas-web` specifically so the standalone data grid (`datagrid-web`) can paint to canvas without pulling in IronCalc — this crate touches `web-sys`/`wasm-bindgen`/`js-sys` but never `ironcalc_base`.

## Key exports

- `CanvasPainter` — the `Painter` impl with setter-state caching
- `WebSurface` — `Surface` impl over `HtmlCanvasElement` (`grid()` / `overlay()` constructors)
- `theme_from_element` (module) — `from_element(&Element) -> CanvasTheme`, mapping CSS custom properties to a resolved theme

## Dependencies

- `iron-canvas-core` — the `Painter`/`Surface` traits and theme types it implements
- `wasm-bindgen`, `js-sys`, `web-sys` — the browser Canvas2D API
- no `ironcalc_base`

## Usage

```rust
let grid = WebSurface::grid(grid_canvas)?;
let overlay = WebSurface::overlay(overlay_canvas)?;
let mut orch = Orchestrator::new(grid, overlay);
orch.set_theme(theme_from_element::from_element(&root_el));
```

## Relationship to sibling crates

Consumed by both `iron-canvas-web` (spreadsheets) and `iron-canvas-datagrid-web` (plain grids). It is the only live-canvas backend; `export` and `recorder` are the static/test backends.
