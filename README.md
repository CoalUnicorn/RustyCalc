# RustyCalc

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)


Alpha-stage spreadsheet built with Rust, compiled to WebAssembly. The calculation engine is [IronCalc](https://github.com/ironcalc/IronCalc), an open-source Excel-compatible engine written in Rust. RustyCalc wraps it with a Leptos CSR frontend and a Canvas 2D grid renderer.

**Status:** prototype. Core editing, formulas, formatting toolbar, multi-sheet, and persistence work. No charts, no collaborative editing yet.

## What works

- Cell editing with formula support (IronCalc handles parsing and evaluation)
- Canvas 2D rendered grid with frozen panes, selection, autofill drag
- Toolbar: undo/redo, font family, font size (−/+), bold, italic, underline, strikethrough, 
  freeze panes, color picker
- Formula bar with live edit sync to the cell overlay
- Sheet tab bar: add, rename, delete, hide/unhide, tab colors, context menus
- Column/row resize by dragging header borders
- Keyboard navigation matching Excel conventions (arrow keys, Ctrl+arrow, Shift+arrow, Page Up/Down, Home/End)
- Keyboard shortcuts: Ctrl+B/I/U (bold/italic/underline), Ctrl+Z/Y (undo/redo)
- Copy/paste (internal clipboard with structural paste, OS clipboard fallback for text)
- Light/dark theme with localStorage persistence
- Auto-save to localStorage every second
- Tauri desktop build
- GitHub Pages deployment

## Known limitations

- `String.leak()` in `storage.rs` for the workbook name. IronCalc's `UserModel::new_empty` requires `&'static str`. Each new workbook leaks a small allocation. Negligible in practice but noted.

## Build

Requires [Trunk](https://trunkrs.dev/) and the `wasm32-unknown-unknown` target.

```
rustup target add wasm32-unknown-unknown
cargo install trunk

# Dev server at localhost:8080/RustyCalc/
trunk serve

# Production build to dist/
trunk build --release

# Tauri desktop
cargo tauri dev

# Tests
wasm-pack test --headless --firefox
```

## Project structure

```
src/
├── main.rs            Entry point (mounts App)
├── app.rs             Root component, context providers, auto-save
├── coord.rs           CellAddress, CellArea, SheetArea - coordinate primitives
├── events.rs          Typed event system (ContentEvent, FormatEvent, etc.)
├── perf.rs            PerfTimings - commit→render pipeline timestamps
├── state.rs           WorkbookState - all UI signals
├── storage.rs         localStorage serialization
├── theme.rs           Light/dark theme, CanvasTheme, COLOR_PALETTE
├── util.rs            UUID generation, error logging, focus management
├── canvas/
│   ├── geometry.rs    Pixel<->cell coordinate math
│   ├── renderer.rs    Canvas 2D drawing (grid, headers, selection, borders)
│   └── types.rs       CanvasLayout, CellRenderData, resolved cell styles
├── input/
│   ├── action.rs      SpreadsheetAction wrapper enum, classify_key(), execute()
│   ├── error.rs       Per-module error types (FormatError, NavError, EditError, StructError)
│   ├── nav.rs         NavAction - arrows, page, home/end, sheet switch
│   ├── edit.rs        EditAction - start, commit, cancel cell editing
│   ├── format.rs      FormatAction - bold, italic, font size/family
│   ├── structure.rs   StructAction - delete, undo/redo, insert/delete rows/cols
│   ├── formula_input.rs  Formula point-mode helpers (pure string ops)
│   └── xlsx_io.rs     File import/export
├── components/
│   ├── cell_editor.rs    Textarea overlay during cell editing
│   ├── color_picker.rs   Color palette + recent colors popup
│   ├── context_menu.rs   Reusable right-click / button-triggered menu
│   ├── file_bar.rs       Workbook management (new, open, save, import/export)
│   ├── formula_bar.rs    Cell address + formula input
│   ├── perf_panel.rs     Debug overlay for commit→render timing
│   ├── sheet_tab_bar.rs  Sheet tabs with rename, color, hide/delete, context menus
│   ├── toolbar.rs        Undo/redo, font family/size, B/I/U/S, freeze panes
│   ├── workbook.rs       Top-level layout + keyboard dispatch
│   └── worksheet.rs      Canvas element + mouse handlers
└── model/
    ├── clipboard_bridge.rs  Serde bridge for IronCalc's pub(crate) Clipboard
    ├── frontend_model.rs    FrontendModel trait, mutate/try_mutate helpers
    ├── frontend_types.rs    Domain types (CssColor, SafeFontFamily, ToolbarState, etc.)
    └── style_types.rs       StylePath, HexColor, BooleanValue for update_range_style API
```

## Docs

- [docs/state-and-events.md](docs/state-and-events.md) - WorkbookState fields, EventBus categories, adding events
- [docs/leptos-patterns.md](docs/leptos-patterns.md) - Leptos conventions (signals, event subscriptions, view bindings)
- [docs/building-components.md](docs/building-components.md) - creating and debugging components
- [docs/adding-actions.md](docs/adding-actions.md) - adding keyboard shortcuts and toolbar actions
- [docs/rust-style-guide.md](docs/rust-style-guide.md) - type modeling patterns (newtypes, enums, error handling)
- [docs/testing-guide.md](docs/testing-guide.md) - wasm-pack test setup, test categories, examples
- [docs/performance-evaluation.md](docs/performance-evaluation.md) - avoiding double evaluation with mutate/try_mutate

## Dependencies

- [IronCalc](https://github.com/ironcalc/IronCalc) - spreadsheet engine (formula parsing, evaluation, OOXML support)
- [Leptos](https://leptos.dev/) 0.7 - reactive UI framework (CSR mode)
- [leptos-use](https://leptos-use.rs/) 0.15 - browser API hooks (ResizeObserver, setInterval)
- [Trunk](https://trunkrs.dev/) - WASM build tool
- [Tauri](https://tauri.app/) 2.x - optional desktop shell


# License

Licensed under either of

* [MIT license](https://opensource.org/licenses/MIT)
* [Apache license, version 2.0](https://opensource.org/licenses/Apache-2.0)


at your option.
