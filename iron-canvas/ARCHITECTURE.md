<!-- last-verified-against: c16e104 (2026-07-25) -->
<!-- working-tree-verified: 2026-07-25 -->
<!-- covers: iron-canvas/crates/ iron-canvas/Cargo.toml .github/workflows/test.yml -->

# iron-canvas architecture

`iron-canvas` is a Cargo workspace at `iron-canvas/` that paints a
worksheet grid into HTML `<canvas>` elements and answers cursor-position
queries against the painted result. The engine is read-only against the
model (`CanvasModel` trait; IronCalc is adapted through the
`IronCalcModel` newtype in `iron-canvas-ironcalc`); it draws what the
model says and never mutates it.

## Workspace topology

| Crate                  | Role                                                                                                   |
| ---------------------- | ------------------------------------------------------------------------------------------------------ |
| `iron-canvas-core`     | Pure-Rust domain + application. Public modules: `autofit` (pure auto-fit measurement), `geometry/`, `types/`, `signal`, `theme` (model only), `model_adapter` (`CanvasModel: CellContentQuery` — the per-cell content slice split into the `CellContentQuery` supertrait so the cell painter takes `&dyn CellContentQuery`, not the full model), `chrome/` (`Chrome`, `PaneSet`, `BlitPlan`, `PaneRegion[Mask]`, `FrameKindTag`), `decoration/` (`Layer` decorations + `Decorations` struct — top-level sibling of `layer/`), `renderer/`, `layer/` (`Surface` trait, `PaintGate`, `LayerBase`), `painter` (`Painter` / `BlitPainter` / `TextMetrics` + `PainterShapes` ext trait surface, no impls). Private modules: `orchestrator` (`Orchestrator<S>` (single type param, model as `Option<Rc<dyn CanvasModel>>`) + `PaintRegime` dispatch — types re-exported at crate root), `render_overlays` (`RenderOverlays` — re-exported at crate root). No `web-sys` / `wasm-bindgen` deps. |
| `iron-canvas-canvas2d` | IronCalc-free Canvas-2D backend extracted from `iron-canvas-web`: `CanvasPainter` (`Painter` impl with ctx-state setter caching and a bounded `(font, text)` measurement memo), `WebSurface` adapter over `HtmlCanvasElement` (double-buffered grid, direct-draw overlay), and the `theme_from_element` CSS-var → `CanvasTheme` bridge. Depends on `web-sys` / `wasm-bindgen` / `js-sys` but **not** `ironcalc_base`, so `iron-canvas-datagrid-web` reuses it without a spreadsheet engine. |
| `iron-canvas-web`      | `#[wasm_bindgen]` facade: `IronCanvas` handle (owns `Orchestrator<FacadeSurface>` where `FacadeSurface` is `WebSurface` by default and `RecordingSurface<WebSurface>` with `--features dev-tools` — zero recording overhead in prod), `JsBackedModel` (IronCalc JS bridge + `(catch, method)` shim; JS-backed style/color conversion reuses `iron-canvas-ironcalc::convert::{style_to_core, color_to_css}` — no local mirror; only theme caching, JS-failure handling, and `cell_kind_from_discriminant` remain bridge-local; adds `iron-canvas-web → iron-canvas-ironcalc` to the dependency graph). Re-exports `CanvasPainter` (Canvas-2D `Painter` impl), the `WebSurface` adapter, and the `theme_from_element` CSS-var bridge from `iron-canvas-canvas2d`. SVG export via `exportSvg(css_w, css_h) -> String` — drives a throwaway `Orchestrator<SvgSurface>` against the live `model: Option<Rc<dyn CanvasModel>>` cached on the facade. PDF export via `exportPdf(css_w, css_h) -> Vec<u8>` (gated on `--features pdf`) — same throwaway-orchestrator shape as `exportSvg`, calling `PdfSurface::render` and discarding the overlay; `Vec<u8>` auto-converts to `Uint8Array` across wasm-bindgen. Dev-only JS recording API exported only with the feature on: `startRecording` / `stopRecording`. Always-on probe: `recordingSupported() -> bool`. |
| `iron-canvas-export`   | Multi-format export backends behind feature flags (`svg` default-on, `pdf` default-on). SVG: `SvgPainter` (XML-escaped `<svg>` with embedded Inter Regular, structured `<g>` groups, clip-path via `<defs>`, DPR-aware) + `SvgSurface` (one-shot `Orchestrator` surface via crate-private `drive_once`; overlay output discarded). PDF: `PdfPainter` (`Painter` + `BlitPainter` no-op + `TextMetrics`; emits PDF 1.7 content-stream ops) + `PdfSurface` (single-page, `/MediaBox` baked at construction, Y-flip CTM prepended once at page open; one-shot via crate-private `drive_once`; overlay output discarded) + hand-rolled `pdf/doc/` writer (two-pass buffered object table + xref; Type1 base-14 Helvetica, no font embedding — WinAnsi-only). `common/metrics.rs` measures the font each backend actually draws: embedded Inter TTF advances for SVG and Helvetica AFM widths for PDF, with the core approximation only for unmapped glyphs. No `web-sys` / `wasm-bindgen` deps — pure `std`. |
| `iron-canvas-recorder` | Two roles: (1) test backend — `RecorderPainter` + `MemSurface` capture a `DrawOp` log; always present. (2) opt-in dev tool — `RecordingSurface<S>` decorator, `.icr` format. Dev-tools wiring gated behind `--features dev-tools` in `iron-canvas-web`. |
| `iron-canvas-ironcalc` | Bridge crate: `IronCalcModel<'a>` newtype implementing `CanvasModel` for IronCalc `UserModel`. Exists because Rust orphan rules prevent `impl CanvasModel for UserModel` outside the trait-defining crate. Also hosts the CF (conditional formatting) bridge — `get_extended_cell_style()` exposes per-cell CF decorations (data bars, icon sets, color scales) to the canvas paint pipeline. The `convert` module (`style_to_core`, `color_to_css`) is the single source of truth for IronCalc→core style mapping, consumed by both the native `IronCalcModel` path and `iron-canvas-web`'s `JsBackedModel`. |
| `iron-canvas-datagrid` | Standalone data-grid widget. `canvas_model.rs` implements `CanvasModel` + `CellContentQuery` directly on `DataGrid` — sortable columns, custom column headers, frozen header toggle, per-cell styles, custom header text. `model_cell.rs` adds `DataGridModel` (a `RefCell<DataGrid>` interior-mutable wrapper that also impls both traits) — moved here from `iron-canvas-datagrid-web` (June 2026) so non-wasm (Leptos) consumers can drive the grid without the wasm facade. Pure-Rust, no `web-sys` / `wasm-bindgen`. |
| `iron-canvas-datagrid-web`| WASM wrapper: the `DataGridCanvas` `#[wasm_bindgen]` handle + the `HoverLayer` decoration (`hover.rs`); re-exports `DataGridModel` from `iron-canvas-datagrid` (no longer defines it). Exposes wire types for column config, mutation, and viewport state. `DataGridCanvas::resize` forwards the browser's fractional `f64` DPR unchanged through the core and Canvas-2D backend. Added June 2026. |

Dependency rule: deleting any adapter crate must leave `iron-canvas-core`
compiling and passing tests on its own. `iron-canvas-core` contains the
engine + the trait surface; concrete painters live in sibling adapters.

This document covers three connected pipelines:

1. **Frame build** — how `Chrome::next(prev, model, canvas, theme, path)`
   produces the per-tick snapshot every renderer phase and every query reads.
   `path: FramePath` selects between two reuse-or-rebuild regimes (`Fresh`,
   `SlotsReuse`); the blit fast-path is a separate constructor,
   `Chrome::next_blit(.., plan) -> BlitOutcome`.
2. **Paint dispatch** — how `Orchestrator::paint_if_dirty` selects one of
   five `PaintRegime` variants and how the renderer's pane caches, damage
   planner, blit preflight, and five-pass cell painter preserve pixels.
3. **Query** — how `IronCanvas::{hit_test, cell_rect, resize_handle_at,
   autofill_handle}` resolve cursor position against that snapshot, by
   delegating to `Orchestrator<FacadeSurface>` (`WebSurface` in prod,
   `RecordingSurface<WebSurface>` with `--features dev-tools`).

## Core invariants

- **One snapshot per painted tick.** `Chrome` is a `pub struct` (in
  `iron-canvas-core`) rebuilt by `Chrome::next(prev, model, canvas,
  theme, path: FramePath)` when the selected regime needs a frame. `path` selects one of
  two reuse-or-rebuild build regimes (`Fresh`, `SlotsReuse`); the blit
  fast-path is the separate `Chrome::next_blit(.., &BlitPlan) ->
  BlitOutcome` constructor. `Damage` uses `FramePath::SlotsReuse`, while
  `Overlay` bypasses both constructors and reuses `last_frame` directly.
  The resulting `Chrome` is the single
  source of truth for the painted geometry.
- **Slot vecs carry absolute canvas coords.** A `RowSlot` stores the
  absolute pixel `top`; a `ColSlot` stores absolute pixel `left`. No
  prefix-sum decoding — every pixel↔cell query reads slot vecs directly.
- **Paint and query share the snapshot.** The renderer reads `Chrome`;
  `IronCanvas` query methods read the same `last_frame: Option<Chrome>`.
  What was painted is what gets hit. No parallel coordinate path exists.
