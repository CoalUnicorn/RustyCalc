# web-test

Manual smoke harness for iron-canvas. Two standalone HTML pages, vanilla
JS, no framework, no bundler.

| Page                    | Purpose                                                                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------ |
| `index.html`            | Paints a three-sheet workbook through the real iron-canvas pipeline. Console-logs a snapshot of every `JsBackedModel` bridge round-trip on first paint. Has a **Save as SVG** button that calls `canvas.exportSvg(800, 400)` and downloads `sheet.svg`. |
| `recording-viewer.html` | Standalone `.icr` viewer. Drag-drop a recording file (or use the file picker) and the page replays it onto a single 2D canvas via a JS mirror of `iron_canvas_recorder::replay`. Compression is intentionally not handled (deferred to `RECORDING_PLAN.md` Phase 3). Fixtures live in `crates/iron-canvas-recorder/tests/fixtures/`. |
| `datagrid.html`         | Standalone vanilla-JS demo of the engine-agnostic `iron-canvas-datagrid-web` bundle (`DataGridCanvas`). Exercises the full interactive API: scroll, header-sort, cell-select, column-resize-drag, SVG export, append/live rows, light/dark theme, and the optional frozen header. Build with `make datagrid` (no IronCalc vendor needed), then `make serve` (or `make datagrid-serve`) and open `/datagrid.html`. |

Open: <http://localhost:8000/index.html> · <http://localhost:8000/recording-viewer.html> · <http://localhost:8000/datagrid.html>

## Build prerequisites

Point `ICALC_PKG` at your local IronCalc wasm bindings (edit `Makefile`
or override on the command line):

```sh
make serve ICALC_PKG=/path/to/IronCalc/bindings/wasm/pkg
```

The IronCalc wasm bindings must be built before `make sync` can copy them:

```sh
cd ../../IronCalc/bindings/wasm && make
cd - && make sync
```

## Workflow

```sh
# Build iron-canvas-web wasm, then copy it + IronCalc into vendor/.
make sync

# python3 -m http.server on $(PORT), default 8000.
make serve

# Remove vendor/iron-canvas and vendor/ironcalc.
make clean
```

`make build` always passes `--no-opt` to `wasm-pack` — wasm-opt
roughly doubles the build time for no visible benefit at this scale,
and the dev-loop iteration cost matters more than a few KB of wasm.

Profile is controlled via `PROFILE`:

```sh
make build PROFILE=dev         # --dev
make build PROFILE=profiling   # --profiling
make build                     # --release (default)
```

## Recording

To produce a `.icr` to drag into `recording-viewer.html`, build with the dev-only `dev-tools` feature. Two paths produce equivalent recorder-aware wasm:

**Standalone harness** (this directory):

```sh
make build FEATURES=dev-tools
make serve
```

`make build` forwards `FEATURES` to `wasm-pack` as `--features "dev-tools"` and copies the resulting `pkg/` into `vendor/iron-canvas`. Run `make sync` only (without rebuilding) if you already have a fresh `pkg/`.

**Full RustyCalc app** (from the workspace root):

```sh
trunk serve --features dev-tools
```

The root `dev-tools` feature propagates to `iron-canvas-web/dev-tools`. The full app also gets the status-bar perf panel under the same flag (see the root [`README.md`](../../README.md#dev-tools)).

In either case the host JS then sees `startRecording()` / `stopRecording()` on `IronCanvas`. The always-on `recordingSupported() -> bool` probe lets a page detect which flavor of wasm is loaded. Without the feature flag those symbols are not exported and the prod bundle pays zero recording cost.
