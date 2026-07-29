# iron-canvas — agent guide

Cargo workspace at `iron-canvas/`. Paints a worksheet grid into HTML
`<canvas>` elements. Read-only renderer — never mutates the model.

## Workspace crates

| Crate                     | Role                                            | wasm? |
| ------------------------- | ----------------------------------------------- | ----- |
| `iron-canvas-core`        | Engine: Chrome, Orchestrator, Painter trait, decorations | No |
| `iron-canvas-canvas2d`    | Canvas-2D backend: CanvasPainter, WebSurface, theme_from_element | Yes |
| `iron-canvas-web`         | `#[wasm_bindgen]` facade, playback, JsBackedModel | Yes |
| `iron-canvas-export`      | Multi-format export: SvgPainter + PdfPainter     | No  |
| `iron-canvas-recorder`    | DrawOp log, ICR schema, RecordingPainter decorator | No  |
| `iron-canvas-ironcalc`    | Bridge: IronCalcModel newtype for CanvasModel    | No  |
| `iron-canvas-datagrid`    | Standalone data-grid model + CanvasModel impl    | No  |
| `iron-canvas-datagrid-web`| WASM facade for DataGridCanvas, 0-based JS API  | Yes |

## Key paths

```
iron-canvas/
├── crates/iron-canvas-core/src/
│   ├── chrome/           ← Chrome snapshot, PaneSet, blit, recycled slots
│   ├── renderer/         ← RendererCore, 4-pass cell paint, caches
│   ├── painter/mod.rs    ← Painter + BlitPainter + TextMetrics traits
│   ├── decoration/       ← Layer trait + 8 overlay decorations
│   ├── layer/mod.rs      ← Surface trait, PaintGate, LayerBase
│   ├── orchestrator.rs   ← PaintRegime dispatch, set_overlays, queries
│   ├── theme.rs          ← CanvasTheme, FORMULA_REF_COLORS
│   └── signal.rs         ← GridSignals bitflags
├── crates/iron-canvas-canvas2d/src/
│   ├── canvas_painter.rs ← Canvas-2D Painter + BlitPainter + TextMetrics impls
│   ├── web_surface.rs    ← WebSurface adapter over HtmlCanvasElement
│   └── theme_from_element.rs ← CSS var → CanvasTheme bridge
├── crates/iron-canvas-web/src/
│   ├── orchestrator.rs   ← IronCanvas (#[wasm_bindgen]), exportSvg, recording
│   ├── playback.rs       ← PlaybackSession + replay_through algorithm
│   └── wire.rs           ← JS wire-shape mirrors (serde-wasm-bindgen)
├── crates/iron-canvas-export/src/
│   ├── svg/painter.rs     ← SvgPainter — declarative SVG export
│   ├── svg/surface.rs     ← SvgSurface — throwaway surface
│   ├── pdf/painter.rs     ← PdfPainter — PDF 1.7 content stream
│   ├── pdf/surface.rs     ← PdfSurface — single-page, MediaBox
│   ├── pdf/doc/           ← hand-rolled PDF writer (object table, xref)
│   └── common/            ← xml_escape, pdf_string_escape, color parsing
├── crates/iron-canvas-ironcalc/src/
│   └── lib.rs             ← IronCalcModel newtype + CF bridge
├── crates/iron-canvas-recorder/src/
│   ├── lib.rs            ← DrawOp, RecordingPainter, RecordingSurface
│   └── recording.rs      ← ICR schema v2, Recording, IcrHeader
├── crates/iron-canvas-datagrid/src/
│   ├── model.rs           ← DataGrid struct, builder, sort, mutation
│   └── canvas_model.rs    ← impl CanvasModel + CellContentQuery for DataGrid
├── crates/iron-canvas-datagrid-web/src/
│   ├── lib.rs             ← DataGridCanvas (#[wasm_bindgen])
│   ├── model_cell.rs      ← DataGridModel (RefCell<DataGrid> wrapper)
│   └── wire.rs            ← JS wire types (GridDataWire, HitTestWire, …)
├── ARCHITECTURE.md       ← Full architecture (canonical)
├── .claude/skills/canvas-patterns/SKILL.md ← Paint pipeline + backends detail
└── docs/book/            ← "Building RustyCalc" mdbook (22 chapters)
```

