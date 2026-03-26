# RustyCalc

[![MIT licensed][mit-badge]][mit-url]
[![Apache 2.0 licensed][apache-badge]][apache-url]

Alpha-stage spreadsheet built with Rust, compiled to WebAssembly. The calculation engine is [IronCalc](https://github.com/ironcalc/IronCalc), an open-source Excel-compatible engine written in Rust. RustyCalc wraps it with a Leptos CSR frontend and a Canvas 2D grid renderer.

**Status:** prototype. Core editing, formulas, multi-sheet, and persistence work. No toolbar, no charts, no collaborative editing yet.

## What works

- Cell editing with formula support (IronCalc handles parsing and evaluation)
- Canvas 2D rendered grid with frozen panes (needs toolbar), selection, autofill drag
- Formula bar with live edit sync to the cell overlay
- Sheet tab bar: add, rename, delete, hide/unhide, tab colors
- Column/row resize by dragging header borders
- Keyboard navigation matching Excel conventions (arrow keys, Ctrl+arrow, Shift+arrow, Page Up/Down, Home/End)
- Copy/paste (internal clipboard with structural paste, OS clipboard fallback for text)
- Undo/redo, insert/delete rows and columns
- Light/dark theme with localStorage persistence
- Auto-save to localStorage every second
- Tauri desktop build
- GitHub Pages deployment

## Known limitations

- `String.leak()` in `storage.rs` for the workbook name. IronCalc's `UserModel::new_empty` requires `&'static str`. Each new workbook leaks a small allocation. Negligible in practice but noted.
- 17 compiler warnings (unused fields/methods for components not yet built: toolbar, named ranges panel, locale panel, ...).

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
├── action.rs          Key→action classification and execution
├── app.rs             Root component, context providers, auto-save
├── formula_input.rs   Formula point-mode helpers (pure string ops)
├── state.rs           WorkbookState — all UI signals
├── storage.rs         localStorage serialization
├── theme.rs           Light/dark theme + CanvasTheme for Canvas 2D
├── util.rs            UUID generation, error logging, focus management
├── canvas/
│   ├── geometry.rs    Pixel<->cell coordinate math
│   └── renderer.rs    Canvas 2D drawing (grid, headers, selection, borders)
├── components/
│   ├── cell_editor.rs Textarea overlay during cell editing
│   ├── file_bar.rs    Theme toggle (more buttons planned)
│   ├── formula_bar.rs Cell address + formula input
│   ├── sheet_tab_bar.rs Sheet tabs with rename, color, hide/delete
│   ├── workbook.rs    Top-level keyboard dispatch
│   └── worksheet.rs   Canvas element + mouse handlers
└── model/
    ├── clipboard_bridge.rs  Serde bridge for IronCalc's pub(crate) Clipboard
    ├── frontend_model.rs    FrontendModel trait abstracting UserModel
    └── frontend_types.rs    Domain types (CssColor, SafeFontFamily, etc.)
```

## Docs

- [docs/adding-actions.md](docs/adding-actions.md) — how to add keyboard shortcuts
- [docs/leptos-patterns.md](docs/leptos-patterns.md) — Leptos conventions used in this codebase

## Dependencies

- [IronCalc](https://github.com/ironcalc/IronCalc) — spreadsheet engine (formula parsing, evaluation, OOXML support)
- [Leptos](https://leptos.dev/) 0.7 — reactive UI framework (CSR mode)
- [leptos-use](https://leptos-use.rs/) 0.15 — browser API hooks (ResizeObserver, setInterval)
- [Trunk](https://trunkrs.dev/) — WASM build tool
- [Tauri](https://tauri.app/) 2.x — optional desktop shell


# License

Licensed under either of

* [MIT license](LICENSE-MIT)
* [Apache license, version 2.0](LICENSE-Apache-2.0)

at your option.