- **`FROZEN_SEP` is PaneSet's concern.** The 3-px gap between the frozen
  band and the scrollable band is woven into the slot vecs by `fill_rows`
  / `fill_cols`. Chrome geometry never reads `FROZEN_SEP` directly; the
  `frozen_separator` painter is the only consumer outside `PaneSet`.

## Frame regimes & dispatch

`Chrome` has two constructors. `Chrome::next(prev, model, canvas, theme,
path: FramePath) -> Chrome` runs the two reuse-or-rebuild regimes the
`path` argument selects (`Fresh`, `SlotsReuse`); `Chrome::next_blit(prev,
model, canvas, theme, &BlitPlan) -> BlitOutcome` is the separate blit
fast-path. These constructors support the five orchestrator regimes:
`Fresh`, `SlotsReuse`, `Damage`, `Viewport`, and `Overlay`.

| `FramePath`         | Selected when                                                                                          | What it does                                                                                                                                                                              |
| ------------------- | ------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Fresh`             | First paint, or structural divergence (scroll past viewport, sheet/freeze/canvas size change, theme).  | Walks the model fresh through private `Chrome::build` (Phase A–E below). Recycles prev's slot Vec allocations via `RecycledSlots` when `prev` is `Some`.                                  |
| `SlotsReuse { stale_panes }` | Viewport + freeze + sheet + canvas size unchanged; content/overlay may have changed.        | Reuses prev's slot vecs as-is; refreshes theme and kind tag. Per-pane content and painted-pixel fingerprint state lives on `RendererCore`'s `PaneCache` (renderer-lifetime, not on `Chrome`), so nothing needs rotating here — see `docs/rendering-and-damage.md` §3–4 for the full skip-compare and row-damage-planning mechanism. `stale_panes` is caller-supplied so a `SlotsReuse` after a blit doesn't inherit the blit's narrow strip mask. |

The blit fast-path is **not** a `FramePath` variant — it has a
two-outcome result, so it lives in `Chrome::next_blit`. Selected for a
pure single-axis scroll whose kept band permits a `Painter::blit` shift,
it delegates to `try_blit_reuse` (chrome/blit.rs): on success it rebuilds
the scroll-axis slot vec around the plan, keeps the cross-axis vec, and
narrows `stale_panes` to the panes the scroll touched, returning
`BlitOutcome::Blitted`; on reject (`Err`) it rebuilds `Fresh` and returns
`BlitOutcome::FreshFallback`.

The orchestrator's `PaintRegime` enum mirrors these paths (`Overlay` ↔
neither constructor; `SlotsReuse` / `Fresh` ↔ the two `FramePath` arms;
`Damage` ↔ the same `FramePath::SlotsReuse` arm with empty stale panes;
`Viewport(BlitPlan)` ↔ `Chrome::next_blit` → `BlitOutcome`).
`Orchestrator::<S>::decide()` (in
`iron-canvas-core/src/orchestrator.rs`) is the single dispatch point
that picks the regime from `GridSignals`, after `is_still_valid` and
`screen_for_blit` produce their verdicts.

The `Overlay` regime is the cheapest path: `paintIfDirty` reuses
`last_frame` without rebuilding and repaints only the overlay layer.
Each `Layer` decoration reads its own live state directly — there is
no `refresh_overlay_inputs` snapshot step. It is selected when only
overlay-affecting state changed (autofill drag, clipboard,
formula-ref highlight, active-cell move).

## Frame build pipeline — the `Fresh` arm

`Chrome::build(model, canvas, theme, recycled)`
is the private path taken by `FramePath::Fresh` (and indirectly by
`Chrome::next_blit`'s `BlitOutcome::FreshFallback`, when `try_blit_reuse`
bails). It runs
five phases in fixed order. The order is load-bearing: phase C
measures a value phase D needs, and both row/col walks must complete
before E can assemble the shared `cell_origin`.

```
A  frozen counts     model.get_frozen_{rows,columns}_count(sheet)
B  row walk          pane_set = PaneSet::with_recycled(recycled);
                     pane_set.fill_rows(model, frozen_row_count, origin_y, view.top_row, canvas.h)
C  measure r.h.t.    row_header_thickness = measure_row_header_width(last_visible_row)
D  col walk          pane_set.fill_cols(model, frozen_col_count, origin_x, view.left_column, canvas.w)
E  assemble          Chrome { pane_set, row_header_thickness, col_header_thickness, cell_origin, … }
```

### Phase A — frozen counts

Reads `view = model.get_selected_view()` (falls back to a fresh-model
default if the JS bridge is mid-transient), `sheet =
model.get_selected_sheet()`, `model.get_frozen_{rows,columns}_count
(sheet).unwrap_or(0)`, and the two header-visibility flags
`get_show_{row,col}_headers(sheet).unwrap_or(true)`. The visibility
flags gate the `origin`/`thickness` locals through Phases B–E: a hidden
strip is modelled as **thickness 0**, so its `CELL_AREA_INSET` collapses
too and cells reclaim the edge.

### Phase B — row walk

`origin_y = HEADER_ROW_HEIGHT + CELL_AREA_INSET` (cell-area top edge),
or `0` when the column header is hidden.
`pane_set.fill_rows` (a thin wrapper over `rows.fill`) walks rows
`1..=frozen_count` then `(frozen_count + 1).max(view.top_row)..=last_row`,
where `last_row = model.last_row(sheet)` — Excel's `LAST_ROW` by default,
a finite model's data extent when overridden. The bound is snapshotted
into `rows.last_id` for the blit-path rebuilds and the autofill-handle
guard. Populates `rows.frozen` and `rows.scroll` plus the
`rows.frozen_offset = end_of_last_frozen + (FROZEN_SEP if frozen_count>0
else 0)`. Stops the scroll walk when `y_cursor >= canvas.h` or at the
bound. Independent of `row_header_thickness` — runs before C.

### Phase C — measure `row_header_thickness`

Pulls the last row index from `pane_set.rows.scroll.last()` (or the
fresh-model fallback). `measure_row_header_width(n)` returns
`digit_count(n) * APPROX_DIGIT_WIDTH_PX + 2 * HEADER_LABEL_PAD_PX`,
with `HEADER_COL_WIDTH` as a minimum width (the formula widens past
that as digit count grows); a hidden row header skips the measure and
sets `row_header_thickness = 0`. Char-count approximation is intentional
— wiring `TextMetrics` into the build path would couple `Chrome::build`
to a painter, which it must not have.

### Phase D — col walk

`origin_x = row_header_thickness + CELL_AREA_INSET` (`0` when the row
header is hidden). `pane_set.fill_cols`
mirrors `fill_rows` on the column axis, populating `cols.frozen` /
`cols.scroll` and `cols.frozen_offset`.

### Phase E — assemble

`cell_origin = Point { x: origin_x, y: origin_y }` (single source of
truth for hit-test and viewport math). `col_header_thickness =
HEADER_ROW_HEIGHT`, or `0` when the column header is hidden (otherwise
static today; the field exists so the day it goes dynamic, only this
assignment changes). Selection is not stored on `Chrome`;
`Decorations::refresh_overlay_state(model)` snapshots it into
`SelectionLayer`, and `Chrome::autofill_handle(selection_range)` stays
pure by receiving that already-refreshed range from the orchestrator.

### `Chrome` fields

| Field                  | Role                                                                                          |
| ---------------------- | --------------------------------------------------------------------------------------------- |
| `sheet`                | Index of the painted sheet — `is_still_valid` compares against it                             |
| `pane_set`             | `rows` and `cols` `AxisSlots`, each holding frozen/scroll Vecs, a frozen offset, and the snapshotted model bound (see "Slot vec shape" below) |
| `row_header_thickness` | Width of the row-number strip, measured in Phase C                                            |
| `col_header_thickness` | `HEADER_ROW_HEIGHT` today; field reserved for a future dynamic column-header                  |
| `cell_origin`          | `Point { x: origin_x, y: origin_y }` — single source for cell-area top-left                   |
| `canvas_size`          | Size at build time; `is_still_valid` reads it to detect a resize                              |
| `theme`                | `Rc<CanvasTheme>` snapshot — the renderer reads `frame.theme.*` instead of holding a renderer field. `Rc` so `Chrome::next` clones a cheap handle each tick rather than deep-copying every palette string |
| `kind`                 | `FrameKindTag::{Fresh, SlotsReused, Blitted}` — which regime produced this frame. Diagnostics + paint-skip gating read it; `paint_*` arms don't dispatch on it (the regime drives that) |
| `stale_panes`          | `PaneRegionMask` of which panes `render_grid` must paint this frame. `Fresh` / `SlotsReuse` → `ALL`; the blit path narrows to the cross-axis panes the scroll touched (the rest are still-valid pixels from before the blit) |

`Chrome` itself carries no per-pane content or paint-skip state — that
lives on `RendererCore`'s `PaneCache` (`renderer/cache/pane_cache.rs`),
which persists across the `Fresh`/`SlotsReuse`/`Blitted` frames a
renderer paints, independent of any one `Chrome` value's lifetime. See
`docs/rendering-and-damage.md` §3–4 for the pane → row → cell fingerprint
tree, the row-damage repaint planner, and the cache invalidation model it
owns.

### Slot vec shape

```rust
struct RowSlot { row: i32, top: i32, height: i32 }   // top is absolute canvas Y
struct ColSlot { col: i32, left: i32, width: i32 }   // left is absolute canvas X

