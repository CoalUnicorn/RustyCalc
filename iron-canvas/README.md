# iron-canvas

**WIP**

A `<canvas>` renderer for [IronCalc](https://github.com/ironcalc/IronCalc) workbooks.

There are two stacked canvases. The grid layer paints cells, headers, borders, and text; the overlay layer paints selection, the autofill handle, marching ants, point-mode, and formula refs (multi-color outlines with hit-tested move/resize handles — `Body` translates the ref, `Edge(Side)` resizes one axis, `Corner(Corner)` resizes both). Both layers read the same IronCalc model. On cursor movement only the overlay repaints.

Every visible pixel composes from core geometry such as `PixelRect`, `Line`, and `Point`. The `Painter` surface covers fills, paths, clears, solid and dashed strokes, optimized horizontal and vertical lines, text, clipping, grouping, and DPR/cache hooks. Scroll blitting is the separate `BlitPainter` capability.

## Workspace layout

`iron-canvas/` is a Cargo workspace:

| Crate                  | Role                                                                                     |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| `iron-canvas-core`     | Pure-Rust engine: geometry, `Chrome`, renderer, `Orchestrator<S: Surface>` (model is an `Option<Rc<dyn CanvasModel>>` field), `Painter` trait surface. No `web-sys` / `wasm-bindgen` deps |
| `iron-canvas-canvas2d` | IronCalc-free Canvas-2D backend: `CanvasPainter` (`Painter` impl, caches ctx state), `WebSurface`, `theme_from_element` CSS-var bridge. No `ironcalc_base` — reusable by `iron-canvas-datagrid-web` |
| `iron-canvas-recorder` | `RecorderPainter` + `MemSurface` for tests; `RecordingPainter<P>` + `RecordingSurface<S>` for opt-in live `.icr` capture |
| `iron-canvas-export`   | Multi-format export: `SvgPainter` + `SvgSurface` (feature `svg`), `PdfPainter` + `PdfSurface` (feature `pdf`). Pure `std`. |
| `iron-canvas-ironcalc` | Bridge crate: `IronCalcModel<'a>` newtype implementing `CanvasModel` for IronCalc `UserModel`. CF (conditional formatting) decoration bridge. |
| `iron-canvas-datagrid` | Engine-agnostic in-memory table implementing `CanvasModel`: `DataGrid`, `DataGridBuilder`, `Column`, `Cell`, `SortDirection`. No IronCalc |
| `iron-canvas-web`      | `#[wasm_bindgen]` facade: `IronCanvas`, `JsBackedModel`. Re-exports the core API plus `WebSurface`, `CanvasPainter`, and `theme_from_element` |
| `iron-canvas-datagrid-web` | `#[wasm_bindgen]` facade for a standalone canvas grid: `DataGridCanvas`. Composes datagrid + canvas2d + export — zero IronCalc |

Consumers depending on the wasm bundle use `iron-canvas-web`. Native /
non-browser backends impl `Surface` (associated `type P = YourPainter`)
and wire `Orchestrator<YourSurface>` directly.

## Development

IronCalc is vendored as a git submodule in the parent RustyCalc repo.

```bash
# From the RustyCalc root:
git submodule update --init   # if you cloned without --recurse-submodules

# Build and test (from the RustyCalc root):
cargo check --target wasm32-unknown-unknown
cargo test --workspace

# Or build/test iron-canvas in isolation:
cd iron-canvas
cargo test --workspace
```

## Quick start

```js
import initIronCanvas, { IronCanvas } from "./iron_canvas_web.js";
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
matching size) and forwards resize / model / repaint calls. The wasm-bindgen
surface is `IronCanvas` only; everything else (Orchestrator, LayerBase, Surface,
WebSurface, the renderer types) is Rust-only.

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
| `canvas.exportSvg(css_w, css_h)` | Render the current sheet as a self-contained SVG string. Drives a throwaway `Orchestrator<SvgSurface>` against the cached model — no painted-pixel state on the live canvas is touched. Returns `""` if no model is bound. |
| `canvas.exportPdf(css_w, css_h)` | Render the current sheet as a self-contained PDF (returned as `Uint8Array`). Gated behind `--features pdf`. |

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
| `canvas.themeChanged()` | Notify that external theme vars changed (e.g. OS dark-mode toggle). Re-reads `--palette-*` and triggers a full repaint. |

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
| `canvas.set_overlays(overlays)` | Batch overlay setter — `RenderOverlays` struct carrying all decorations at once. |

#### Queries

All queries read the last painted `Chrome`. Return immediately; no rebuild triggered.
Before the first `paintIfDirty` every query returns its absent variant.

```rust
// What is the cursor over?
let hit: HitTest = canvas.hit_test(x, y);
// HitTest::Cell { row, column }
// HitTest::RowHeader(row) | ColHeader(col) | Corner
// HitTest::AutofillHandle { row, column }
// HitTest::FormulaRef { ref_idx, zone, grab_row, grab_col }
//   // zone: RefZone::Body | Edge(Side) | Corner(Corner)
//   // grab_row/grab_col preserve the relative pointer position inside the ref
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
        HitTest::AutofillHandle { .. } => { /* start autofill drag */ }
        HitTest::FormulaRef { ref_idx, zone, grab_row, grab_col } => {
            // Body → translate the ref; Edge(Side) → single-axis resize;
            // Corner(Corner) → two-axis resize. ref_idx indexes the
            // Vec<FormulaRef> last pushed via set_formula_refs.
        }
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

A workbook with only frozen rows collapses to `top_left` + `bottom_left`; only frozen columns to `top_left` + `top_right`; nothing frozen to `bottom_right` alone. Overlays live on the second canvas and paint *after* the grid — they never alter the snapshot the next hit-test reads.

## How it works

### Layers, surfaces, painters

Both layers read the same `Option<Rc<dyn CanvasModel>>` held by the `Orchestrator<S>` — a single type param, with the model carried as a field rather than a second generic. The grid canvas is opaque (`alpha: false`) and the overlay uses `alpha: true, desynchronized: true`. Each layer is a `LayerBase<S, R>` where `S: Surface` owns the painter and `R: LayerOps<Painter = S::P>` is the renderer wrapper. The surface hands the renderer an `Rc<S::P>` clone at construction, so paint methods do not re-borrow through the surface on every call. `LayerBase` also carries a `PaintGate` (typed `GridSignals` dirty bits) and a long-lived `RendererCore<S::P>` whose caches survive across frames.

`Surface` is the backend-agnostic drawing target. It owns an associated `type P: Painter + BlitPainter` plus `painter`, `clone_painter`, `resize`, and `present`. `WebSurface` wraps an `HtmlCanvasElement` and an `Rc<CanvasPainter>`. `MemSurface` (in `iron-canvas-recorder`) wraps an `Rc<RecorderPainter>` and drives `Orchestrator<MemSurface>` through every paint regime inside core's integration tests.

The `Painter` trait is unsealed; adapter crates implement it. Renderer code does not touch `CanvasRenderingContext2d` directly. The layer-clear and full-canvas-fill paths route through `Painter::clear_rect` and `Painter::rect_fill` so SVG, PDF, and recorder backends see the same op stream. Five painter types ship today: `CanvasPainter`, `SvgPainter`, `PdfPainter`, `RecorderPainter`, and the `RecordingPainter<P>` decorator.

### Model and overlay decorations

`CanvasModel` is the read-only adapter trait at `crates/iron-canvas-core/src/model_adapter.rs`, extending the per-cell `CellContentQuery` trait with sheet, viewport, header, and geometry queries. `CellContentQuery` provides four batched range accessors for styles, formatted values, cell types, and decorations. The JS bridge overrides the first three to collapse a pane fetch into one boundary crossing; decoration fetching currently uses the default loop. Forwarding impls let `Rc<T>` and `Rc<dyn CanvasModel>` pass through the same query surface.

Selection, autofill preview, clipboard ants, point-mode, and formula-ref outlines each implement the `Layer` trait in `crates/iron-canvas-core/src/decoration/`. `LayerBase::paint_overlay_layer` walks the built-ins in fixed z-order, followed by consumer layers registered through `Orchestrator::add_decoration`.

### Per-frame snapshot and dispatch

`Chrome` has two construction paths. `Chrome::next(prev, model, canvas, theme, path)` handles `FramePath::Fresh` and `FramePath::SlotsReuse`; the pure-scroll fast path is `Chrome::next_blit(.., &BlitPlan) -> BlitOutcome`. The resulting `Chrome` is the single source of truth for hit-test geometry.

`paint_if_dirty` (on `Orchestrator`; `IronCanvas::paintIfDirty` delegates to it) drains typed dirty signals from both layers and dispatches to one of four regimes in cheapness order. `Overlay` repaints the overlay only and skips the grid rebuild. `Viewport` runs the scroll blit. `SlotsReuse` keeps the viewport stable and refetches only the masked panes. `Fresh` is the full rebuild.

### Pane pipeline and theme

`render_grid` paints the four pane quadrants (`top_left`, `top_right`, `bottom_left`, `bottom_right`), then frozen separators, then headers, then the corner box. Each pane runs five deferred sub-passes over one reused slot vec: background, conditional-formatting decoration, grid borders, explicit borders, then text. The sub-pass order is the contract — decorations stay below borders, explicit borders win over grid borders at shared edges, and text runs last so overflow is not clipped by a neighbour's background.

`CanvasTheme` fields are `Cow<'static, str>`. `light()` and `dark()` are built-in palettes (`Cow::Borrowed`, ptr-eq cache hit); host overrides via `ThemeVariables` are `Cow::Owned`. On wasm32, `setThemeFromElement` reads `--palette-*` off `getComputedStyle`.

### Recording

`iron-canvas-recorder` does double duty: it is both the test backend (`RecorderPainter` + `MemSurface` driving `Orchestrator<MemSurface>` through every regime) and the dev-only producer of `.icr` recording files.

Enable the producer by building `iron-canvas-web` with the `dev-tools` feature:

```sh
wasm-pack build --target web --features dev-tools     # standalone
# or, from the RustyCalc workspace root:
trunk serve --features dev-tools                      # full app
```

With the feature on, `IronCanvas` exports `startRecording()` / `stopRecording()` and `RecordingSurface<S>` forks every painter call into a per-frame buffer. The output is a single uncompressed JSON document conforming to the `.icr` schema in `crates/iron-canvas-recorder/src/recording.rs` (`IcrHeader { schema_version, iron_canvas_version, canvas_w, canvas_h, dpr, theme, started_at_unix_ms, partial }` plus frames containing `frame_idx`, `t_ms`, `regime`, `signals`, `grid_ops`, and `overlay_ops`).

Replay an `.icr` by opening [`web-test/recording-viewer.html`](web-test/recording-viewer.html) and drag-dropping the file; the page mirrors `iron_canvas_recorder::replay` in JS and paints onto a single 2D canvas. The always-on `recordingSupported() -> bool` probe lets the page detect whether the loaded wasm has recording compiled in. Without the feature flag, recording symbols are not exported and the prod bundle pays zero overhead.

## Tests

`RecorderPainter` (in `iron-canvas-recorder`) is the testing entry point.
Renderer tests construct one, drive a render pass, and assert against the
resulting `Vec<DrawOp>`. The four-regime integration test in
`crates/iron-canvas-core/tests/orchestrator_regimes.rs` drives
`Orchestrator<MemSurface>` through `Fresh` / `SlotsReuse` / `Viewport` /
`Overlay` and asserts the expected op log for each.

```
cargo test --workspace
```

## Status

Pre-1.0. Wasm consumers depend on `iron-canvas-web` and import through
`iron_canvas_web::*` (which re-exports `IronCanvas` plus the
`iron-canvas-core` public surface). Native / non-browser consumers pull
`iron-canvas-core` directly and provide their own `Surface` impl.
