# iron-canvas

**WIP**

A `<canvas>` renderer for [IronCalc](https://github.com/ironcalc/IronCalc) workbooks.

Two stacked canvases — a static **grid** layer (cells, headers, borders, text) and a dynamic **overlay** layer (selection, autofill handle, marching ants, formula refs). Both are driven from the same `IronCalc` model; only the overlay repaints on cursor movement.

Everything visible is a `PixelRect` or a `Line`. The painter surface is five primitives: `rect_fill`, `rect_stroke`, `rect_dashed`, `stroke_line`, `fill_text` (plus clip + text helpers). New visuals reduce to those or they don't ship.

## Quick start

```js
import initIronCanvas, { IronCanvas } from "./iron_canvas.js";
import initIronCalc, { Model } from "./wasm.js";

await Promise.all([initIronCanvas(), initIronCalc()]);

const canvas = IronCanvas.create(gridCanvasEl, overlayCanvasEl);
canvas.setModel(model);
canvas.resize(800, 400, window.devicePixelRatio || 1);
canvas.requestRepaint();

const loop = () => { canvas.paintIfDirty(); requestAnimationFrame(loop); };
requestAnimationFrame(loop);
```

The consumer mounts two `<canvas>` elements (overlay on top, `pointer-events: none`,
matching size) and forwards resize / model / repaint calls. `GridLayer` and
`OverlayLayer` are private — the wasm-bindgen surface is `IronCanvas` only.

## CSS stacking

```html
<div style="position:relative; width:800px; height:400px">
  <canvas id="grid"    style="position:absolute;inset:0;width:100%;height:100%"></canvas>
  <canvas id="overlay" style="position:absolute;inset:0;width:100%;height:100%;pointer-events:none"></canvas>
</div>
```

## API reference

### JS / wasm-bindgen surface

These are the methods exported via `#[wasm_bindgen]` and available from JavaScript.

#### Lifecycle

| Method | Description |
| ------ | ----------- |
| `IronCanvas.create(gridCanvas, overlayCanvas)` | Construct over two stacked canvases. Returns `IronCanvas` or throws. |
| `canvas.resize(css_w, css_h, dpr)` | Resize both layers. Call whenever the element's CSS size or DPR changes. |
| `canvas.dispose()` | Release the canvas. Call when unmounting. |

#### Model

| Method | Description |
| ------ | ----------- |
| `canvas.setModel(model)` | Bind an IronCalc `Model` JS handle. Triggers a full repaint. |

#### Repaint triggers

Setters are value-compared — pushing the same value is a no-op. Call `paintIfDirty`
from your rAF loop; it skips silently when nothing changed.

| Method | Description |
| ------ | ----------- |
| `canvas.requestRepaint()` | Force a full grid + overlay repaint on the next `paintIfDirty`. Use after structural changes (sheet switch, freeze). Does not raise `CONTENT` — use `markContentDirty` when cell values changed. |
| `canvas.markContentDirty()` | Signal that cell values have changed. Grid refetches all panes on the next `paintIfDirty`. |
| `canvas.paintIfDirty()` | Drive the paint loop. Call from `requestAnimationFrame`. |

#### Theme

| Method | Description |
| ------ | ----------- |
| `canvas.set_theme_name(name)` | Switch to a built-in palette: `"light"` or `"dark"`. |
| `canvas.setThemeFromElement(el)` | *(wasm32 only)* Read `--palette-*` CSS vars off `el` and apply them. |

### Rust API

The following are available from Rust (e.g. Leptos components) but are **not**
exported to JS. They live in the plain `impl IronCanvas` block.

#### Overlay state

Push these whenever selection or formula context changes. Each setter value-compares
and raises `OVERLAY`; only the overlay layer repaints.

| Method | Description |
| ------ | ----------- |
| `canvas.request_overlay_repaint()` | Force overlay repaint after active-cell move. |
| `canvas.set_extend_to(target)` | Autofill drag target (`Option<AutofillTarget>`). |
| `canvas.set_clipboard(area)` | Clipboard marching-ants area (`Option<SheetArea>`). |
| `canvas.set_point_range(range)` | Point-mode range highlight (`Option<RCRange>`). |
| `canvas.set_formula_refs(refs)` | Formula-ref outlines (`Vec<FormulaRef>`). |

#### Queries

All queries read the last painted `Chrome`. Return immediately; no rebuild triggered.
Before the first `paintIfDirty` every query returns its absent variant.

```rust
// What is the cursor over?
let hit: HitTest = canvas.hit_test(x, y);
// HitTest::Cell { row, column }
// HitTest::RowHeader(row) | ColHeader(col) | Corner
// HitTest::AutofillHandle { row, column }
// HitTest::Outside

// Pixel rect of a visible cell
let rect: Option<PixelRect> = canvas.cell_rect(row, col);

// Row/column resize handle within tolerance pixels of (x, y)
let handle: Option<ResizeTarget> = canvas.resize_handle_at(x, y, tolerance);
// ResizeTarget::Column(col) | Row(row)

// Position of the autofill handle dot
let pt: Option<Point> = canvas.autofill_handle();

// Logical canvas size
let size: CanvasSize = canvas.canvas_size();
```

#### Hit-test example (Leptos)

```rust
let on_mousemove = move |ev: MouseEvent| {
    let rect = canvas_el.get_bounding_client_rect();
    let x = ev.client_x() as f64 - rect.left();
    let y = ev.client_y() as f64 - rect.top();
    match canvas.hit_test(x, y) {
        HitTest::Cell { row, column } => { /* select cell */ }
        HitTest::AutofillHandle { .. } => { /* start drag */ }
        HitTest::Outside => {}
        _ => {}
    }
    if let Some(handle) = canvas.resize_handle_at(x, y, 4.0) {
        // set col-resize / row-resize cursor
    }
};
```

## Anatomy

`Chrome` is the per-frame snapshot every painter and every hit-test query reads.
It wraps the cell area with the row-header strip, the column-header strip, the
corner box, and (when frozen panes are active) the frozen separators. Inside, the
cell area splits into up to four pane quadrants holding the rows/columns.

```text
┌─ Sheet ────────────────────────────────────────────────────────────┐
│ ┌─ Chrome ──────────┬─ Col header strip (chrome) ──────────────┐   │
│ │ Corner box        │  A   B   C   D   E   F   G   H           │   │
│ │ (chrome)          │                                          │   │
│ ├───────────────────┼──────────────────────────────────────────┤   │
│ │ Row header strip  │ ┌─ Pane (top_left, frozen × frozen) ───┐ │   │
│ │ (chrome)          │ │ ┌─Cell─┬─Cell─┐                      │ │   │
│ │  1                │ │ │  bg  │ text │  shared border edges │ │   │
│ │  2                │ │ ├──────┼──────┤                      │ │   │
│ │  3                │ │ │ text │  bg  │                      │ │   │
│ │  4                │ │ └──────┴──────┘                      │ │   │
│ │  5                │ ├──── frozen separator (chrome) ───────┤ │   │
│ │  6                │ │ Pane (bottom_left, scroll × frozen)  │ │   │
│ │  7                │ │ ┌─Cell─┬─Cell─┬─Cell─┐               │ │   │
│ │ ...               │ │ │      │      │      │               │ │   │
│ │                   │ │ └──────┴──────┴──────┘               │ │   │
│ │                   │ └──────────────────────────────────────┘ │   │
│ └───────────────────┴──────────────────────────────────────────┘   │
│                                                                    │
│ Overlay canvas (transient, painted on top of the grid):            │
│   selection · autofill handle · marching ants ·                    │
│   point-mode · formula refs · header highlights                    │
└────────────────────────────────────────────────────────────────────┘
```

A workbook with only frozen rows collapses to `top_left` + `bottom_left`; only
frozen columns to `top_left` + `top_right`; nothing frozen to `bottom_right` alone.
See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the build phases (A → E) and query
pipeline.

## How it works

**Two layers, one model.** Both layers read the same `Rc<dyn CanvasModel>`. The
grid canvas is opaque (`alpha: false`); the overlay uses
`alpha: true, desynchronized: true`. Each layer wraps a `LayerBase<R: LayerOps>`
that holds a `PaintGate` (typed `GridSignals` dirty bits) and a long-lived
`RendererCore<CanvasPainter>` whose caches survive across frames.

**Overlay decorations.** Selection, autofill preview, clipboard ants, point-mode,
and formula-ref outlines are each a struct implementing the `Layer` trait in
`src/layer/decoration/`. `OverlayLayer::paint` walks them in fixed z-order; each
reads its own state directly — there is no monolithic `render_overlays`.

**Painter trait.** Sealed; renderer code never touches `CanvasRenderingContext2d`.
Three impls: `CanvasPainter` (production, caches ctx state), `SvgPainter`
(snapshot output), `RecorderPainter` (test-only `DrawOp` log).

**Model.** `CanvasModel` is the read-only adapter trait (`src/model_adapter.rs`) —
singular accessors plus three batched range accessors (`get_cell_styles_in`,
`get_formatted_cell_values_in`, `get_cell_types_in`) that collapse a JS-bridge
pane fetch to one boundary crossing.

**Per-frame snapshot.** `Chrome::next(prev, model, canvas, theme, path)` is the
single constructor. `path: FramePath` selects one of three build regimes
(`Fresh` / `SlotsReuse` / `Blit`). The `Fresh` arm walks the model once per axis
into four slot vecs; `SlotsReuse` reuses the previous frame's vecs; `Blit` shifts
one axis in-place for a pure scroll. The resulting `Chrome` is the single source
of truth for hit-test geometry.

**`paintIfDirty`.** Drains typed dirty signals from both layers and dispatches to
one of four regimes in cheapness order: `Overlay` (overlay-only, no grid rebuild),
`Viewport` (scroll blit), `SlotsReuse` (stable viewport, masked pane refetch),
`Fresh` (full rebuild).

**Pane pipeline.** `render_grid` paints four quadrants (`top_left` / `top_right` /
`bottom_left` / `bottom_right`), then frozen separators, then headers, then corner
box. Each pane runs four deferred sub-passes over one reused slot vec:
bg → grid borders → explicit borders → text. Order is load-bearing.

**Theme.** `CanvasTheme` fields are `Cow<'static, str>`. `light()` / `dark()` are
built-in palettes (`Cow::Borrowed`, ptr-eq cache hit); host overrides via
`ThemeVariables` are `Cow::Owned`. On wasm32, `setThemeFromElement` reads
`--palette-*` off `getComputedStyle`.

## Tests

`RecorderPainter` is the testing entry point — see [`src/test/painter.rs`](src/test/painter.rs)
for the recorder and inline examples. Renderer tests construct a `RecorderPainter`,
drive a render pass, and assert against the resulting `Vec<DrawOp>`.

```
cargo test
```

## Status

Pre-1.0. Public API is `IronCanvas` + the `geometry` re-exports from `lib.rs`.
Everything else is internal and may move.