struct AxisSlots<S: AxisSlot> {        // src/geometry/slot.rs
    frozen: Vec<S>, scroll: Vec<S>, frozen_offset: i32,
    last_id: i32,
}

struct PaneSet {                       // one AxisSlots per axis
    rows: AxisSlots<RowSlot>,
    cols: AxisSlots<ColSlot>,
    row_header_labels: Vec<String>, col_header_labels: Vec<String>,
}
```

Each axis's `frozen_offset` is the absolute pixel coord where the
scrollable band begins (`y` for rows, `x` for cols; = `scroll[0].{top,left}`
when scroll is non-empty, explicit field for the empty-scroll edge case).
`AxisSlots` owns the axis-generic queries (`slot`, `pixel_to_id`,
`boundary_at`, `frozen_count`, `fill`, …); `PaneSet` composes one per axis.

#### Axis-generic walks — the `AxisSlot` trait

`RowSlot` and `ColSlot` both implement the `AxisSlot` trait
(`new` / `id` / `start` / `extent`, with `end` defaulted to
`start + extent`). Every "do this for rows *and* cols" function in
`geometry/slot.rs` is one generic implementation over `<S: AxisSlot>`;
`PaneSet`'s public `fill_rows` / `fill_cols` / `pixel_to_row` /
`pixel_to_col` / `row_boundary_at` / `col_boundary_at` are thin
axis-bind wrappers around them. Adding a new axis-symmetric query
means writing one generic function, not two.

| Helper                       | Role                                                                                                                                                                  |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fill_axis`                  | Phases B / D's slot walker. Pushes slots over an inclusive `range`, breaks **post-push** when `cursor >= max_cursor` (so the last slot is partially visible, not hidden). `max_cursor: Option<i32>` — `None` disables the break (the frozen band always paints in full regardless of viewport size); the scroll band passes `Some(canvas_extent.ceil() as i32)`. Returns the post-band cursor — Phase B/D use that for `frozen_offset_{y,x}` |
| `scroll_first`               | Effective scroll start: `(frozen_count + 1).max(view_first)`. The "scroll band starts where frozen ends or where the user scrolled to, whichever is further" invariant. Called from Phase B/D *and* `is_still_valid` so the cache check matches build-time decisions exactly |
| `slot_at`                    | `id → &Slot` lookup across the frozen-then-scroll pair. Frozen ids are 1-indexed; scroll ids index from `scroll.first().id()`                                          |
| `top_id` / `last_visible_id` | Scroll-band id accessors with empty-band fallbacks. Used by `pane_set.top_row()` / `last_row()` and by `screen_for_blit`'s viewport-shift computation                  |
| `pixel_to_id`                | Linear scan frozen-then-scroll for the slot covering a canvas pixel. Powers `hit_test`'s row/col resolution                                                            |
| `boundary_at`                | Snap a pixel to a slot's trailing edge when within `hit_zone`. Powers `resize_handle_at`. Breaks when a slot's end passes `pixel + hit_zone` — slot vecs are monotonic, so no later slot can match |
| `row_height` / `col_width`   | Model reads with `DEFAULT_ROW_HEIGHT` / `DEFAULT_COL_WIDTH` fallback. Called from `fill_axis`'s `measure` closures and from `chrome/blit.rs`'s overlap-extent checks    |

### Cached-frame validity — `is_still_valid` & `screen_for_blit`

Two separate methods on `Chrome` answer "can we skip the full
rebuild?" — they cover different fast paths and are called in
sequence by the orchestrator.

#### `is_still_valid(model, size) -> FrameValidity`

Returns an enum, not a bool. The verdict gates the `SlotsReuse` vs
`Fresh` choice; `Overlay` is gated separately by overlay-only signal
bits, not by this method.

| Verdict                     | Meaning                                                                                       | Picks regime    |
| --------------------------- | --------------------------------------------------------------------------------------------- | --------------- |
| `FrameValidity::SlotsReuse` | Same canvas size, sheet, view top/left, frozen counts. Slot vecs are usable as-is.            | `SlotsReuse`    |
| `FrameValidity::Rebuild`    | Any of the four inputs diverged. Slot vecs must be walked fresh from the model.               | `Fresh` (unless `screen_for_blit` qualifies it as `Blit`) |

The four inputs:

| Input             | Source                                 |
| ----------------- | -------------------------------------- |
| `canvas_size`     | `IronCanvas` size setter               |
| `top_row` / `left_column` | `model.get_selected_view()`     |
| `rows.frozen_count()` / `cols.frozen_count()` | `model.get_frozen_*_count(sheet)` |
| `sheet`           | `model.get_selected_sheet()`           |

The `top_row` / `left_column` check uses the **effective** scroll
start `(frozen_count + 1).max(view.top_row)` (and column mirror)
compared against `pane_set.top_row()` / `left_column()` (each reads
`rows.scroll.first()` / `cols.scroll.first()`), so a view scrolled
inside the frozen band (`view.top_row <= frozen_count`) matches the
painted state and stays `SlotsReuse`. Selection changes are
overlay-only — they never invalidate the slot vecs. Theme changes
force a full `Fresh` repaint: `Orchestrator::set_theme` drops
`last_frame`, invalidates the renderer paint cache, and raises
`STRUCTURAL | OVERLAY` on both layers. Without the cache invalidation
`SlotsReuse` would repaint cells in the old palette under fresh
chrome — `is_still_valid` doesn't check theme and the per-cell pixel
cache holds resolved hex colors from the previous theme.

`Decorations::refresh_overlay_state(model)` refreshes
`SelectionLayer.selection_range` / `SelectionLayer.active_cell` from
`model.get_selected_view()`, then mirrors `selection_range` into
`AutofillLayer`. The `Overlay` arm calls it first; the grid-painting
arms call it after the grid paint, before the overlay paint. Other
decorations read their own live state directly inside `Layer::paint`.

#### `screen_for_blit(model, canvas, theme, active_cell: &ActiveCellSnapshot) -> Option<BlitPlan>`

Runs as a follow-up after `is_still_valid` returns `Rebuild` — a
structural divergence might still be a pure scroll. `active_cell` is
sourced from `SelectionLayer.active_cell` (the snapshot stamped at
the end of the previous paint by `refresh_overlay_state`); `decide()`
only calls `screen_for_blit` when the layer's `active_cell.is_some()`.
Returns `Some(BlitPlan)` only when **all** of these hold:

- canvas size unchanged
- theme unchanged
- sheet unchanged
- frozen-row / frozen-col counts unchanged
- `ActiveCellSnapshot` re-hashed against the live cell still matches
- viewport shifted along exactly one axis (no two-axis scroll)
- the kept band is non-empty after the shift, with row/col extents
  along the kept axis matching what the previous frame painted

On `Some`, the orchestrator calls `Chrome::next_blit(.., &plan)`, which
returns a `BlitOutcome` (`Blitted` on in-place reuse, `FreshFallback`
when reuse rejects); on `None`, it builds `FramePath::Fresh`. The qualification
mechanics (probing per-pane shifts, building the `Vec<PaneShift>` +
`repaint_strip`) live in `chrome/blit.rs`; `screen_for_blit` itself
is the disqualifier screen.

## Query pipeline

```
iron-canvas-web        →  iron-canvas-core               →  iron-canvas-core
IronCanvas (#[wasm_bindgen])  Orchestrator<S> (pub fn)       Chrome (pub fn)
──────────────────────────────────────────────────────────────────────────────
hit_test(x: f64, y: f64)   →  hit_test(x, y)              →  hit_test(x: i32, y: i32)
cell_rect(row, col)        →  cell_rect(row, col)         →  cell_rect(row, col)
resize_handle_at(x, y, t)  →  resize_handle_at(x, y, t)   →  resize_handle_at(x, y, t)
autofill_handle()          →  autofill_handle()           →  autofill_handle(sel)
pixel_to_cell(x, y)        →  pixel_to_cell(x, y)         →  pane_set.{rows,cols}.pixel_to_id
```

`IronCanvas` is a thin facade — query methods delegate to the
`Orchestrator<FacadeSurface>` it owns. The
orchestrator round-trips `f64` inputs through `x.round() as i32` and
bails with the absent variant (`HitTest::Outside` / `None`) when
`last_frame` is `None`. After the first paint, every query reads the
cached `Chrome` directly. `fit_column_width` / `fit_row_height`
(auto-fit measurement) also live on the orchestrator but read the
model + painter metrics rather than `last_frame`.

### `hit_test(x, y) -> HitTest`

Two stages. The orchestrator first walks the decorations front-to-back
(`Decorations::hit_order()`: formula-refs → point-mode → clipboard →
autofill → selection — the one place this precedence is written); the
first layer to return `Some` wins. Only when every decoration passes
does the query fall through to `Chrome::hit_test`, the pure geometric
tree, where header zones take priority over cell zones at the
cell-area boundary:

```
1. x < 0 || y < 0                                       → Outside
2. x < cell_origin.x && y < cell_origin.y               → Corner
3. y < cell_origin.y    (above cell area)               → ColumnHeader(c) | Outside
4. x < cell_origin.x    (left of cell area)             → RowHeader(r) | Outside
5. pixel_to_row(y) and pixel_to_col(x) both resolve     → Cell { row, column }, else Outside
```

