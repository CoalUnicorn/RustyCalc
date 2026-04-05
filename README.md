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
├── app.rs             Root component, context providers, auto-save
├── state.rs           WorkbookState - all UI signals
├── storage.rs         localStorage serialization
├── theme.rs           Light/dark theme, CanvasTheme, COLOR_PALETTE
├── util.rs            UUID generation, error logging, focus management
├── canvas/
│   ├── geometry.rs    Pixel<->cell coordinate math
│   └── renderer.rs    Canvas 2D drawing (grid, headers, selection, borders)
├── input/
│   ├── action.rs      SpreadsheetAction wrapper enum, classify_key(), execute()
│   ├── error.rs       Per-module error types (FormatError, NavError, EditError, StructError)
│   ├── helpers.rs     mutate(), try_mutate(), EvaluationMode, selection_area(), selection_bounds()
│   ├── nav.rs         NavAction - arrows, page, home/end, sheet switch
│   ├── edit.rs        EditAction - start, commit, cancel cell editing
│   ├── format.rs      FormatAction - bold, italic, font size/family
│   ├── structure.rs   StructAction - delete, undo/redo, insert/delete rows/cols
│   └── formula_input.rs  Formula point-mode helpers (pure string ops)
├── components/
│   ├── cell_editor.rs    Textarea overlay during cell editing
│   ├── file_bar.rs       Theme toggle (more buttons planned)
│   ├── formula_bar.rs    Cell address + formula input
│   ├── sheet_tab_bar.rs  Sheet tabs with rename, color, hide/delete, context menus
│   ├── toolbar.rs        Undo/redo, font family/size, B/I/U/S, freeze panes
│   ├── workbook.rs       Top-level layout + keyboard dispatch
│   └── worksheet.rs      Canvas element + mouse handlers
└── model/
    ├── clipboard_bridge.rs  Serde bridge for IronCalc's pub(crate) Clipboard
    ├── frontend_model.rs    FrontendModel trait abstracting UserModel
    ├── frontend_types.rs    Domain types (CssColor, SafeFontFamily, ToolbarState, etc.)
    └── style_types.rs       StylePath, HexColor, BooleanValue for update_range_style API
```

## Docs

- [docs/rust-style-guide.md](docs/rust-style-guide.md) - Rust design patterns and type modeling principles
- [docs/leptos-patterns.md](docs/leptos-patterns.md) - Leptos conventions used in this codebase
- [docs/building-components.md](docs/building-components.md) - how to create and debug components
- [docs/adding-actions.md](docs/adding-actions.md) - how to add keyboard shortcuts and toolbar actions  
- [docs/testing-guide.md](docs/testing-guide.md) - Comprehensive guide to writing and organizing tests
- [docs/performance-evaluation.md](docs/performance-evaluation.md) - **Critical:** avoid double evaluation performance issues

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