## Invariants (break these and things break silently)

1. **One snapshot per tick.** `Chrome` is the single source of truth.
   Queries read the same `last_frame` that the renderer painted.
2. **`CellValueHash(u64)` newtype** — `DefaultHasher` is per-run seeded.
   Cross-process comparison of hashes is undefined behaviour prevented
   at the type level.
3. **Four-pass cell order is the contract.** BG → Grid borders → Explicit
   borders → Text. Reordering breaks visual layering.
4. **Frozen separators paint AFTER cells.** The 3px divider must win its
   pixels over the frozen cell's grid stroke.
5. **`set_theme` drops `last_frame` and invalidates paint cache.** Theme
   changes skip `is_still_valid` because the per-cell cache holds old
   palette colors.
6. **`set_theme` immediately re-stamps `IcrHeader.theme`** on any active
   recording via `restamp_recording_theme`.

## When modifying code

- **Adding a `GroupClass` variant**: touch `painter/mod.rs` enum + `as_str`
  match + `SvgPainter` match arm + regenerate golden fixtures if the
  variant appears in recorder output.
- **Changing `ICR_SCHEMA_VERSION`**: bump the constant in `recording.rs`,
  update golden fixtures via `ICR_REGEN=1 cargo test`.
- **Adding a crate**: update workspace `Cargo.toml` members, update
  `ARCHITECTURE.md` file map tables, update this file's crate list.
- **Modifying `Painter` trait**: every backend (`CanvasPainter`,
  `SvgPainter`, `RecorderPainter`, `RecordingPainter`, `PdfPainter`)
  must be updated. `DrawOp` enum must stay in sync.
- **Adding a decoration**: implement `Layer` trait, add to z-order in
  `paint_overlay_layer`, add `GroupClass` variant.

## Running tests

```bash
# All tests
cargo test --workspace

# iron-canvas-core only (no wasm deps)
cargo test -p iron-canvas-core

# Recorder + golden fixtures
cargo test -p iron-canvas-recorder

# Regenerate golden fixtures
ICR_REGEN=1 cargo test -p iron-canvas-recorder --test golden_fixture

# Export backends (SVG + PDF)
cargo test -p iron-canvas-export

# Feature-gated: recorder tests without dev-tools
cargo test -p iron-canvas-core --no-default-features
```

## Documentation

- **`ARCHITECTURE.md`** — Frame build pipeline, query pipeline, state topology,
  file map. Canonical reference. Update when adding modules or changing
  architecture.
- **`.claude/skills/canvas-patterns/SKILL.md`** — Paint pipeline detail,
  caches, painter backends, theme. Load this skill for renderer/painter work.
- **`docs/book/`** — "Building RustyCalc" mdbook. Code-first, decision-focused,
  anchored to real commits. `mdbook serve` to preview.

## Common gotchas

- **`CanvasPainter` setter cache is per-painter, not per-context.**
  Grid and overlay painters each have their own `last_fill`/`last_stroke`.
- **`present()` is per-layer in every regime arm.** Grid first, then overlay.
  Don't depend on once-per-frame semantics.
- **`BlitPainter::blit → None` is safe only for throwaway orchestrators.**
  The live canvas needs a real blit. SVG/PDF backends are throwaway-only.
- **`Rc::ptr_eq` on model** — `Orchestrator` holds `Option<Rc<dyn CanvasModel>>`.
  Re-setting the same Rc causes one redundant Fresh repaint (STRUCTURAL signal).
  Workbook-swap adapters should reuse the Rc handle to avoid this.
- **`last_dpr: i32` is cached on `IronCanvas`** — set by every `resize()`,
  used by `startRecording` and playback without querying the engine.