`Chrome::hit_test` never returns `AutofillHandle` or `FormulaRef` —
both come from the decoration walk, so a formula-ref hit beats the
autofill handle where their pads overlap, and a decoration pad that
bleeds past the cell area (the 8-px ref pad on a row-1 ref) beats the
header zone underneath.

`AutofillLayer::hit_test` resolves the handle against
`autofill_handle_rect ± AUTOFILL_HIT_PAD_PX`. The handle visually
protrudes into the cell to the bottom-right of the selection's
`(r2, c2)`; the row/column carried in `AutofillHandle` are the
drag-target cell's own coords, while the variant says "begin autofill"
rather than "select this cell". `pixel_to_{row,col}` are linear scans
over the slot vecs (frozen first, scroll second).

`FormulaRefsLayer::hit_test` iterates its painted `Vec<FormulaRef>` in
reverse-paint order (last painted wins overlap), skips non-`Direct`
kinds and refs on other sheets, and calls
`classify_ref_zone(rect, x, y, REF_HANDLE_HIT_PAD_PX)` against each
`frame.range_rect(...)`. Zone precedence is `Corner > Edge > Body`: a
pointer inside the corner-pad of two intersecting edges classifies as
`Corner`, never as `Edge`. The returned `RefZone` drives the host's
drag handler (`Body` → move, `Edge(Side)` → single-axis resize,
`Corner(RectCorner)` → two-axis resize).

### `cell_rect(row, col) -> Option<PixelRect>`

```rust
if !pane_set.row_in_frame(row) || !pane_set.col_in_frame(col) { return None; }
Some(PixelRect {
    top_left: Point { x: col_to_x(col), y: row_to_y(row) },
    width:    col_extent_at(col),
    height:   row_extent_at(row),
})
```

`None` for any cell outside the visible region (frozen band ∪ scrollable
band). No extrapolation — callers that need an off-frame cell must
trigger a repaint first.

### `resize_handle_at(x, y, tolerance) -> Option<ResizeTarget>`

Probes **header strips only**:

```
y < col_header_thickness && x > row_header_thickness
    → pane_set.cols.boundary_at(x, tolerance).map(ResizeTarget::ColumnEdge)
x < row_header_thickness && y > col_header_thickness
    → pane_set.rows.boundary_at(y, tolerance).map(ResizeTarget::RowEdge)
otherwise                                       → None
```

Cell area returns `None` even when the cursor sits over a column edge —
resize is a header-strip affordance. Corner returns `None` (neither
inequality holds). `tolerance` is caller-controlled (in `i32` CSS
pixels) because it tracks cursor styling, not paint geometry. The
`ResizeTarget::{RowEdge,ColumnEdge}(i32)` index is the row/col whose **trailing
edge** the cursor is near — dragging right enlarges that row/col.

### `autofill_handle() -> Option<Point>`

```rust
let n = selection_range.normalized();
// pane_set.{rows,cols}.last_id — the model bound snapshotted at fill time
if n.r2 >= rows.last_id || n.c2 >= cols.last_id { return None; } // at grid end
if !row_in_frame(n.r2) || !col_in_frame(n.c2) { return None; } // off-frame
Some(Point {
    x: col_to_x(n.c2) + col_extent_at(n.c2),
    y: row_to_y(n.r2) + row_extent_at(n.r2),
})
```

Position-only query for callers that need the handle's coordinates
(e.g. drag-start state). For "is the cursor *over* the handle" use
`hit_test` and match `HitTest::AutofillHandle` instead — the two queries
are not interchangeable because `hit_test` applies `AUTOFILL_HIT_PAD_PX`
and `autofill_handle` does not.

### `range_rect(range) -> Option<PixelRect>`

Maps a sheet-coordinate `RCRange` to canvas pixel bounds. `pub` on
`Chrome` — consumed by overlay paint (selection rectangle, formula-ref
outlines) inside the engine, so it does not appear in the
`IronCanvas → Orchestrator → Chrome` table above.

```rust
if !range_intersects_fold(range, frozen_rows, frozen_cols) { return None; }
let x = col_to_x(range.c1);
let y = row_to_y(range.r1);
let right  = if range.c2 > last_visible_col() && range.c2 > frozen_cols
             { canvas_size.w as i32 }
             else { col_to_x(range.c2) + col_extent_at(range.c2) };
let bottom = if range.r2 > last_visible_row() && range.r2 > frozen_rows
             { canvas_size.h as i32 }
             else { row_to_y(range.r2) + row_extent_at(range.r2) };
Some(PixelRect { top_left: Point { x, y }, width: right - x, height: bottom - y })
```

`range_intersects_fold` (private) guards the slot lookups against
out-of-fold refs like `=BB3` when column BB is off screen — without it
the `col_to_x` / `row_to_y` calls would fall through to the `0`
defaults and emit a rect at the canvas origin. The canvas-edge clamp
on the right/bottom keeps an over-extending selection painting as a
continuous block to the edge, rather than ending at the last visible
slot's boundary.

## Dispatch & paint regimes

`IronCanvas::paintIfDirty` (in `iron-canvas-web`) delegates to
`Orchestrator::paint_if_dirty` (in `iron-canvas-core`). With
`--features dev-tools`, the facade also short-circuits playback and
brackets the delegate call with recording capture; the regime decision
still lives entirely in the orchestrator.
The orchestrator is the per-tick entry point: it drains the typed
`GridSignals` from both layers via `drain_signals()`, unions them,
early-exits when nothing is dirty or no model is set, then calls
`decide()` to pick a `PaintRegime` and dispatches to one of five
`paint_*` arms. The `match` is exhaustive — adding a regime breaks
the build, by design.

`paint_if_dirty(&mut self) -> PaintResult` reports what the tick did:
`Idle` (nothing dirty, or no model set — both early exits above),
`Painted` (the dispatched arm committed), or `Retry` (the attempt was
held back rather than committed — see "Paint/query coherence" below
for what "held" means per regime; the caller must keep calling
`paint_if_dirty` until it stops seeing `Retry`). `PaintResult` is
deliberately not `#[must_use]` — a permanent polling loop that ignores
it still behaves correctly, since a held attempt keeps re-raising its
own signals. `pending_content` (and `pending_damage`) are cleared at
the end of `paint_if_dirty` only when the result is *not* `Retry`; on
`Retry` the arm has already folded the failed scope back into
`pending_content` / `pending_damage` itself, and the orchestrator
re-raises the drained `GridSignals` on the grid layer so the next tick
re-enters `decide()` with no new external signal.

### `GridSignals` — typed dirty bits

`bitflags::bitflags! struct GridSignals: u8`:

| Flag         | Raised by                                                   | Meaning                                       |
| ------------ | ----------------------------------------------------------- | --------------------------------------------- |
| `VIEWPORT`   | *(reserved — no setter today)*                              | Scroll changed. Currently inferred geometrically by `screen_for_blit`; flag exists for a future typed scroll-changed setter |
| `CONTENT`    | `markContentDirty` / `mark_content_dirty`, `markRowsDamaged` / `mark_rows_damaged` | Cell value changed; cached buffers stale      |
| `STRUCTURAL` | `set_theme*`, `set_model`, `requestRepaint`, `resize`       | Sheet/freeze/size/theme/model identity change |
| `OVERLAY`    | `set_overlays`, `request_overlay_repaint`, `requestRepaint`, `resize` | Selection / formula-ref / autofill drag       |

Layers accumulate signals via `raise(bits)` and surrender them via
`drain_signals()`. The bit layout mirrors `PaneRegionMask` so
overlay/grid bits can coexist cleanly in one `u8`.

`requestRepaint` (the JS blanket setter) drops `last_frame` and clears
`pending_content` (and `pending_damage`), so the next paint always
dispatches `Fresh` — the `Overlay` / `Viewport` / `Damage` / `SlotsReuse`
arms all gate on a surviving `last_frame`. It raises `STRUCTURAL | OVERLAY`
and explicitly **not** `CONTENT`: that bit is reserved for real cell-value
changes via the typed `markContentDirty` / `markRowsDamaged` setters,
which route the per-pane refetch (`markRowsDamaged` additionally names
the changed rows so `decide` can try the cheaper `Damage` arm).

### `decide()` — the regime cascade

`decide(sig, model) -> PaintRegime` is pure over `&self`; arm methods
own the mutation. Decisions cascade in **cheapness order** — only the
first matching rule wins:

1. **`Overlay`** — `!grid_dirty && overlay_dirty && validity == SlotsReuse && last_frame.is_some()`.
   No grid bits, just overlay-affecting state. No `Chrome::next` call;
   reuse `last_frame` directly.
2. **`Viewport(plan)`** — `!content_dirty && screen_for_blit returns Some(plan)`.
   Pure single-axis scroll qualified by the 7-bullet `screen_for_blit`
   disqualifier list. The `!CONTENT` gate is load-bearing: a blit over
   stale content would propagate pre-edit pixels (the recalc-bug
   class, fixed in commit `25d91d2`).
3. **`Damage { spans, signals }`** — `content_dirty && !STRUCTURAL && validity == SlotsReuse`
   and every `CONTENT` raise since the last paint named its rows via
   `mark_rows_damaged` against the sheet still on screen (`pending_damage
   == CellDamage::Rows { sheet, spans }` matching `last_frame.sheet`).
   Repaints only those full-width row bands through
   `Chrome::next(.., FramePath::SlotsReuse { stale_panes: EMPTY })` +
   `paint_grid_damage`. A plain `mark_content_dirty` call, a cross-sheet
   or >8-span damage set, or an un-rowed content raise poisons
   `pending_damage` and falls through to `SlotsReuse` instead.
