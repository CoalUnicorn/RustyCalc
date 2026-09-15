# web-test

Browser harnesses for iron-canvas. They use vanilla JavaScript with no
framework or bundler.

| Page                    | Purpose                                                                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------ |
| `index.html`            | Interactive and automatable `IronCanvas` JS API harness. Exercises queries, core column autofit, overlays, themes, SVG export, bulk bridge fetches, workbook/sheet switching, and the real two-canvas paint path. Loads the generated sample or a workbook from `demo/`. |
| `recording-viewer.html` | Standalone `.icr` viewer. Drag-drop a recording file (or use the file picker) and the page replays it onto a single 2D canvas via a JS mirror of `iron_canvas_recorder::replay`. Compression is intentionally not handled (deferred to `RECORDING_PLAN.md` Phase 3). Fixtures live in `crates/iron-canvas-recorder/tests/fixtures/`. |
| `datagrid.html`         | Standalone vanilla-JS demo of the engine-agnostic `iron-canvas-datagrid-web` bundle (`DataGridCanvas`). Exercises the full interactive API: scroll, header-sort, cell-select, column-resize-drag, SVG export, append/live rows, light/dark theme, and the optional frozen header. Build + serve with `make datagrid-serve` (no IronCalc vendor needed) and open `/datagrid.html`. `make serve` can host it only when `vendor/iron-canvas-datagrid/` was populated by an earlier `make datagrid`. |

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
# Build iron-canvas-web wasm.
make build

# Convert demo/*.xlsx to browser-loadable .ic files and copy the two wasm
# packages into vendor/.
make sync

# python3 -m http.server on $(PORT), default 8000.
make serve

# JavaScript helper tests (no browser required).
make check

# Start an isolated server + ChromeDriver and run the visible harness checks.
make browser-test

# Run the same browser checks against every workbook in demo/.
make browser-test-all

# Remove vendored packages and generated demo/*.ic files.
make clean
```

## Spreadsheet demos

The IronCalc JavaScript package contains the calculation engine but not the
`.xlsx` reader. `make demos` uses IronCalc's existing `xlsx_2_icalc` binary to
convert each tracked `demo/*.xlsx` source into an ignored `demo/*.ic` browser
artifact. `Model.from_bytes(...)` loads that artifact without adding another
wasm bundle or a server-side import endpoint.

Choose a demo from the workbook selector in `index.html`. `make sync` and
`make serve` depend on `make demos`, so the browser files stay in sync with
their spreadsheet sources. A rebuilt IronCalc wasm package also invalidates
the derived files, preventing the reader/converter bitcode schemas from
silently drifting apart.

## Browser automation

The harness installs `window.ironCanvasHarness`:

```js
await window.ironCanvasHarness.ready;
await window.ironCanvasHarness.loadWorkbook("dynamic_arrays");
const sizing = await window.ironCanvasHarness.autofitColumns();
const report = await window.ironCanvasHarness.runChecks();
```

`report` is JSON-serializable and contains `passed`, `failed`, and the result
of every check. Stable `data-testid` attributes are also present for UI-driven
tools. Open `index.html?workbook=forensics&autorun=1` to load a demo and run
the same checks automatically while leaving their results visible for a human.
The forensics check preserves its intentionally narrow imported A:E widths as
the baseline, then requires `fitColumnWidth` to widen several columns after the
host applies the measured values through IronCalc's `setColumnsWidth`.

`make browser-test` drives this contract through headless Chromium using the
standard WebDriver protocol; it has no npm dependencies. It defaults to
`dynamic_arrays`; use `make browser-test WORKBOOK=forensics` for one named demo
or `make browser-test-all` for all three. Override `PORT` or `WEBDRIVER_PORT`
when those defaults are already in use.

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
