# iron-canvas

A `<canvas>` renderer for [IronCalc](https://github.com/ironcalc/IronCalc) workbooks.

Two stacked canvases — a static **grid** layer (cells, headers, borders, text) and a dynamic **overlay** layer (selection, autofill handle, marching ants, formula refs). Both are driven from the same `IronCalc` model; only the overlay repaints on cursor movement.

Everything visible is a `PixelRect` or a `Line`. The painter surface is five primitives: `rect_fill`, `rect_stroke`, `rect_dashed`, `stroke_line`, `fill_text` (plus clip + text helpers). New visuals reduce to those or they don't ship.

## Usage

```rust
use iron_canvas::IronCanvas;

let canvas = IronCanvas::create(grid_canvas_el, overlay_canvas_el)?;
canvas.resize(css_w, css_h, dpr);
canvas.set_model(model);
canvas.paint_if_dirty();
```

The consumer mounts two `<canvas>` elements (overlay on top, `pointer-events: none`, matching size) and forwards resize / model / repaint calls. `GridLayer` and `OverlayLayer` are private — the wasm-bindgen surface is `IronCanvas` only.

## How it works

**Two layers, one model.** Both layers read the same `Rc<dyn CanvasModel>`. Grid is opaque, overlay is `desynchronized: true`. Each layer wraps a `LayerBase<R: LayerOps>` with one `PaintGate` dirty bit and a long-lived `RendererCore<CanvasPainter>` whose caches survive across frames.

**Painter trait.** Sealed; renderer code never touches `CanvasRenderingContext2d`. Three impls: `CanvasPainter` (production, caches ctx state), `SvgPainter` (snapshot output), `RecorderPainter` (test-only `DrawOp` log). Colors cross as `PaintColor<'a>::{Static(&'static str), Borrowed(&'a str)}` so theme constants ptr-eq against the painter cache zero-alloc.

**Model.** `CanvasModel` is the read-only adapter trait (`src/model_adapter.rs`); `UserModel<'a>` from `ironcalc_base` plugs in via blanket impl.

**Per-frame snapshot.** `FrameContext::current(model, size, theme)` walks the model once per axis into four slot vecs (`frozen_rows` / `scroll_rows` / `frozen_cols` / `scroll_cols`). Renderer and query API (`hit_test`, `cell_rect`, `autofill_handle`) read the same snapshot, so painted pixels and hit-tests agree by construction.

**`paint_if_dirty`.** If only the overlay is dirty and `last_frame.is_still_valid(model, size)`, repaint the overlay against the cached frame with no model walk. Otherwise rebuild `FrameContext`, paint dirty layers, cache.

**Pane pipeline.** `render_grid` paints four quadrants (`top_left` / `top_right` / `bottom_left` / `bottom_right`), then frozen separators, then headers, then corner box. Each pane runs four deferred sub-passes over one reused slot vec: bg → grid borders → explicit borders → text. Order is load-bearing.

**Theme.** `CanvasTheme` fields are `Cow<'static, str>`. `light()` / `dark()` are built-in palettes (`Cow::Borrowed`, ptr-eq cache hit); `ThemeVariables` produces host overrides (`Cow::Owned`, content-eq). On wasm32, `CanvasTheme::from_element(&Element)` reads `--palette-*` off `getComputedStyle`.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the full design.

## Tests

`RecorderPainter` is the testing entry point — see [`src/test/painter.rs`](src/test/painter.rs) for the recorder and inline examples (`rect_fill_records_op`, `push_pop_clip_balances_depth`, `measure_text_width_parses_font_size_from_css`). Renderer tests construct a `RecorderPainter`, drive a render pass, and assert against the resulting `Vec<DrawOp>`.

```
cargo test
```

## Status

Pre-1.0. Public API is `IronCanvas` + the `geometry` re-exports from `lib.rs`. Everything else is internal and may move.