4. **`SlotsReuse { mask, signals }`** — `validity == SlotsReuse && last_frame.is_some()`.
   `mask = pending_content` when `content_dirty && !pending_content.is_empty()`,
   else `ALL` (theme touches every pane uniformly, so a theme-only
   signal gets `ALL`; an empty `pending_content` likewise falls back
   to `ALL`). `signals` carries the full `GridSignals` word so the
   arm can check `signals.overlay_dirty()` to decide whether to
   repaint the overlay after the grid.
5. **`Fresh(GridSignals)`** — fallback. First paint, or structural
   divergence not qualifying as a blit, or no `last_frame`. Recycles
   slot Vec allocations from prev via `RecycledSlots` when possible.
   The arm inspects `signals.overlay_dirty()` / `signals.contains(CONTENT)`
   for the same reasons.

The `PaintRegime` enum carries everything its arm needs (mask, dirty
bits, plan), so the dispatch `match` body in `paintIfDirty` is pure
pattern-destructure.

### `Orchestrator<S>` fields

State that used to live on `IronCanvas` now lives on `Orchestrator<S>`
in `iron-canvas-core`. `IronCanvas` (in `iron-canvas-web`) keeps only
facade state — `orch: Orchestrator<FacadeSurface>`, the cached `model`
for export re-push, `last_dpr`, and the dev-tools `mode: CanvasMode`
(live / recording / playback) — and delegates every setter / query /
paint call to `orch`.

All fields are crate-private (`pub(crate)` or weaker). External access is
through the public setter / query methods; the only introspection surface
is `grid_surface()` / `overlay_surface()`, gated behind the
`surface-introspection` feature, for the recorder integration tests and
the web facade's recording bracket that inspect emitted `DrawOp`s.

| Field             | Role                                                                                                                              |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `grid`            | `LayerBase<S, GridRenderer<S::P>>` — the bottom surface; paints the worksheet pixels. `pub(crate)`                                |
| `overlay`         | `LayerBase<S, OverlayRenderer<S::P>>` — the top surface; iterates `Layer` decorations. `pub(crate)`                               |
| `theme`           | `Rc<CanvasTheme>` — wrapped once on `set_theme` and `Rc::clone`d into each `Chrome`. Setters value-compare before raising; pushing identical theme is a no-op                                          |
| `decos`           | `Decorations` — owns `SelectionLayer`, `AutofillLayer`, `ClipboardLayer`, `PointModeLayer`, `FormulaRefsLayer` as one group. Exposes `active_cell_repaint()` and `refresh_overlay_state(model)` for the orchestrator; `overlay_slice()` and `hit_order()` are the single source of paint/hit-test ordering. `set_overlays(RenderOverlays)` does a bulk compare of all four overlay inputs, returning `true` so the caller raises `OVERLAY` once |
| `model`           | `Option<Rc<dyn CanvasModel>>`. Single type param (no `M` on `Orchestrator<S>`). Any `CanvasModel` impl (`JsBackedModel`, Leptos-side adapters) routes through. Workbook swaps reuse the same `Rc` handle |
| `last_frame`      | `Option<Chrome>`. Owned single-threadedly by `paint_if_dirty`; queries (`hit_test`/`cell_rect`/…) read it. `None` before first paint |
| `size`            | `CanvasSize` (logical CSS). Written by `resize`; read by `Chrome::next` and `is_still_valid`                                     |
| `pending_content` | `PaneRegionMask` accumulating between `mark_content_dirty` calls; `PaneRegionMask::EMPTY` is the "nothing pending" state. Consumed by `SlotsReuse`'s `mask`. Reset to `EMPTY` at the end of `paint_if_dirty` when the result is `Painted` or `Idle`; a `Retry` instead overwrites it with the held scope (see "Paint/query coherence" below) |
| `pending_damage`  | `CellDamage` paired with `pending_content`. It stays row-specific only while every queued content raise names compatible rows; otherwise it becomes `Exceeded` and the dispatcher uses `SlotsReuse` |
| `last_regime`     | `Option<PaintRegimeTag>` — data-free mirror of the regime `paint_if_dirty` last dispatched. Stamped after `decide()`, before the arm runs. Read by the recording pipeline via `last_regime()`; `None` before first paint |
| `last_signals`    | `GridSignals` — the word the last `paint_if_dirty` acted upon. Empty before first paint. Read by the recording pipeline to stamp `Frame.signals` |
| `last_trace`      | `FrameTrace` — regime, signal word, per-pane `PaneVerdict`, whole-frame outcome, blit fallback, and fetched-cell-slot count from the last paint. Exposed read-only by `last_trace()` |

`FrameTrace` is diagnostic state, not a second dispatcher. The renderer
records `PaneVerdict::{Skip, Rows, Full, Strip, Held}` and model-fetch
traffic while the existing paint arm runs; the orchestrator stamps the
regime and signals afterward. A failed blit preflight records
`HeldOnBridgeFailure` and leaves both pixels and pane caches untouched.

## Paint/query coherence

The coherence invariant: by the time a query runs, `last_frame` is
the snapshot the last paint emitted from. Hit zones cannot disagree
with painted pixels because there is no second coordinate path to
disagree with.

`last_frame: Option<Chrome>` lives on `Orchestrator<S>`. `Overlay`
keeps the existing `Chrome` in place — no rebuild, nothing that can
fail. The other four arms write into it once the attempt is decided,
but a held attempt (`PaintResult::Retry`) writes differently
depending on regime — the held-attempt rule:

- **`Viewport` is a whole-frame rollback.** A held blit preflight
  (`paint_grid_blit` returning `BlitPaint::Held`) has painted nothing,
  so `last_frame` is restored to a `Clone` of the pre-attempt `Chrome`
  taken before the attempt — the snapshot matches exactly what is
  still on screen, and neither layer presents.
