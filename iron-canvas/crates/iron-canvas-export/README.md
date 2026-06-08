# iron-canvas-export

Static export backends for iron-canvas — render a grid to SVG or PDF, no browser.

## What it does

Provides two `Painter`/`Surface` implementations that serialize a frame to a document format instead of a live canvas. Both are pure `std` — no `web-sys`, no DOM — so the same `CanvasModel` that renders on screen can be exported server-side or in a headless context. `SvgSurface::render()` spins up a throwaway `Orchestrator`, paints once, and returns the document string.

## Crate role

The "save a picture of the grid" layer. Consumed by both web facades to back their `exportSvg`/`exportPdf` methods. Depends only on `iron-canvas-core`.

## Key exports

- `SvgPainter`, `SvgSurface` (feature `svg`) — DPR-aware, XML-escaped `<svg>` with structured `<g>` groups and `<defs>` clip-paths; `SvgSurface::render(model, theme, size) -> String` is the one-shot entry point
- `PdfPainter`, `PdfSurface` (feature `pdf`) — single-page PDF 1.7 via a hand-rolled writer; base-14 Helvetica, WinAnsi-only, no font embedding
- `common` (module) — shared `xml_escape`, PDF string escaping, CSS-color and font-size parsing

## Dependencies

- `iron-canvas-core` only — no platform crates

## Usage

```rust
let svg = SvgSurface::render(model, theme, CanvasSize { w, h });
```

## Feature flags

- `svg` (default) — SVG painter + surface
- `pdf` (default) — PDF painter + surface

Both default-on; disable one to trim a build that needs only the other.

## Relationship to sibling crates

A backend sibling to `canvas2d` (live) and `recorder` (test). Same `Painter` contract, different sink — pixels vs. recorded ops vs. serialized documents. `web` re-exports its PDF path under that crate's own `pdf` feature.
