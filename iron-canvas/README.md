# iron-canvas

**WIP**

A `<canvas>` renderer for [IronCalc](https://github.com/ironcalc/IronCalc) workbooks.

Two stacked canvases — a static **grid** layer (cells, headers, borders, text) and a dynamic **overlay** layer (selection, autofill handle, marching ants, formula refs). Both are driven from the same `IronCalc` model; only the overlay repaints on cursor movement.

Everything visible is a `PixelRect` or a `Line`. The painter surface is five primitives: `rect_fill`, `rect_stroke`, `rect_dashed`, `stroke_line`, `fill_text` (plus clip + text helpers). New visuals reduce to those or they don't ship.

## Usage

```rust
use iron_canvas::IronCanvas;

let canvas = IronCanvas::create(grid_canvas_el, overlay_canvas_el)?;
canvas.resize(css_w, css_h, dpr);
canvas.set_model(model);
canvas.paintIfDirty();
```

The consumer mounts two `<canvas>` elements (overlay on top, `pointer-events: none`, matching size) and forwards resize / model / repaint calls. `GridLayer` and `OverlayLayer` are private — the wasm-bindgen surface is `IronCanvas` only.

## Anatomy

`Chrome` is the per-frame snapshot every painter and every hit-test query reads. It wraps the cell area with the row-header strip, the column-header strip, the corner box, and (when frozen panes are active) the frozen separators. Inside, the cell area splits into up to four pane quadrants holding the rows/columns.

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

A workbook with only frozen rows collapses to `top_left` + `bottom_left`; only frozen columns to `top_left` + `top_right`; nothing frozen to `bottom_right` alone. Overlays live on the second canvas and paint *after* the grid — they never alter the snapshot the next hit-test reads. See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the build phases (A → E) and query pipeline.

## How it works

**Two layers, one model.** Both layers read the same `Rc<dyn CanvasModel>`. Grid is opaque, overlay is `desynchronized: true`. Each layer wraps a `LayerBase<R: LayerOps>` with one `PaintGate` dirty bit and a long-lived `RendererCore<CanvasPainter>` whose caches survive across frames.

**Painter trait.** Sealed; renderer code never touches `CanvasRenderingContext2d`. Three impls: `CanvasPainter` (production, caches ctx state), `SvgPainter` (snapshot output), `RecorderPainter` (test-only `DrawOp` log). Colors cross as `PaintColor<'a>::{Static(&'static str), Borrowed(&'a str)}` so theme constants ptr-eq against the painter cache zero-alloc.

**Model.** `CanvasModel` is the read-only adapter trait (`src/model_adapter.rs`) — singular accessors plus three batched range accessors (`get_cell_styles_in`, `get_formatted_cell_values_in`, `get_cell_types_in`) that collapse a JS-bridge pane fetch to one boundary
crossing. `UserModel<'a>` from `ironcalc_base` plugs in via the trait's default loop impl. 

**Per-frame snapshot.** `Chrome::current(model, size, theme)` walks the model once per axis into four slot vecs on `Chrome.pane_set` (`frozen_rows` / `scroll_rows` / `frozen_cols` / `scroll_cols`). Renderer and query API (`hit_test`, `cell_rect`, `autofill_handle`) read the same `Chrome`, so painted pixels and hit-tests agree by construction.

**`paintIfDirty`.** If only the overlay is dirty and `last_frame.is_still_valid(model, size)`, `refresh_overlay_inputs` re-snapshots selection from the model and the overlay repaints against the cached frame with no model walk. Otherwise the outgoing frame is consumed by `Chrome::rebuild` (which recycles its slot Vec allocations) — or `Chrome::current` on the very first paint — dirty layers paint, and the result is cached as `last_frame`.

**Pane pipeline.** `render_grid` paints four quadrants (`top_left` / `top_right` / `bottom_left` / `bottom_right`), then frozen separators, then headers, then corner box. Each pane runs four deferred sub-passes over one reused slot vec: bg → grid borders → explicit borders → text. Order is load-bearing.

**Theme.** `CanvasTheme` fields are `Cow<'static, str>`. `light()` / `dark()` are built-in palettes (`Cow::Borrowed`, ptr-eq cache hit); `ThemeVariables` produces host overrides (`Cow::Owned`, content-eq). On wasm32, `CanvasTheme::from_element(&Element)` reads `--palette-*` off `getComputedStyle`.


## Tests

`RecorderPainter` is the testing entry point — see [`src/test/painter.rs`](src/test/painter.rs) for the recorder and inline examples (`rect_fill_records_op`, `push_pop_clip_balances_depth`, `measure_text_width_parses_font_size_from_css`). Renderer tests construct a `RecorderPainter`, drive a render pass, and assert against the resulting `Vec<DrawOp>`.

```
cargo test
```

## Status

Pre-1.0. Public API is `IronCanvas` + the `geometry` re-exports from `lib.rs`. Everything else is internal and may move.