- **`Damage` / `SlotsReuse` / `Fresh` commit pane-locally.** The new
  `Chrome` — geometry, scroll position, committed slot vecs — always
  replaces `last_frame`, even when some panes' model fetch failed this
  tick. Only the successfully painted panes present; a held pane keeps
  showing its own prior pixels (correct, since it didn't repaint)
  while its scope is folded back into `pending_content` (`Damage`
  also into `pending_damage`) for the next `paint_if_dirty` to retry.

Queries (`hit_test`, `cell_rect`, `range_rect`,
`resize_handle_at`, `autofill_handle`) read `last_frame` via
`as_ref()` — they never trigger a rebuild themselves, so they always
see one coherent snapshot: either the fully-committed new frame, or
(on a rolled-back `Viewport` hold) the untouched previous one — never
something in between.

Before the first paint, `last_frame` is `None` and every query
returns its absent variant. `resize` also drops `last_frame` whenever
the new size or DPR actually differs from the current one — a
same-size, same-DPR call is a no-op — because a backing-store resize
can clear both canvases, so geometry invalidation must be atomic with
the resize itself; the next `paint_if_dirty` then dispatches `Fresh`.

## File map

Paths are relative to the workspace root (`iron-canvas/`). The
`iron-canvas-core/` column carries the engine; `iron-canvas-web/`
carries the wasm-bindgen facade, and the Canvas-2D backend it
re-exports lives in `iron-canvas-canvas2d/`.

### `iron-canvas-core` (engine)

| File                                                  | Owns (for these pipelines)                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/iron-canvas-core/src/chrome/mod.rs`           | `Chrome` struct (`pub`), public `Chrome::next` (2-way dispatch on `FramePath`: `Fresh` / `SlotsReuse`) plus `Chrome::next_blit(.., &BlitPlan) -> BlitOutcome` (the separate blit constructor; `BlitOutcome::{Blitted, FreshFallback}`), both routed through private `Chrome::build` (phases A–E for the `Fresh` arm), `ActiveCellSnapshot` (stores row/col + `value_hash: CellValueHash` used by `screen_for_blit` to detect active-cell content drift without reading the model), `CellValueHash(u64)` newtype (blocks accidental cross-process comparison at the type level — `DefaultHasher` is per-run seeded), `FrameValidity` enum, `is_still_valid`, `screen_for_blit` (scroll-blit screening), query methods (`hit_test`, `cell_rect`, `range_rect`, `resize_handle_at`, `autofill_handle`, `autofill_handle_rect`)                                                            |
| `crates/iron-canvas-core/src/chrome/pane_set.rs`      | `PaneSet` composes `rows: AxisSlots<RowSlot>` and `cols: AxisSlots<ColSlot>` plus resolved header-label Vecs; `measure_row_header_width` (Phase C). `impl PaneSet` carries `with_recycled`, `fill_rows`, `fill_cols`, and readable row/column wrappers for extent/position/frame membership; generic pixel and boundary queries live on `AxisSlots` |
| `crates/iron-canvas-core/src/chrome/blit.rs`          | Scroll-blit fast-path: `FramePath` enum (`Fresh` / `SlotsReuse` — the blit variant moved out to `Chrome::next_blit`), `BlitPlan` + `PaneShift`, qualification (`try_blit_rows` / `try_blit_cols` invoked from `Chrome::screen_for_blit`), in-place Chrome reuse (`try_blit_reuse`, now returning `Result<Chrome, Chrome>` — `Err` hands `prev` back unconsumed). `Chrome::next_blit` (in `chrome/mod.rs`) wraps it into a `BlitOutcome`: `Ok` → `Blitted`, `Err` (e.g. row-header digit boundary 99→100 widens the cross-axis origin) → `FreshFallback` (rebuilt `Fresh`). `paint_viewport_regime` matches the `BlitOutcome` variant so the demoted path gets full cache invalidation instead of stale per-pane caches |
| `crates/iron-canvas-core/src/chrome/blit_rebuild.rs`  | Private module (extracted from `pane_set.rs`). `probe_axis_shift` (measures kept-band slot extents to qualify a single-axis scroll for blit), `rebuild_axis_slots` (re-populates the scroll-band slot vec around a `BlitPlan`), `overlaps_match` (verifies kept-band extents match the previous frame). Consumed by `blit.rs` — not imported directly by `mod.rs` |
| `crates/iron-canvas-core/src/chrome/recycled_slots.rs`| Private module (extracted from `pane_set.rs`); `RecycledSlots` is re-exported as `chrome::RecycledSlots`. — reuses previous-frame `Vec<RowSlot>` / `Vec<ColSlot>` allocations when building a `Fresh` frame, avoiding dealloc/realloc churn. Fed into `PaneSet::with_recycled` in Phase B/D |
| `crates/iron-canvas-core/src/chrome/kind.rs`          | Private module. `FrameKindTag::{Fresh, SlotsReused, Blitted}` — runtime tag stamped onto every `Chrome` by `Chrome::next`; renderer diagnostics + per-pane fingerprint gating read it. Re-exported as `chrome::FrameKindTag`                                                                                                                                                                                                                     |
| `crates/iron-canvas-core/src/chrome/pane_region.rs`   | Private module. `PaneRegion` enum (`TopLeft`/`TopRight`/`BottomLeft`/`BottomRight`); `range(frame)` returns the address-space `RCRange` a pane spans; `PaneRegionMask` bitset used by `stale_panes`. Re-exported as `chrome::{PaneRegion, PaneRegionMask}`                                                                                                                                                                                       |
| `crates/iron-canvas-core/src/geometry/slot.rs`        | `RowSlot`, `ColSlot`, `AxisSlot`, and `AxisSlots<S>`. Each axis owns frozen/scroll Vecs, `frozen_offset`, and `last_id`; axis-generic fill and query helpers live here with `row_height` / `col_width` fallbacks |
| `crates/iron-canvas-core/src/geometry/constants.rs`   | `HEADER_ROW_HEIGHT`, `HEADER_COL_WIDTH`, `HEADER_SEPARATOR_WIDTH`, `CELL_AREA_INSET`, `FROZEN_SEP`, `AUTOFILL_HANDLE_PX`, `AUTOFILL_HIT_PAD_PX`, `REF_HANDLE_HIT_PAD_PX` (wider than `AUTOFILL_HIT_PAD_PX` because the ref overlay has no visible knob to aim at), `LAST_ROW`, `LAST_COLUMN`, `DEFAULT_ROW_HEIGHT`, `DEFAULT_COL_WIDTH`                                                                                                                                                                                                                                      |
| `crates/iron-canvas-core/src/types/ui.rs`             | `HitTest` enum (`Cell`, `RowHeader`, `ColumnHeader`, `Corner`, `AutofillHandle`, `FormulaRef { ref_idx, zone, grab_row, grab_col }`, `Outside`); `RefZone` enum (`Body`, `Edge(Side)`, `Corner(RectCorner)`) — precedence `Corner > Edge > Body`; `Side` (`Top`/`Right`/`Bottom`/`Left`); `RectCorner` (`TopLeft`/`TopRight`/`BottomLeft`/`BottomRight`); `ResizeTarget` enum (`RowEdge(i32)`, `ColumnEdge(i32)`)                                                                                                                                                                                                                                                                                                        |
| `crates/iron-canvas-core/src/types/coord.rs`          | `RCRange`, `SheetArea`, `AutofillTarget`, `FormulaRef`, `FormulaRefKind` (closed origin enum: `Direct` / `DefinedName` / `Unresolved`; `Direct` is the only draggable kind per the REF_DRAG plan). Used by `selection_range`, query inputs, and overlay state                                                                                                                                                                                |
| `crates/iron-canvas-core/src/orchestrator.rs`         | Private module (`Orchestrator<S>`, `PaintRegime`, `PaintRegimeTag`, `PaintResult` re-exported at crate root). `Orchestrator<S: Surface>` (single type param; model is `Option<Rc<dyn CanvasModel>>` field) + the `PaintRegime` dispatch: `paint_if_dirty` entry, `decide()` cascade, five `paint_*_regime` arms (`paint_overlay`, `paint_viewport`, `paint_damage`, `paint_slots_reuse`, `paint_fresh`). `PaintRegimeTag` is the data-free public mirror (`Overlay`/`Viewport`/`SlotsReuse`/`Fresh`/`Damage`, serde `snake_case`) stamped into `last_regime` after `decide()` so out-of-engine consumers (recording) can attribute frames without seeing `BlitPlan`/`PaneRegionMask`/`GridSignals`. Setters (`set_theme*`, `set_model`, `set_overlays`, `mark_content_dirty`, `mark_rows_damaged`, `request_repaint`, `request_overlay_repaint`); `set_overlays` delegates to `Decorations::set_overlays` which does a single bulk compare of all four overlay fields before returning `true` — folds 4 per-field setters into 1 signal raise; query glue (`hit_test`, `cell_rect`, `resize_handle_at`, `autofill_handle`) delegates into `Chrome`. `set_theme` on real change drops `last_frame`, calls `grid.invalidate_paint_cache()`, and raises `STRUCTURAL | OVERLAY` on both layers — the per-cell paint cache holds the old palette and `is_still_valid` doesn't see theme changes, so without these the next paint reuses stale-color cells. `Decorations::refresh_overlay_state(model)` refreshes `SelectionLayer` from the model + mirrors `selection_range` into `AutofillLayer`; the `Overlay` arm calls it first, the grid-painting arms after the grid paint, before the overlay paint. `grid_surface()` / `overlay_surface()` accessors (returning `&S`), gated behind the `surface-introspection` feature, used by both the recorder integration tests and the web facade's recording bracket — feature-gated because the inner-surface handle is an engine internal, not a public knob |
| `crates/iron-canvas-core/src/render_overlays.rs`      | Private module (`RenderOverlays` re-exported at crate root). Input bag for the overlay layer: `extend_to`, `clipboard`, `point_range`, `formula_refs: Vec<FormulaRef>` (each `FormulaRef` carries its own `sheet_area` / `color_idx` / `kind`; there is no separate active-index field) |
| `crates/iron-canvas-core/src/layer/mod.rs`            | `Surface` trait (`type P: Painter + BlitPainter`, `painter`, `clone_painter`, `resize`, `present`), `PaintGate`, and `LayerBase<S, R>`. Grid specializations are `paint_grid`, `paint_grid_blit`, and `paint_grid_damage`; a blit frame fetches and bridge-validates every strip/full-pane fallback before shifting any pixel, then calls `present` only after the paint arm finishes |
| `crates/iron-canvas-core/src/renderer/mod.rs`         | `RendererCore`, `GridRenderer`, and `OverlayRenderer`; renderer-lifetime pane/intern caches and blit staging; per-frame `FrameTrace` collection. `render_grid`, `render_grid_blit`, and `render_grid_damage` drive pane painting and the five cell passes |
| `crates/iron-canvas-core/src/renderer/cache/pane_cache.rs` | Per-pane bulk-fetch buffers plus `PaneFingerprintState` (`painted` and reusable `scratch` trees). `classify_shift` is the mutation-free half used by blit preflight; `prepare_shift` rotates kept-band buffers only after preflight succeeds |
| `crates/iron-canvas-core/src/renderer/cell/fingerprint.rs` | Pane → row → cell fingerprint tree and `RepaintPlan::{Skip, Rows, Full}`. Equal pane digests skip all five cell passes; bounded row spans select damage painting; unsafe border or span cases select full-pane repaint |
| `crates/iron-canvas-core/src/renderer/cell/mod.rs`    | Pane fetch, bridge-failure hold, fingerprint planning, full/row/strip paint, and blit preflight. A validated unshiftable full-pane fetch is staged and adopted by `render_pane` rather than fetched twice |
| `crates/iron-canvas-core/src/renderer/blit_work.rs`   | `BlitPaneWork` and pixel-clip widening for revealed strips. The main `BottomRight` pane carries the repaint clip; frozen-band siblings use narrowed address ranges |
| `crates/iron-canvas-core/src/decoration/mod.rs`       | `Layer` trait (`group(&self) -> GroupClass`, `paint(&self, frame, painter)`, `hit_test(...)` — default-None) + `RepaintActiveCell` payload (produced via `SelectionLayer::active_cell_repaint()`, not the trait). Decoration iteration lives in `OverlayRenderer::paint_overlay_layer` (`layer/mod.rs`), which wraps each decoration in a `begin_group(layer.group()) / end_group()` bracket in fixed z-order. Top-level sibling of `layer/` — not nested inside it |
| `crates/iron-canvas-core/src/decoration/selection.rs`   | `SelectionLayer` — owns `selection_range: Option<RCRange>` + `active_cell: Option<ActiveCellSnapshot>`. `paint` paints the fill, `active_cell_repaint() -> Option<RepaintActiveCell>` returns the cell the renderer must repaint between phases (Some only when the model has a selected view), `paint_stroke` paints stroke + autofill handle. The three-phase orchestration lives in `paint_overlay_layer` (fill → optional active-cell repaint → stroke → optional header highlights), not in the trait |
| `crates/iron-canvas-core/src/decoration/autofill.rs`    | `AutofillLayer` — autofill drag target state + preview paint                                                                                                                                                                                                                                                                                                                                                                         |
| `crates/iron-canvas-core/src/decoration/clipboard.rs`   | `ClipboardLayer` — clipboard marching-ants overlay                                                                                                                                                                                                                                                                                                                                                                                   |
| `crates/iron-canvas-core/src/decoration/point_mode.rs`  | `PointModeLayer` — point-mode range highlight                                                                                                                                                                                                                                                                                                                                                                                        |
| `crates/iron-canvas-core/src/decoration/formula_refs.rs`| `FormulaRefsLayer` — formula-ref multi-color outlines, plus `Layer::hit_test` for drag affordances. Paint reads `refs: Vec<FormulaRef>`. `hit_test` walks `refs` in reverse-paint order, skips non-`Direct` kinds and refs on other sheets, and runs `classify_ref_zone(rect, x, y, REF_HANDLE_HIT_PAD_PX)` to return `HitTest::FormulaRef { ref_idx, zone, grab_row, grab_col }` — `Corner > Edge > Body` precedence in one classifier |
| `crates/iron-canvas-core/src/signal.rs`               | `GridSignals` bitflags (`VIEWPORT` / `CONTENT` / `STRUCTURAL` / `OVERLAY`) — typed dirty bits accumulated by each layer's `raise(bits)`, drained by `paint_if_dirty`, fed into `Orchestrator::decide`. `grid_dirty()` / `overlay_dirty()` predicates short-circuit the cascade                                                                                                                                                                  |
| `crates/iron-canvas-core/src/model_adapter.rs`        | Two read-only model traits. **`CellContentQuery`** is the per-cell content slice the cell painter consumes via `&dyn CellContentQuery`: the single accessors (`get_cell_style`, `get_cell_type`, `get_formatted_cell_value`, `get_extended_cell_style`) return `Fetched<T>` = `Value`/`Absent`/`BridgeFailed`, so a transient bridge failure is a named variant distinct from an empty cell — the single-cell active-cell repaint skips entirely on `BridgeFailed` rather than flashing blank; the bulk `*_in` now return `Vec<Fetched<T>>` too (migrated from `Vec<Option<T>>` so a per-cell `BridgeFailed` survives the bulk channel; the default `*_in` impls loop the single accessor, forwarding each `Fetched` verbatim). The pane-cache's take-able scratch consumes slots via `Fetched::take_value` — mirrors `Option::take` (leaves `Absent` behind, collapses `BridgeFailed`→`None`, since the hold decision already happened upstream in the preflight). **`CanvasModel: CellContentQuery`** extends it with the sheet-level config/selection accessors used during `Chrome::build` (`get_selected_view`, `get_selected_sheet`, `get_frozen_rows_count`, `get_frozen_columns_count`, `get_row_height`, `get_column_width`, `get_show_grid_lines`, `get_show_row_headers`, `get_show_col_headers`, `get_row_header_text`, `get_column_header_text` — the two header flags default to `true` and let `Chrome::build` collapse a hidden strip's thickness + inset to 0). Splitting the eight content methods out lets `render_pane` narrow to `&dyn CellContentQuery` while `render_grid` keeps the full `&dyn CanvasModel`. Two `Rc<T>` blanket impls (one per trait, via the `forward_methods!` macro) because supertraits aren't auto-derived: `Rc<T>: CanvasModel` requires a real `Rc<T>: CellContentQuery` |
| `crates/iron-canvas-core/src/autofit.rs`              | `fit_width` / `fit_height` — pure Excel-style auto-fit measurement over a caller-supplied used-range span (bounded by `FIT_SCAN_CAP` so a pathological used-range can't stall the canvas), plus `font_css`. Read-only: returns the extent the consumer would apply via a resize; rebuilds font CSS through `renderer::cache::font::escape_font_family` so measurement matches the text pass                                                                                                                            |
| `crates/iron-canvas-core/src/painter/mod.rs`          | Backend-neutral `Painter`, `BlitPainter`, `TextMetrics`, paint types, shapes, and typed `GroupClass` brackets. It also owns `parse_font_size_px` and the flat `approx_text_width` fallback used by the Recorder and by export backends only when their real font tables lack a glyph. Concrete impls stay in adapter crates |
| `crates/iron-canvas-core/src/geometry/pixel_rect.rs` + `prim.rs` | `PixelRect`, `Point`, `Line`, `Span` derive `Serialize`/`Deserialize` so the `.icr` schema can embed them directly without an owned-copy mirror                                                                                                                                                                                                                                                                |

### `iron-canvas-web` (wasm-bindgen facade) + `iron-canvas-canvas2d` (Canvas-2D backend)

| File                                                  | Owns                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/iron-canvas-web/src/orchestrator.rs`          | `IronCanvas` (`#[wasm_bindgen]`) owns `Orchestrator<FacadeSurface>`, cached model and painter handles, `last_dpr: f64`, and dev-tools live/record/playback state. Its JS paint surface includes `resize`, dirty setters, `paintIfDirty`, `fontsChanged`, `frameTrace`, and SVG/PDF export; query and overlay setters delegate to the core orchestrator. `resize` preserves fractional DPR end-to-end. `fontsChanged` clears both Canvas-2D measurement memos and marks all content dirty. `frameTrace` formats the core's last `FrameTrace`. Recording and playback preserve the same `f64` DPR. |
| `crates/iron-canvas-web/src/wire.rs`                  | JS-facing wire-shape mirrors for the `IronCanvas` JS API. `#[cfg(target_arch = "wasm32")] mod wire;` — only compiled into the wasm build. `Serialize` outputs (`HitTestWire`, `RefZoneWire`, `SideWire`, `CornerWire`, `ResizeTargetWire`, `CanvasSizeWire`, `CellCoordWire`) tagged on `kind` with `rename_all = "camelCase"`; `Deserialize` inputs (`RCRangeWire`, `AutofillTargetWire`, `SheetAreaWire`, `FormulaRefKindWire`, `FormulaRefWire`, `RenderOverlaysWire`, `CanvasThemeWire`, `ThemeVariablesWire`). `From<Wire>` impls bridge to the engine types. Exists because (a) engine enums use tuple variants that serde's `tag = "kind"` rejects, (b) `iron-canvas-core` is deliberately kept free of `wasm-bindgen` / `serde-wasm-bindgen` deps, (c) `CanvasSize` is engine-side serde-free per the `.icr` schema's convention. `RenderOverlaysWire::into_engine()` maps the wire bag into the engine `RenderOverlays` (returns `Result<_, String>`; the orchestrator wraps any error in `JsError`). |
| `crates/iron-canvas-canvas2d/src/web_surface.rs`      | `WebSurface` acquires the 2D contexts. The grid paints into a detached back canvas, reads blit source pixels from the visible front canvas, and `present()` copies the completed back buffer 1:1 to the front. The overlay draws directly with `alpha: true, desynchronized: true`; its `present()` is a no-op |
| `crates/iron-canvas-canvas2d/src/canvas_painter.rs`   | `CanvasPainter` — Canvas-2D `Painter` + `BlitPainter` + `TextMetrics`. Holds setter caches, palette intern, `dpr: Cell<f64>`, and a bounded measurement memo. `clear_measure_cache` is separate from paint-state invalidation because font-load events, not ordinary paints, invalidate measured widths |
| `crates/iron-canvas-canvas2d/src/measure_cache.rs`    | Small bounded linear memo keyed by `(font_css, text)`. It avoids repeated JS `measureText` crossings without adding a hash map or unbounded retention |
| `crates/iron-canvas-web/src/wasm/mod.rs`              | `JsBackedModel` — `(catch, method)` shim over IronCalc's `Model` JS handle; once-per-class `console.warn` via `Cell<u64>` throw / serde-shape counters. Overrides the bulk `get_cell_styles_in` / `get_formatted_cell_values_in` / `get_cell_types_in` with one batched extern each (`getCellStylesIn` / `getFormattedCellValuesIn` / `getCellTypesIn`) so a pane fetch crosses the JS boundary **once** instead of per-cell — the one impl with a real boundary now batches most, not least. Capability is probed once in `new()` via `Reflect::has` (**D-1**: a missing `*In` is a static absence, not a throw — never burns the throw counter); when a flag is false the override drops to a named per-cell loop, bit-identical to the old default. A returned array is trusted only past the **D-2** gate: length must equal `(r2-r1+1)*(c2-c1+1)`, else the whole call degrades to per-cell. A `null` element is no longer rejected — it maps to `Fetched::Absent` (a blank cell) for the renderer to paint over `cell_bg`, so a blank-cell pane skips the per-cell refetch (a host may omit blank-cell payloads). The bulk overrides fill `Vec<Fetched<T>>` (the trait's June-2026 bulk-channel migration from `Vec<Option<T>>`); the per-cell degrade path forwards each slot's `Fetched` verbatim. Genuine failures — a throw, a non-array payload, or a wrong-length array — still route to per-cell via the `note_throw`/`note_serde_err` arms + the length check. The host's `getCellStylesIn` must return dxf-merged styles (parity with the per-cell `getCellStyle`). Workbook theme: `Color::Theme(idx, tint)` is resolved bridge-side against a theme fetched lazily via `getTheme` and **cached for the model's lifetime** (probed like the `*In` methods; absent/failing hosts get the Office default). Host contract: after `model.setTheme(...)` the host **must** call `IronCanvas.themeChanged()` — it drops the cached theme and marks content dirty; without it the stale cache silently misrenders theme colors (no error — a host bug). |
| `crates/iron-canvas-web/src/wasm/diag.rs`             | `console_warn` / `console_log` shim                                                                                                                                                                                                                                                                                                       |
| `crates/iron-canvas-canvas2d/src/theme_from_element.rs` | `from_element` / `from_root` — reads CSS `--palette-*` custom properties off a host DOM node and builds a `CanvasTheme`                                                                                                                                                                                                                  |
| `crates/iron-canvas-web/src/playback.rs`              | `PlaybackSession` — owns a parsed `Recording`, wall-clock play anchor, and live canvas size/DPR stash for restore on exit. `target_frame_for(now_ms)` walks forward from the anchor frame to find the frame matching elapsed wall-clock time. `find_fresh_anchor(frames, target)` scans backward for the most recent `Fresh` frame ≤ `target` to anchor cumulative grid replay. `replay_through(grid_painter, overlay_painter, recording, target_idx)` is the core seek algorithm: find Fresh anchor, replay grid ops cumulatively from anchor to target, replay overlay ops per-frame. Generic over `P: Painter + BlitPainter` so it works against bare `CanvasPainter` and the dev-tools `RecordingPainter<CanvasPainter>` alike.                                                                                              |
| `crates/iron-canvas-web/src/replay.rs`                | Standalone viewer entry points: `icrReplayGridOps` / `icrReplayOverlayOps` — `#[wasm_bindgen]` fns that parse a `Vec<DrawOp>` from JSON and dispatch through `iron_canvas_recorder::replay` against a `CanvasPainter`. Used by `web-test/recording-viewer.html`.                                                                                                                                                                                                    |

