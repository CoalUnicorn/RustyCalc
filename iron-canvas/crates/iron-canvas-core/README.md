# iron-canvas-core

The pure-Rust rendering engine at the heart of iron-canvas, with zero platform dependencies.

## What it does

Holds everything needed to decide *what* to paint and *where*, without knowing *how* pixels reach a screen. It owns the per-frame `Chrome` snapshot, the `Orchestrator` that drives dispatch, the five-pass cell paint in `renderer`, and the `Painter`/`CanvasModel` traits that every backend and data source plug into.

## Crate role

The foundation. Every other crate in the workspace depends on `iron-canvas-core` and nothing the other way around. Backends (`canvas2d`, `export`, `recorder`) implement its `Painter` trait; data sources (`ironcalc`, `datagrid`) implement its `CanvasModel` trait. The dependency rule is testable: deleting any adapter crate must leave `iron-canvas-core` compiling and green on its own.

## Key exports

- `Orchestrator`, `PaintRegime`, `PaintRegimeTag`: frame dispatch and the blit-vs-repaint decision
- `Chrome` (module): read-only per-frame geometry/state snapshot
- `Painter` (trait, in `painter`): the drawing surface backends implement
- `CanvasModel`, `CanvasView`, `CellContentQuery`: the read-only data adapter traits
- `CanvasTheme`, `ThemeVariables`: resolved colors and metrics
- `PixelRect`, `CanvasSize`, `Point`, `Line`, `Span`, `col_name`: geometry
- `CellStyle`, `CellKind`, `CellDecoration`, `Alignment`, `Border`: style model
- `HitTest`, `ResizeTarget`, `RefZone`, `Side`: pointer-to-cell results
- `FormulaRef`, `RCRange`, `SheetArea`, `AutofillTarget`: coordinate types
- `RenderOverlays`: pushed overlay state (autofill target, marching ants, point-mode range, formula refs)
- `Layer`, `DecorationId`: consumer-defined overlay decorations registered on the orchestrator

## Dependencies

- `serde`: snapshot/wire (de)serialization for the web bindings
- `bitflags`: compact pane/region masks

No `web-sys`, no `wasm-bindgen`, no `ironcalc_base`. This crate compiles for any target.

## Feature flags

- `surface-introspection`: dev/test only; exposes painter-call introspection hooks used by the recorder and integration tests.

## Relationship to sibling crates

Defines the two seams the workspace pivots on: the `Painter` trait (implemented by canvas2d, recorder, export) and the `CanvasModel` trait (implemented by ironcalc, datagrid). Everything else is a backend or a data source bolted onto these.