### `iron-canvas-recorder` (recording producer + test backend)

| File                                                       | Owns                                                                                                                                                                                                                                                                                                                                                                       |
| ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/iron-canvas-recorder/src/lib.rs`                   | `DrawOp` (serde-derived enum mirroring every `Painter`/`BlitPainter` call); `RecorderPainter` (in-RAM op-log painter with `clip_depth`/`group_depth` balance asserts on drop); `MemSurface` (test `Surface` that owns one `RecorderPainter`, no backing pixels); `RecordingPainter<P>` (per-painter decorator that forks every call into a shared op buffer when `enabled` is set; always forwards to inner so production rendering is unaffected); `RecordingSurface<S>` (the Surface decorator the web facade wraps — `begin_frame`/`end_frame` are per-tick buffer ops, `enable_recording`/`disable_recording` flip an `Rc<Cell<bool>>` shared with the painter; `set_skip_groups(HashSet<GroupClass>)` installs the filter set, with matched `begin_group`/`end_group` pairs pruned via the shared `skip_depth: Rc<Cell<u32>>`. Flipping enable mid-frame is unsupported because it could land orphan `push_clip` without `pop_clip` and trip the balance asserts, and mid-frame `set_skip_groups` would corrupt `skip_depth`); `replay(target, &[DrawOp])` for round-trip tests |
| `crates/iron-canvas-recorder/src/recording.rs`             | The `.icr` on-disk schema. `ICR_SCHEMA_VERSION` is `3`: v2 added `IcrHeader::dpr: f64` for playback resize; v3 added the `damage` regime tag. `Frame` stores index/time, regime, signal bits, and grid/overlay `DrawOp`s. The exact-version JSON document remains deliberately uncompressed and inspectable with `jq` |
| `crates/iron-canvas-recorder/tests/golden_fixture.rs` + `tests/fixtures/fresh_paint.icr` + `tests/fixtures/overlay_paint.icr` | Regression sentinels — `fresh_paint.icr` exercises every `DrawOp` variant (grid-layer section brackets: `Cells`/`FrozenSep`/`Headers`/`Corner`); `overlay_paint.icr` exercises all 8 overlay decoration subgroups (`SelectionFill`/`ActiveCellRepaint`/`SelectionStroke`/`HeaderHighlights`/`Autofill`/`Clipboard`/`PointMode`/`FormulaRefs`). `ICR_REGEN=1 cargo test -p iron-canvas-recorder --test golden_fixture` regenerates both fixtures. Doubles as smoke fixtures for the standalone viewer |

### `iron-canvas-export` (multi-format export backends)

| File | Owns |
| ---- | ---- |
| `crates/iron-canvas-export/src/lib.rs` | Feature-gated modules: `common`, `svg` (feature `svg`), `pdf` (feature `pdf`). Both features default-on. Crate-private `drive_once<S: Surface>` helper — the single `new → set_theme → set_model → resize → request_repaint → paint_if_dirty` sequence shared by `SvgSurface::render` and `PdfSurface::render`; overlay-discard policy stays at each call site. |
| `crates/iron-canvas-export/src/common/escape.rs` | `xml_escape` — shared XML entity escaping for SVG. `pdf_string_escape` — PDF literal string escaping. |
| `crates/iron-canvas-export/src/common/color.rs` | CSS-color parser shared by SVG + PDF. (`common/text.rs` is gone — its font-size parser + `DEFAULT_FONT_SIZE_PX` moved to `iron-canvas-core`'s `painter` module in June 2026.) |
| `crates/iron-canvas-export/src/common/metrics.rs` | Real output-font metrics: embedded Inter TTF glyph advances for SVG and published Helvetica AFM widths for printable-ASCII PDF text. An unmapped glyph alone falls back to core `approx_text_width` |
| `crates/iron-canvas-export/assets/` | Inter Regular Latin-subset TTF plus its OFL license and provenance. SVG embeds the TTF as a base64 `@font-face`; PDF does not use it |
| `crates/iron-canvas-export/src/svg/painter.rs` | `SvgPainter` — `<svg>` emitter, embedded Inter `@font-face`, Inter text output/measurement, clip-path via `<defs>`, `<g>` groups for `GroupClass` brackets, DPR-aware. Implements `Painter` + `BlitPainter` (no-op) |
| `crates/iron-canvas-export/src/svg/surface.rs` | `SvgSurface` — one-shot `Surface`, `finish() -> String`. `SvgSurface::render` calls `crate::drive_once` and discards the overlay. |
| `crates/iron-canvas-export/src/pdf/painter.rs` | `PdfPainter` — emits PDF 1.7 content-stream ops, draws base-14 Helvetica, and measures with matching Helvetica widths. Implements `Painter` + `TextMetrics`; `BlitPainter` is a no-op |
| `crates/iron-canvas-export/src/pdf/surface.rs` | `PdfSurface` — single-page, `/MediaBox` baked at construction, Y-flip CTM prepended at page open. `PdfSurface::render` builds two independent surfaces via `PdfSurface::new` and discards the overlay (same pattern as `SvgSurface::render`); both call `crate::drive_once`. The web facade's `exportPdf` also uses `PdfSurface::render` (overlay discarded). (`PdfSurface::with_stream`, a shared grid+overlay content-stream constructor, currently has no caller.) |
| `crates/iron-canvas-export/src/pdf/doc/` | Hand-rolled PDF writer: two-pass buffered object table + xref. Type1 base-14 Helvetica, no font embedding (WinAnsi-only). |

### `iron-canvas-ironcalc` (IronCalc bridge)

| File | Owns |
| ---- | ---- |
| `crates/iron-canvas-ironcalc/src/lib.rs` | `IronCalcModel<'a>` — newtype wrapping `&'a UserModel<'a>` and implementing `CanvasModel`. Exists because Rust orphan rules prevent `impl CanvasModel for UserModel` outside the trait-defining crate. Also exposes `get_extended_cell_style()` for CF (conditional formatting) decoration bridge — returns per-cell `CfDecorationPaint` for data bars, icon sets, and color scales. The bulk `*_in` accessors (styles, types, decorations) inherit the `CanvasModel` trait default — a per-cell loop over the merged accessors; no override, since there is no JS boundary to amortise here (**C-1**, June 2026). |

## Verification

The workspace CI has separate native and real-browser wasm jobs in
`.github/workflows/test.yml`. The browser job installs the pinned
`wasm-bindgen-test-runner`, points it at ChromeDriver, and runs:

```bash
(cd iron-canvas && \
  cargo test --target wasm32-unknown-unknown \
    -p iron-canvas-web -p iron-canvas-datagrid-web --locked)
```

Both facade suites use `wasm_bindgen_test_configure!(run_in_browser)`;
their resize tests exercise fractional DPR (`1.25`) through the actual
DOM canvas backing stores. `iron-canvas-web/tests/render_wasm.rs` also
reads raw Canvas2D `ImageData` and compares retained output byte-for-byte
with a forced-fresh render for a border-free row repaint and two
explicit-border full-fallback cases. Those tests are raster proof for
their named scenarios; new retained-pixel behavior needs its own
forced-fresh comparison.
