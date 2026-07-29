<!-- last-verified-against: c16e104 (2026-07-25) -->
<!-- working-tree-verified: 2026-07-25 -->
<!-- covers: src/ Cargo.toml iron-canvas/ IronCalc/ .github/workflows/test.yml -->

# RustyCalc architecture

RustyCalc is a single-page spreadsheet that runs entirely in the browser
as WebAssembly. It pairs three layers, each with a single responsibility:

- **IronCalc** (vendored at `../IronCalc/`) — the calculation engine.
  Stores cells, parses formulas, evaluates dependents, owns
  workbook semantics (sheets, defined names, frozen panes, xlsx I/O).
- **iron-canvas** (workspace at `iron-canvas/`) — a read-only renderer.
  Paints the worksheet grid into HTML `<canvas>` elements and answers
  cursor queries against the painted result. Never mutates the model.
- **The Leptos app at `src/`** — the binding layer. Owns UI state,
  routes user input through typed action enums, mutates IronCalc, then
  invalidates the canvas and wakes a demand-driven rAF scheduler.

The whole binary is a `csr` Leptos crate (`Cargo.toml` line 11). There
is no server side; `wasm-bindgen` ferries data across the JS boundary
for clipboard, file pickers, CSS variables, and the rendering context.

## Workspace layout

```
RustyCalc/
├── Cargo.toml                # binary crate `rusty-calc` (csr Leptos)
├── src/                      # this layer — Leptos app
│   ├── main.rs               # mount_to_body(<App />)
│   ├── app.rs                # top-level component, context provider, autosave
│   ├── app_state.rs          # AppState — theme, sidebar, perf, registry version
│   ├── state/                # WorkbookState, ModelStore, DragState, EditingCell, AutoscrollState, CameraSpec, …
│   ├── events/               # EventBus + 5 typed category enums
│   ├── coord/                # RefNode, CellAddress, CellArea, SheetRange, ActiveRef
│   │   ├── mod.rs            # Re-exports + tests
│   │   ├── types.rs          # Structs + inherent impls
│   │   └── convert.rs        # From impls across coordinate types
│   ├── theme.rs              # leptos-use color-mode binding, COLOR_PALETTE, FORMULA_REF_COLORS
│   ├── storage/              # localStorage protocol — magic+version header, workbook registry, xlsx round-trip
│   │   ├── mod.rs             # re-exports
│   │   ├── persist.rs         # load/save/create/delete (b"RCAL" magic+version, ModelEntry, autosave)
│   │   ├── registry.rs        # WorkbookId UUIDs, group-aware listing, sanitize_name
│   │   └── share.rs           # share-URL generation (base-62, SHA-256 word hash)
│   ├── verify.rs             # Share verification: word-hash extraction, consent modal gate
│   ├── perf.rs               # PerfTimings (commit/input/eval timestamps, render_ms, frame_trace)
│   ├── util.rs
│   ├── model/                # IronCalc binding
│   │   ├── frontend_model.rs # FrontendModel trait on UserModel, mutate / try_mutate, EvaluationMode
│   │   ├── frontend_types.rs # CssColor — wire type, carries model data zero-copy through renderer
│   │   ├── style_types.rs    # StylePath — validated style property paths for IronCalc's update_range_style()
│   │   └── clipboard_bridge.rs # AppClipboard + PasteMode
│   ├── input/                # keyboard / mouse → SpreadsheetAction → model
│   │   ├── keyboard/         # classify_key → SpreadsheetAction → execute()
│   │   │   ├── action.rs     #   SpreadsheetAction enum
│   │   │   ├── classify.rs   #   key+modifiers → SpreadsheetAction
│   │   │   └── dispatch.rs   #   execute() router
│   │   ├── mouse/            # hit-test + drag handlers
│   │   │   ├── mousedown.rs / mousemove.rs / mouseup.rs / wheel.rs / dblclick.rs / contextmenu.rs
│   │   │   ├── click.rs              #   cell / header / corner click resolution
│   │   │   ├── cursor_hint.rs        #   idle cursor priority chain (matches mousedown priorities)
│   │   │   └── formula_ref.rs        #   formula-ref overlay hit + drag
│   │   ├── formula/          # shared formula-editor input handling
│   │   │   ├── analysis.rs   #   tokenize formula → FormulaAnalysis (refs + bare_ref_spans + status)
│   │   │   ├── input.rs      #   on:input routing
│   │   │   ├── edit_sync.rs  #   FormulaEditState trait + sync_edit helper
│   │   │   ├── ref_mode.rs   #   point-mode arming
│   │   │   └── status.rs     #   FormulaStatus enum
│   │   ├── formula_overlay.rs # formula-overlay state (point-mode, ref dragging)
│   │   ├── nav.rs            # NavAction + execute_nav
│   │   ├── edit.rs           # EditAction (Start / Commit / Cancel) + execute_edit
│   │   ├── format.rs         # FormatAction + execute_format
│   │   ├── structure.rs      # StructAction (insert/delete/clear/undo/redo) + execute_struct
│   │   ├── sheet.rs          # sheet add/delete/rename/reorder/visibility
│   │   ├── workbook.rs       # workbook create/delete/rename/group operations
│   │   ├── xlsx_io.rs        # xlsx import/export through `ironcalc` crate
│   │   ├── error.rs          # EditError / NavError
│   │   └── mod.rs
│   ├── components/           # Leptos components — organised by layer; see "Component tree"
│   │   ├── mod.rs            # exactly: `pub mod {chrome,panels,ui,workbook};`
│   │   ├── ui/               # generic primitives (no domain knowledge)
│   │   │   ├── popover.rs            # anchored floating panel — pos-based, click-outside dismiss
│   │   │   ├── context_menu.rs       # ContextMenu / Button / Item / Separator (delegates layout to Popover)
│   │   │   ├── color_picker.rs       # color-picker popover
│   │   │   ├── inline_rename.rs      # inline rename for sheet tabs and drawer items
│   │   │   ├── drawer.rs             # <Drawer> — right-pinned non-modal panel, slide-in, Esc
│   │   │   ├── range_picker.rs       # <RangePickerInput> — text + ⊞ arm, reactive grid capture
│   │   │   └── modal.rs              # generic <Modal> wrapper
│   │   ├── chrome/           # app-shell components
│   │   │   ├── formula_bar.rs        # <FormulaBar> — active-cell address + formula input
│   │   │   ├── left_drawer.rs        # <LeftDrawer> — workbook registry sidebar
│   │   │   ├── sheet_tab_bar.rs      # <SheetTabBar> — bottom strip: tabs, add/delete/rename
│   │   │   ├── status_bar.rs         # <StatusBar> — selection readout + status banner
│   │   │   └── toolbar/              # <Toolbar> — two-tier: section TabStrip over a per-section OverflowRow
│   │   │       ├── mod.rs             #   component + slot table per ToolbarSection
│   │   │       ├── section.rs         #   ToolbarSection enum (Home/Data/View/File) + ToolSlot descriptor
│   │   │       ├── tab_strip.rs       #   <TabStrip> — top-tier section selector (pure view state)
│   │   │       ├── overflow.rs        #   <OverflowRow> — measures slot widths, collapses overflow into ⋯ menu
│   │   │       ├── chrome_controls.rs #   hamburger + repo/version link + light/dark theme toggle
│   │   │       ├── share_controls.rs  #   Share button + popover + verify word + Trust badge
│   │   │       ├── file_ops.rs        #   File tab — xlsx import/export (migrated from file_bar)
│   │   │       ├── view_options.rs    #   View tab — row/column header + gridline visibility toggles
│   │   │       ├── camera.rs          #   View tab — <InsertCamera>, floating live picture of the selection
│   │   │       ├── icon.rs            #   FileIcon / ChromeIcon SVG sets
│   │   │       ├── alignment.rs       #   horizontal/vertical alignment, text wrap, merge
│   │   │       ├── color_pickers.rs   #   fill + font color pickers
│   │   │       ├── font.rs            #   font family + font size
│   │   │       ├── format_toggles.rs  #   bold / italic / underline / strikethrough / clear
│   │   │       ├── freeze.rs          #   freeze-panes controls
│   │   │       ├── named_ranges.rs    #   Named Ranges manager trigger
│   │   │       ├── number_format.rs   #   number-format dropdown + quick buttons
│   │   │       ├── style.rs           #   <BorderPicker> — cell border styling
│   │   │       └── undo_redo.rs       #   undo / redo buttons
│   │   ├── panels/           # feature panels and overlays mounted at workbook scope
│   │   │   ├── header_context_menu.rs  # right-click on column/row header (DOM bridge for canvas)
│   │   │   ├── perf_panel.rs           # <PerfPanel> — commit/eval/render timings + record button
│   │   │   ├── playback_panel.rs       # <PlaybackPanel> — load/seek/play/pause/exit controls
│   │   │   ├── share_popover.rs        # share URL generation popover
│   │   │   ├── share_verify.rs         # share verification consent modal
│   │   │   ├── named_ranges/           # Named Ranges manager dialog
│   │   │       └── form.rs / formula_input.rs / list.rs
│   │   │   └── conditional_formatting/   # Conditional Formatting rule editor
│   │   │       └── editor.rs / form.rs / list.rs
│   │   └── workbook/         # the worksheet surface itself
│   │       ├── mod.rs                # <Workbook> — tabindex wrapper, keyboard handler, camera persistence
│   │       ├── one_shot_raf.rs       # shared demand-driven rAF wrapper; returns a poke closure
│   │       ├── camera/               # Camera tool — floating live-picture widgets
│   │       │   ├── mod.rs                # <CameraLayer> (keyed For over cameras) + <Camera> widget
│   │       │   ├── canvas.rs             # CameraCanvas — Orchestrator<WebSurface> + DataGridModel
│   │       │   ├── extract.rs            # extract_grid — UserModel range → headerless styled DataGrid
│   │       │   └── watch.rs              # events_touch_source — does an event touch the source range?
│   │       ├── editing/              # in-cell editor + formula overlay
│   │       │   ├── cell_editor.rs        # <CellEditor> — transparent textarea over active cell
│   │       │   ├── formula_overlay.rs    # formula-ref overlay component
│   │       │   └── formula_text_area.rs  # shared formula-editor UI
│   │       └── worksheet/            # <Worksheet> — grid/overlay canvases, rAF loop
│   │           ├── mod.rs                # entry component + view tree
│   │           ├── adapter.rs            # WorksheetModelAdapter (CanvasModel impl over ModelStore)
│   │           ├── raf_loop.rs           # use_one_shot_raf — lazy construction, paint_if_dirty, playback tick, font listener
│   │           ├── subscribe.rs          # the subscribe-Effect — events → IronCanvas dirty bits
│   │           ├── overlay_memo.rs       # reactive_overlay memo
│   │           └── dev_tools_effects.rs  # recording / playback command-drain Effects
│   └── test/                 # action-level tests
└── iron-canvas/              # rendering workspace — see iron-canvas/ARCHITECTURE.md
```

External crates that anchor the design: `leptos = 0.8` (CSR features),
`leptos-use = 0.19` (raf, resize observer, color mode, debounce),
`ironcalc_base` / `ironcalc` (path-deps to `../IronCalc/`), `gloo-storage`,
`web-sys` (canvas / clipboard / file APIs), `wasm-bindgen`.

## Boot

```
fn main()                              src/main.rs
  └── mount_to_body(|| view! { <App /> })

#[component] fn App()                  src/app.rs
  ├── storage::load_selected()         // (WorkbookId, UserModel<'static>) from localStorage,
  │                                    // or storage::create_new() on first launch
  ├── EventBus::new()                  // 5 RwSignal<Vec<…>>
  ├── AppState::new(events)            // theme/sidebar/perf/registry
  ├── WorkbookState::new(events)       // editing/drag/cursor/menus/named-ranges
  ├── wb_state.current_uuid.set(uuid)
  ├── StoredValue::new_local(model)    // ModelStore — UserModel<'static>
  ├── StoredValue::new_local(None)     // AppClipboard
  ├── provide_context(app_state)
  ├── provide_context(wb_state)
  ├── provide_context(model)
  ├── provide_context(clipboard)
  ├── Effect::new — autosave on content/format/structure events (debounced)
  ├── on `beforeunload` — final flush to localStorage
  └── view! { <div id="app"> <LeftDrawer /> <Workbook /> </div> }
```

Every downstream component reads these four context values:

| Context        | Type                                                    | Reactive? |
| -------------- | ------------------------------------------------------- | --------- |
| `AppState`     | `Copy` struct of `Split<T>` and signal handles          | Yes       |
| `WorkbookState`| `Copy` struct of `Split<T>` + `NodeRef` + `EventBus`    | Yes       |
| `ModelStore`   | `StoredValue<UserModel<'static>, LocalStorage>`         | **No**    |
| `AppClipboard` | `StoredValue<Option<AppClipboard>, LocalStorage>`       | **No**    |

The crucial part is that `ModelStore` is **non-reactive**. Mutating the
`UserModel` does not trigger Leptos to re-render anything. The
`EventBus` is the side-channel that announces "a mutation happened" to
subscribers — and only subscribers that care about that specific
category run.

**Emit convention (blessed, not routed).** The `src/input/` layer is the
primary emit surface for UI-driven actions (keyboard, mouse, toolbar,
formula bar). Components may emit directly for component-scoped concerns
(clipboard copy/paste in `workbook/mod.rs`, resize observer in
`worksheet/mod.rs`, file I/O in `toolbar/file_ops.rs`, share-verify in
`share_verify.rs`) — these are not user-input actions dispatched through
`input/`, they are self-contained component behaviours that happen to
produce events. No cross-layer emit coupling exists (components do not
import from `input/` directly).

## State topology

### `Split<T>` — zero-cost paired signals

```rust
pub struct Split<T: Clone + Send + Sync + 'static>(ReadSignal<T>, WriteSignal<T>);
```

Hand-rolled `Copy`/`Clone` impls (Leptos signals are arena-ID `Copy`,
so `Split<String>` is also `Copy`). Every reactive piece of UI state in
`WorkbookState` is a `Split<T>` — passing the bundle around costs the
same as passing two `u32`s.

### `AppState` — global UI

| Field                | Type                          | Meaning                                            |
| -------------------- | ----------------------------- | -------------------------------------------------- |
| `events`             | `EventBus`                    | Hoisted here for any AppState-only emitters        |
| `theme_mode`         | `Signal<ColorMode>` (leptos-use) | Resolved theme — only `Light` / `Dark`, never `Auto` |
| `set_theme_mode`     | `WriteSignal<ColorMode>`      | User preference — persisted to localStorage by leptos-use |
| `sidebar_open`       | `Split<bool>`                 | Left drawer collapsed state                        |
| `collapsed_groups`   | `Split<Vec<String>>`          | Which workbook groups are folded in the drawer     |
| `show_perf_panel`    | `Split<bool>`                 | Perf panel visibility (prod: false; dev-tools: true)|
| `perf`               | `PerfTimings`                 | Commit/input/eval timestamps, last draw duration, committed text, and optional frame trace |
| `recording_active`   | `Split<bool>`                 | `true` while iron-canvas is capturing paint frames |
| `recording_cmd`      | `Split<Option<RecordingCmd>>` | One-shot Start/Stop from PerfPanel record button   |
| `export_cmd`         | `Split<Option<ExportCmd>>`    | One-shot Svg/Pdf from PerfPanel export buttons     |
| `playback_loaded`    | `Split<bool>`                 | `true` after loadRecording takes ownership of canvases |
| `playback_playing`   | `Split<bool>`                 | Mirrors `IronCanvas::isPlaying()` from rAF tick    |
| `playback_frame`     | `Split<u32>`                  | Current displayed frame index, synced from rAF     |
| `playback_frame_count`| `Split<u32>`                 | Total frames in loaded recording; set on Load      |
| `playback_cmd`       | `Split<Option<PlaybackCmd>>`  | One-shot Load/Seek/Play/Pause/Exit from PlaybackPanel |
| `registry_version`   | `RwSignal<u64>`               | Bumped on workbook create/delete/rename/group      |

`registry_version` exists so the left drawer can subscribe to *structure
changes only*, never to scroll or edit events that the EventBus
broadcasts. Drawer re-renders during scroll were the original problem.

### `WorkbookState` — workbook-scoped UI

| Field                       | Type                                       | Role                                                    |
| --------------------------- | ------------------------------------------ | ------------------------------------------------------- |
| `events`                    | `EventBus`                                 | Shared with `AppState`                                  |
| `current_uuid`              | `Split<Option<WorkbookId>>`                | Which workbook is loaded                                |
| `recent_colors`             | `Split<Vec<CssColor>>`                     | LRU color list, max 16, persisted to localStorage       |
| `editing_cell`              | `Split<Option<EditingCell>>`               | In-progress cell edit (address, text, cursor, mode, focus, `formula_analysis`, `text_dirty`) |
| `formula_input_ref`         | `NodeRef<html::Input>`                     | Formula-bar input element                               |
| `cell_editor_ref`           | `NodeRef<html::Textarea>`                  | In-cell editor textarea                                 |
| `drag`                      | `Split<DragState>`                         | One enum — see below                                    |
| `hover_cursor`              | `Split<CursorHint>`                        | Idle cursor style; set by `handle_mousemove`            |
| `dragged_ref_override`      | `Split<Option<RefOverride>>`               | Ghost-range during a formula-ref overlay drag           |
| `context_menu`              | `Split<Option<ContextMenuState>>`          | Right-click menu state                                  |
| `status_message`            | `Split<Option<StatusMessage>>`             | Persistent error banner                                 |
| `auto_scroll`               | `AutoscrollState`                          | Edge-scroll JS interval handle (non-reactive)           |
| `active_drawer`             | `Split<Option<ActiveDrawer>>`             | Which drawer panel is open (mutual exclusion). Variants: `ConditionalFormatting`, `NamedRanges` |
| `range_capture`             | `Split<Option<RangeCaptureTarget>>`       | Which field is armed to receive grid selections. Variants: `CfRange`, `CfFormula`, `NamedRange`, `Camera(u32)` |
| `editing_cf_rule`           | `Split<Option<CfRuleEditState>>`          | In-progress CF rule editing (index, range, CfRuleInput) |
| `editing_named_range`       | `Split<Option<EditingDefinedName>>`        | In-progress row inside the named-range modal            |
| `cameras`                   | `Split<Vec<CameraSpec>>`                   | Floating Camera widgets — live pictures of a range; persisted per-workbook |

`DragState` is one enum so two drag modes can never overlap:

```rust
enum DragState {
    Idle,
    Selecting,
    Extending { to_row: i32, to_col: i32 },              // autofill drag
    ResizingCol { col: i32, x: f64 },
    ResizingRow { row: i32, y: f64 },
    Pointing { ref_node: RefNode, ref_text: TextRef },   // formula point-mode
    DraggingFormulaRef { idx: usize, anchor: SheetRange, grab_cell: CellAddress },
}
```

### `EventBus` — typed broadcast channel

```rust
pub struct EventBus {
    pub content:    RwSignal<Vec<ContentEvent>>,
    pub format:     RwSignal<Vec<FormatEvent>>,
    pub navigation: RwSignal<Vec<NavigationEvent>>,
    pub structure:  RwSignal<Vec<StructureEvent>>,
    pub theme:      RwSignal<Vec<ThemeEvent>>,
}
```

`emit_event(SpreadsheetEvent)` sorts the event into its category vector
and **replaces** that category's signal (does not append). Subscribers
read the signal and check `is_empty()` — if non-empty, a new action of
that category just happened. Empty categories are cleared with `.set(vec![])`
to avoid unnecessary reactive churn.

Five subscriber patterns observed in the tree:

| Subscriber                    | Categories watched          | Purpose                                    |
| ----------------------------- | --------------------------- | ------------------------------------------ |
| `LeftDrawer`                  | `registry_version` only     | Workbook list — never scroll-driven        |
| `Worksheet` Effect            | content/format/nav/structure/theme | Pushes canvas state/dirty intent, then calls the scheduler `poke` |
| `App` autosave Effect         | content + format + structure (debounced 500ms) | Serializes UserModel to localStorage |
| `FormulaBar` Effect           | navigation                  | Refreshes its displayed formula text       |
| `Toolbar` Memos               | format + navigation + content | `toolbar_state` (button states) + `undo_redo_state` (can_undo/redo) |

The bus is intentionally one-way: emitters never `await` consumers and
consumers never call back into the bus from inside their Effect.

## The model layer

`src/model/frontend_model.rs` defines four narrow traits — `FormulaAnalyzer`,
`DefinedNameManager`, `SheetQuery`, `Navigator` — all implemented for
`UserModel<'_>`. These replace the former umbrella `FrontendModel` trait.
They are the project-friendly surface over IronCalc: every query returns a project type (`ToolbarState`, `CellAddress`,
`SheetRange`, `FrozenPanes`, `DefinedName`, …); every mutation has a
descriptive name (`nav_arrow`, `nav_extend_column_selection`, `set_selected_area`,
`create_defined_name`, …). IronCalc boundary types (`Area`, `ClipboardTuple`,
`SelectedView`) are converted at the trait edge and never leak past it.

### `mutate` and `try_mutate`

```rust
pub enum EvaluationMode { Immediate, Deferred }

pub fn mutate<F>(store: ModelStore, mode: EvaluationMode, f: F)
where F: FnOnce(&mut UserModel<'static>);

pub fn try_mutate<E, F>(store: ModelStore, mode: EvaluationMode, f: F)
    -> Result<(), E>
where F: FnOnce(&mut UserModel<'static>) -> Result<(), E>;
```

Both wrap `ModelStore.update_value(...)` and, when `mode == Immediate`,
call `UserModel::evaluate()` after `f` returns. The split:

- `EvaluationMode::Immediate` — the mutation may change formula results
  (cell input, paste, defined-name create/rename/delete, row/col delete,
  undo/redo). Recalc must run before the canvas reads cell values.
- `EvaluationMode::Deferred` — pure navigation, selection, or formatting
  changes (arrow keys, range selection, bold). No formula re-eval needed.

With `dev-tools`, `try_mutate` also stamps `perf.commit_start`,
`perf.input_done`, and (for successful immediate evaluation)
`perf.eval_done` around the closure and `evaluate` call.

### IronCalc patterns this layer respects

Sourced from `.claude/skills/ironcalc-patterns/`. Highlights enforced
at this boundary:

- All writes go through `UserModel` (not raw `Model`) — undo/redo, view
  state, frozen panes, and selection all live on `UserModel`.
- `set_user_input` (cell typed text) is the only commit-from-text path.
- Defined-name mutations always pair with `EvaluationMode::Immediate`.
- `CellAddress`, `CellArea`, `SheetRange`, `RefNode` are the four shapes
  flowing through the input pipeline. `RefNode` wraps an ironcalc
  `Node::ReferenceKind | Node::RangeKind` so absolute-row/column flags
  and named-range identity survive the round-trip.

## Component tree

```
<App>
├── <LeftDrawer>                     # workbook registry — subscribes to registry_version
└── <Workbook>                       # tabindex=0 wrapper; owns on:keydown
    ├── <Toolbar>                    # two-tier: <TabStrip> (Home/Data/View/File) over <OverflowRow>;
    │                                #   header row also hosts hamburger, <ShareControls>, theme toggle
    ├── <FormulaBar>                 # active-cell address + formula input
    ├── <Worksheet>                  # grid + overlay canvases + CellEditor
    │   ├── <canvas .ws-grid>        # grid layer (opaque)
    │   ├── <canvas .ws-overlay>     # overlay layer (alpha)
    │   └── <CellEditor>             # absolutely-positioned textarea over active cell
    ├── <CameraLayer>                # Camera tool — inset:0 pointer-events:none overlay; keyed <For> of <Camera> widgets
    ├── <HeaderContextMenuOverlay>   # right-click on column/row header
    ├── <SheetTabBar>                # bottom strip — tabs, add/delete/rename/colorize
    ├── <StatusBar>                  # selection-range readout, sum/avg/count, error banner
    │   ├── <PerfPanel>              # dev-tools: paint timings + Record/SVG/PDF buttons (recordingSupported gate)
    │   └── <PlaybackPanel>          # dev-tools: load/seek/play/pause/exit controls (recordingSupported gate)
    ├── <Show when=move || matches!(state.active_drawer.get(), Some(ActiveDrawer::ConditionalFormatting))>
    │   └── <Drawer on_close=…>      # right-pinned non-modal panel; grid stays live
    │       └── <ConditionalFormattingDialog>  # rule type, operator, format, range field
    └── <Show when=move || matches!(state.active_drawer.get(), Some(ActiveDrawer::NamedRanges))>
        └── <Drawer on_close=…>      # right-pinned non-modal panel
            └── <NamedRangesDialog>  # create/edit/delete defined names
```

Three components own component-scoped editing state separate from
`editing_cell`:

- `<NamedRangesDialog>` uses `WorkbookState::editing_named_range:
  Split<Option<EditingDefinedName>>` — its own slim shape, shares the
  `sync_edit` helper (via the `FormulaEditState` trait) with
  `<CellEditor>` and `<FormulaBar>`.
- `<ConditionalFormattingDialog>` uses `WorkbookState::editing_cf_rule:
  Split<Option<CfRuleEditState>>` — rule index, sqref range, and
  `CfRuleInput`.
- `<InlineRename>` (used in `<SheetTabBar>` and `<LeftDrawer>`) holds
  ephemeral text in a local `RwSignal<String>` that disappears on commit.

The two drawers never mount simultaneously — `ActiveDrawer` is an enum,
so `Split<Option<ActiveDrawer>>` guarantees mutual exclusion. Each
drawer gets its own document-level Esc listener; the RangePickerInput
inside handles Esc precedence (first Esc disarms the range picker,
second Esc closes the drawer via `on_close`).

## The Worksheet pipeline — demand-driven frame sequence

This is the heart of the binding layer. Reactive `Effect`s push typed
state/dirty intent into IronCanvas and call a shared `poke` closure.
`use_one_shot_raf` coalesces those pokes to animation frames, runs the
paint closure, then self-pauses when the closure returns `false`.

The scheduler remains active only while initial canvas construction is
waiting for mounted, non-zero-size nodes or recording playback is
running. A burst such as a held arrow key still produces at most one
paint per browser frame, but idle worksheets request no animation
frames.

### Bringing IronCanvas up — lazy construction

```
Worksheet():
  grid_ref / overlay_ref       = NodeRef
  canvas_handle               = StoredValue<Option<IronCanvas>>::new_local(None)
  on_cleanup → canvas_handle.take().dispose()
  container_ref                = NodeRef<Div>
  use_resize_observer(container_ref) → ic.resize(w, h, dpr); poke()
                                       // resize() itself self-invalidates
                                       // (drops last_frame) on a real
                                       // size/DPR change; no separate
                                       // structural event needed

poke = use_one_shot_raf(|| -> bool {
  if canvas_handle.is_none() {
    if both refs Some AND grid.client_size > 0:
      let adapter = Rc::new(WorksheetModelAdapter { store: model })
                    as Rc<dyn CanvasModel>;
      ic = IronCanvas::create(grid_el, overlay_el)?;
      ic.resize(w, h, window().device_pixel_ratio());
      ic.set_model(adapter);
      ic.setThemeFromElement(<html>);
      push current state via the same setters the Effect uses;
      canvas_handle.set(Some(ic));
    else:
      return true;                  // poll until construction is possible
  }
  tick playback if active;
  let ic = canvas_handle.as_mut().unwrap();
  if theme_dirty.take_value() { ic.setThemeFromElement(<html>); }
  let result = ic.paint_if_dirty(); // JsPaintResult: Idle/Painted/Retry/Playback
  let action = scheduling_after(result, playback_is_still_playing);
  publish trace / count frame / update timing per `action`;
  return action.keep_alive;         // Retry forces true; Idle/Painted/Playback
                                     // hand back playback_is_still_playing
});
```

`scheduling_after(result, playback_is_still_playing) -> SchedulerAction`
(`raf_loop.rs`) is the single place that turns a tick's `JsPaintResult`
into what happens next: `Idle` and `Playback` skip the frame counter,
trace publish, and timing sample, and hand `playback_is_still_playing`
straight through; `Painted` grants all three; `Retry` publishes the
trace (so a held attempt is visible in the perf panel) and forces
`keep_alive` to `true` regardless of playback state, so the loop stays
armed until the held attempt commits on a later tick with no new
external poke.

The lazy block runs only while construction is pending. Subscriptions,
resize, workbook swaps, font-load completion, camera changes, and
playback commands call `poke()` when they create work.

### The `WorksheetModelAdapter`

```rust
struct WorksheetModelAdapter {
    store: ModelStore,
    show_headers: Split<bool>,
}

impl CanvasModel for WorksheetModelAdapter {
    fn get_selected_sheet(&self) -> u32 {
        self.store.with_value(CanvasModel::get_selected_sheet)
    }
    // ... model reads delegate through ModelStore;
    // header visibility reads show_headers
}
```

`ModelStore` and `Split<bool>` are `Copy`, so the adapter is freely `'static`. Wrapping it
in `Rc<dyn CanvasModel>` makes one stable pointer that survives
workbook swaps — when `storage::load_selected` replaces the inner
`UserModel`, every adapter method picks up the new value on its next
`with_value` borrow, with no canvas reconstruction.

### The reactive overlay memo

```rust
let reactive_overlay = Memo::new(move |_| {
    let extend_to = match state.drag.get() {
        DragState::Extending { to_row, to_col } => Some(AutofillTarget { row: to_row, col: to_col }),
        _ => None,
    };

    let editing_cell = state.editing_cell.get();
    let mut formula_refs: Vec<ActiveRef> = editing_cell
        .as_ref()
        .map(|e| e.formula_analysis.refs().to_vec())
        .unwrap_or_default();

    if let Some(override) = state.dragged_ref_override.get()
        && let Some(r) = formula_refs.get_mut(override.idx)
    {
        r.sheet_area = override.range;
    }

    let point_range = match (state.drag.get(), editing_cell.as_ref()) {
        (DragState::Pointing { ref_node, .. }, Some(e)) => {
            let range = ref_node.area(&e.address);
            (range.sheet == selected_sheet).then_some(range.area)
        }
        _ => None,
    };

    OverlayTuple { extend_to, point_range, formula_refs }
});
```

Why a memo and not direct reads inside the subscribe-Effect: if the
Effect subscribed to `drag` directly, `set_drag(Selecting)` on
mousedown would cause an extra Effect run *before* the navigation
event fires. The memo's `PartialEq` gate collapses
`Idle ↔ Selecting` to "same tuple, no re-run".

### The subscribe-Effect

```rust
Effect::new(move |prev: Option<OverlayTuple>| {
    let has_content   = !events.content.get().is_empty();
    let has_format    = !events.format.get().is_empty();
    let has_nav       = !events.navigation.get().is_empty();
    let has_structure = !events.structure.get().is_empty();
    let has_theme     = !events.theme.get().is_empty();
    let overlay        = reactive_overlay.get();
    let overlay_changed = prev.is_some_and(|p| p != overlay);

    if !(has_content || has_format || has_nav || has_structure || has_theme || overlay_changed) {
        return overlay;
    }
    poke();

    let overlays = RenderOverlays { /* … from overlay tuple + clipboard_draw … */ };
    if has_theme { theme_dirty.set_value(true); }

    canvas_handle.update_value(|slot| if let Some(ic) = slot.as_mut() {
        ic.set_overlays(overlays);
        if has_structure || has_format {
            ic.requestRepaint();         // drops last_frame → Fresh
        } else if has_content {
            for event in content_events {
                match event {
                    CellChanged(address) =>
                        ic.mark_rows_damaged(address.sheet, address.row, address.row),
                    RangeChanged(area) =>
                        ic.mark_rows_damaged(area.sheet, area.r1, area.r2),
                    FormulaChanged | CalculationUpdated | NamedRangesChanged =>
                        ic.mark_content_dirty(),
                }
            }
            if has_nav { ic.request_overlay_repaint(); }
        } else if has_nav {
            ic.request_overlay_repaint();// Overlay regime, cheapest
        }
    });
    overlay
});
```

This Effect is the single fan-out point from RustyCalc state into
IronCanvas dirty bits. The mapping is deliberate and matches the
canvas's `PaintRegime` cascade (see `iron-canvas/ARCHITECTURE.md`):

| RustyCalc signal | IronCanvas call | Canvas regime selected |
| --- | --- | --- |
| theme | `setThemeFromElement` in rAF + `requestRepaint` | `Fresh` |
| structure or format | `requestRepaint` | `Fresh` (geometry may move) |
| `CellChanged` | `mark_rows_damaged(sheet, row, row)` | `Damage` when slots are reusable |
| `RangeChanged` | `mark_rows_damaged(sheet, r1, r2)` | `Damage` when slots are reusable |
| formula/recalc/named-range content | `mark_content_dirty` | conservative `SlotsReuse` fallback |
| content + navigation | content call + `request_overlay_repaint` | grid regime + overlay |
| navigation alone | `request_overlay_repaint` | `Overlay` |
| overlay memo change | `set_overlays` | `Overlay` |

Every non-empty batch calls `poke()` before pushing the dirty state.
Multiple content events accumulate row spans inside the engine; one
unrowed event poisons the batch to the conservative path.

### Recording and playback dispatch Effects

Two additional `Effect`s in `<Worksheet>` handle the dev-tools
recording/playback lifecycle, gated behind `#[cfg(feature = "dev-tools")]`.
They follow the same one-shot drain pattern as the subscribe-Effect:

```rust
// Recording Effect — drains RecordingCmd::{Start, Stop}
Effect::new(move |_| {
    let Some(cmd) = app.recording_cmd.get() else { return; };
    canvas_handle.update_value(|slot| {
        match cmd {
            RecordingCmd::Start => ic.start_recording(JsValue::UNDEFINED)?,
            RecordingCmd::Stop  => { let bytes = ic.stop_recording()?;
                                     trigger_download(bytes, "recording-{ts}.icr"); },
        }
    });
    app.recording_cmd.set(None);
});

// Playback Effect — drains PlaybackCmd::{Load, Seek, Play, Pause, Exit}
Effect::new(move |_| {
    let Some(cmd) = app.playback_cmd.get() else { return; };
    canvas_handle.update_value(|slot| {
        match cmd {
            Load(bytes)  => { ic.load_recording(&bytes)?; poke(); },
            Seek(idx)    => { ic.seek_recording(idx)?; poke(); },
            Play         => { ic.play_recording(perf::now())?; poke(); },
            Pause        => ic.pause_recording(),
            Exit         => { ic.exit_playback(); poke(); },
        }
        // mirror result state back into AppState signals
    });
    app.playback_cmd.set(None);
});
```

Both Effects fire outside the event-bus path — recording/playback
commands originate from the `<PerfPanel>` and `<PlaybackPanel>` UI
buttons, not from spreadsheet mutations. The commands write into
`Split<Option<RecordingCmd>>` / `Split<Option<PlaybackCmd>>`, the
Effect drains them onto `IronCanvas`, then clears the signal.

### Demand-driven rAF with playback tick

The `use_one_shot_raf` callback serves two purposes when dev-tools is enabled:

```rust
use_one_shot_raf(|| -> bool {
    // … lazy IronCanvas construction (unchanged) …

    // A successful Play poke starts the loop. Returning `playing`
    // keeps it alive until pause/end.
    #[cfg(feature = "dev-tools")]
    let playing = app.playback_loaded.get_untracked()
        && app.playback_playing.get_untracked();
    if playing {
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                if ic.tick_playback(perf::now()) {
                    app.playback_frame.set(ic.recording_current_frame());
                }
                if !ic.is_playing() { app.playback_playing.set(false); }
            }
        });
    }

    // … theme refresh + paint_if_dirty() …
    playing
});
```

`paint_if_dirty` short-circuits when a `PlaybackSession` is loaded
(the canvases are rented by the replay engine), so playback ticks
and live paints never overlap. Load/seek/exit also poke because they
change the displayed frame or request the returning live `Fresh` paint.

### Font-load invalidation

The worksheet registers a scope-bound `document.fonts` `loadingdone`
listener in `raf_loop.rs`. It calls `IronCanvas::fonts_changed()` to
clear both Canvas2D text-measure memos and mark content dirty, then
calls `poke()`. The wake is required because clearing an engine cache
does not resume a self-paused scheduler.

### `Worksheet` view tree

```rust
view! {
    <div node_ref=container_ref class="ws">
        <canvas
            node_ref=grid_ref
            role="application"
            class=move || /* base + cursor hint from state.drag / hover_cursor */
            on:mousedown=on_mousedown
            on:mousemove=on_mousemove
            on:mouseup=on_mouseup
            on:dblclick=on_dblclick
            on:wheel=on_wheel
            on:contextmenu=on_contextmenu
        />
        <canvas node_ref=overlay_ref class="ws-canvas ws-overlay" aria-hidden="true" />
        <CellEditor />
    </div>
}
```

The cursor class is composed from `DragState` (active drag wins) and
`hover_cursor` (idle hint). The active drag never flickers back to a
hover hint mid-drag because the `match` puts the drag arms first.

## Hit-test pipeline

`<canvas>` mouse events route into `src/input/mouse/` (one file per event kind plus `click.rs`, `cursor_hint.rs`, `formula_ref.rs`). Each handler
takes `(MouseEvent, ModelStore, WorkbookState, CanvasHandle)` and uses
the canvas as the authoritative source for "what is at (x, y)".

```
mousedown (x, y):
  resize_handle_at(x, y, HIT_ZONE)?               // CSS-pixel tolerance
    Some(ResizeTarget::ColumnEdge(c)) → set drag = ResizingCol { col: c, x }
    Some(ResizeTarget::RowEdge(r))    → set drag = ResizingRow { row: r, y }
  else hit_test(x, y):
    HitTest::Outside              → no-op
    HitTest::Corner               → select all
    HitTest::ColumnHeader(c)      → full-column selection (Shift extends, Ctrl toggles)
    HitTest::RowHeader(r)         → full-row selection
    HitTest::AutofillHandle{r,c}  → set drag = Extending { to_row: r, to_col: c }
    HitTest::FormulaRef{ref_idx,zone,grab_row,grab_column}
                                  → set drag = DraggingFormulaRef + dragged_ref_override
    HitTest::Cell{r,c}            → set drag = Selecting + nav_select_range(r,c)

mousemove (x, y):
  match drag.get():
    Idle           → compute_cursor_hint(x, y) → hover_cursor
    Selecting      → extend selection to hit_test(x, y).cell()
    Extending      → update Extending { to_row, to_col }
    ResizingCol/Row→ set new width/height via try_mutate(Immediate, …)
    Pointing       → splice point-mode ref into edited formula
    DraggingFormulaRef → update dragged_ref_override

mouseup:
  match drag.take():
    Selecting / Extending → emit SelectionRangeChanged
    ResizingCol/Row       → emit LayoutChanged (Format category)
    DraggingFormulaRef    → commit ghost range into editing_cell.text
                            via sync_edit; clear dragged_ref_override

wheel:
  classify deltaY/deltaX, call nav_scroll on UserModel (Deferred eval),
  emit NavigationEvent::ScrollChanged

dblclick:
  hit_test(x, y).cell() → start edit (EditAction::EnterEditMode)
```

`compute_cursor_hint(x, y)` runs `resize_handle_at` first, then
`hit_test`. The priority must match `handle_mousedown` exactly so the
cursor previews the action that would actually fire — if the hover
hint resolved to "cell" but mousedown would hit the resize handle, the
cursor would lie.

Two contracts in the canvas surface make this pipeline trustworthy:
queries read the same `last_frame: Option<Chrome>` that the renderer
emitted, so hit zones cannot disagree with painted pixels; and
`resize_handle_at` deliberately probes header strips only — cell-area
column edges return `None`.

## Formula-ref overlay flow

This is the most interesting cross-layer path because it round-trips
through every layer in the binary: user text → IronCalc lexer → typed
Rust refs → IronCanvas overlay paint → drag handle → text mutation → re-eval.

### 1. Tokenize on every keystroke

The cell editor and formula bar both fire `on:input`, routed through
`sync_edit` (in `src/input/formula/edit_sync.rs`). `sync_edit` reads
`(value, cursor)` from either `HtmlInputElement` or `HtmlTextAreaElement`,
runs `analyze_formula(text, context_cell, defined_names, sheet_names)`,
then commits via the `FormulaEditState` trait:

```rust
pub trait FormulaEditState {
    fn context_cell(&self) -> CellAddress;
    fn apply_edit(&mut self, text: String, cursor: usize, analysis: FormulaAnalysis);
}
```

`EditingCell` and `EditingDefinedName` both implement this trait. The
trait is the only place the rest of the input pipeline knows about
"some thing being edited" — point-mode logic in `keyboard/` works
against either.

### 2. `FormulaAnalysis` — paintable refs

```rust
pub struct FormulaAnalysis {
    pub status: FormulaStatus,                 // Valid / Unresolved / ParseError / LexerError / NotFormula
    pub bare_ref_spans: Vec<TextRef>,          // A1 vs Sheet1!A1
}
```

`status` carries the list of `ActiveRef` overlays when the formula is
parseable. Each `ActiveRef` wraps a `RefNode` (which preserves
absolute/relative flags and named-range identity), the byte span of
the rendered ref in the text, a `FormulaRefKind` (`Direct` /
`DefinedName` / `Unresolved`), and the resolved `SheetRange` that the
canvas paints. Color is **not** stored here — only `color_idx`. The
renderer resolves it via `theme::FORMULA_REF_COLORS[idx % N]`, keeping
presentation out of `formula_analysis`.

### 3. Memo composition

The `reactive_overlay` memo reads `editing_cell.formula_analysis.refs()`
and bounds-checks a patch to `refs[override.idx].sheet_area` from
`dragged_ref_override` (if set). The result is an `OverlayTuple` with
autofill target, visible-sheet point range, and `Vec<ActiveRef>`.

### 4. Push to canvas

The subscribe-Effect packages the memo output into `RenderOverlays`
(an iron-canvas type) and calls `ic.set_overlays(overlays)`. The
`FormulaRef`s carry their pre-resolved `SheetRange` so the
overlay paint never reads the model again. On the canvas side this
flows through `FormulaRefsLayer` (see `iron-canvas/ARCHITECTURE.md`
for the decoration trait surface).

### 5. Hover and drag

`hit_test` returns
`HitTest::FormulaRef { ref_idx, zone, grab_row, grab_column }` when the cursor
is over a painted ref outline (`RefZone` carries which edge — N / S /
E / W / corner — for cursor styling). `compute_cursor_hint` maps
`RefZone` to a cursor variant (`RefMove` / `RefExtendNS` /
`RefExtendEW` / `RefCornerNwse` / `RefCornerNesw`).

mousedown on a `FormulaRef` hit sets:

```rust
drag = DragState::DraggingFormulaRef {
    idx: ref_idx,
    anchor: original_range,
    grab_cell: (grab_row, grab_column),
};
```

mousemove translates the cursor to a new `SheetRange`, sets
`dragged_ref_override = Some(RefOverride { idx, range })`. The memo
recomputes, `set_overlays` fires, the overlay layer repaints. No
formula text mutation yet.

### 6. Commit

mouseup calls `sync_edit` with the rewritten ref substring spliced
into `editing_cell.text`. `analyze_formula` re-runs on the new text,
the result lands back in `editing_cell.formula_analysis`, the memo
recomputes, the overlay repaints. `dragged_ref_override` clears to `None`.
The cell is not yet committed to the model — only on Enter (which
fires `EditAction::Commit`).

The separate `Split<Option<RefOverride>>` is the drag preview channel:
the canvas paints the candidate range during mousemove, while only
mouseup rewrites `editing_cell.text`. This keeps full formula analysis
off the per-mousemove path.

## Edit pipeline

```
EditAction → execute_edit(action, model, state):
  Start(text):
    emit NavigationEvent::EditingStarted { address }
    set editing_cell = Some(EditingCell {
        address, text, cursor: text.len(),
        mode: Accept, focus: Cell, text_dirty: true,
        formula_analysis: default,
    })

  EnterEditMode:
    set editing_cell.mode = Edit  (entered via F2 or dblclick)

  CommitAndMove(dir):
    try_mutate(Immediate, |m| m.set_user_input(addr, text))?
    m.nav_arrow(dir)
    emit ContentEvent::CellChanged { address }
    emit NavigationEvent::EditingStopped + SelectionChanged
    clear editing_cell

  Cancel:
    emit NavigationEvent::EditingStopped
    clear editing_cell — model unchanged
```

`EditMode` decides arrow-key behavior during edit:
- `Accept` (default after a printable key) — arrows commit + navigate.
- `Edit` (F2 or dblclick) — arrows move the text cursor. `text_dirty`
  arms point-mode for the next arrow keypress if the cursor sits
  after an operator.

`EditFocus::{Cell, FormulaBar}` arbitrates which DOM element owns the
edit. The formula bar can take over from the cell editor mid-edit;
both write through `sync_edit` so `text` / `cursor` / `formula_analysis`
stay coherent.

## Keyboard pipeline

`<Workbook>` owns the top-level `on:keydown`. It classifies the key,
peels off the clipboard ops (which need async navigator APIs), and
routes everything else through `execute`:

```
on_keydown(ev):
  action = classify_key(ev.key, modifiers, editing_cell.is_some(), edit_mode)
  match action {
    SpreadsheetAction::Copy  → ev.prevent_default(); spawn copy_to_clipboard(model, state, clipboard);
    SpreadsheetAction::Cut   → ev.prevent_default(); spawn cut_to_clipboard(...);
    SpreadsheetAction::Paste → ev.prevent_default(); spawn paste_from_clipboard(...);
    other                    → execute(&action, model, &state); ev.prevent_default();
  }

execute(SpreadsheetAction):
  Nav(a)       → execute_nav(a, model, state)
  Edit(a)      → execute_edit(a, model, state)
  Format(a)    → execute_format(a, model, state)
  Structure(a) → execute_struct(a, model, state)
```

Before `classify_key` runs, `<Workbook>` applies an editing-cell **point-mode
pre-pass** that intercepts two keys while a formula is open. Arrow keys splice a
point-mode ref at the caret (arming `DragState::Pointing`); **F4** cycles the
`$`-flags of the ref under the caret through Excel's
`A1 → $A$1 → A$1 → $A1 → A1` order via `RefNode::cycle_absolute`. The cycle
re-encodes each axis against the editing cell — an absolute axis stores the
literal coordinate, a relative axis stores the offset — so the resolved target
never moves, only its markup. Both paths splice the rewritten ref back into
`editing_cell.text`, re-run `analyze_formula`, and `prevent_default` before the
key reaches `classify_key`.

Each category module follows the same shape:
1. Translate the action into a `FrontendModel` call.
2. Wrap it in `mutate(model, mode, …)` or `try_mutate(model, mode, …)?`
   with the right `EvaluationMode`.
3. Emit a typed event on success; populate `StatusMessage` on error.

`Toolbar` buttons construct `SpreadsheetAction` values directly and
call `execute`, so the keyboard and toolbar paths converge before
touching the model. Tests in `src/test/` mostly exercise this layer
(actions in, model state + events out).

## Persistence

`src/storage/` owns the localStorage protocol — the `b"RCAL"` magic+version
header, the `selected` / `models` keys, xlsx round-trip, `WorkbookId` UUIDs,
and the read/write surface that the autosave Effect drives. `src/verify.rs`
handles share-URL verification: word-hash extraction (SHA-256) and the
consent-modal gate before loading untrusted payloads.

```
key "selected"   → WorkbookId  (hyphenated UUID v4)
key "models"     → HashMap<WorkbookId, ModelEntry>
                   where ModelEntry = {
                       name: String,
                       group: Option<String>,
                       base64_xlsx: String,   // ironcalc → xlsx → base64
                       updated: ISO-8601,
                   }
```

`WorkbookId` is `[u8; 16]` with hyphenated-string `Display` / `FromStr`,
generated via `window().crypto().getRandomValues(...)`. `Copy` — no
heap.

The autosave Effect in `<App>` is `use_debounce_fn_with_options(500ms,
{ leading: false })`. It serializes the current `UserModel` to xlsx
bytes via the `ironcalc` (xlsx) crate, base64-encodes them, and writes
back into the `"models"` map.

A `beforeunload` listener does one final flush so a fast tab-close
loses no work. The listener closure is intentionally `forget()`-leaked —
it must outlive every drop in the page.

## Theme

```
leptos-use.useColorMode → mode: Signal<ColorMode>  (Light | Dark | Auto)
   ↳ persists user pref to localStorage
   ↳ writes <html data-theme=…>
   ↳ AppState exposes get_theme() / set_theme()

theme.rs::use_rusty_calc_theme()
   ↳ wires `.emit_auto(false)` so the signal never carries Auto
   ↳ default Auto: system pref via `prefers-color-scheme`

Theme tokens live in CSS — `:root[data-theme=light]` / `[data-theme=dark]`
declare `--palette-bg-cell`, `--palette-text`, etc. iron-canvas reads them
via `IronCanvas::setThemeFromElement(<html>)`.

ThemeEvent::ThemeToggled → emitted by AppState::set_theme
   ↳ subscribe-Effect sets theme_dirty
   ↳ scheduled frame reads theme_dirty, calls set_theme_from_element before paint_if_dirty
```

The `theme_dirty` fence is load-bearing: leptos-use writes
`data-theme` synchronously, but reading the new CSS variables from
`<html>` must happen after the browser has applied the attribute
change — deferring to rAF guarantees that.

`FORMULA_REF_COLORS: &[&str]` and `COLOR_PALETTE: &[&str]` are
declared in `theme.rs` so the formula overlay and the color-picker
swatches stay in sync without an extra event hop.

## Perf instrumentation

`<PerfPanel>` (mounted when `app.show_perf_panel`) combines three
commit timestamps with the duration of the latest `paint_if_dirty`:

```
commit_start  ← try_mutate, before the mutation closure
input_done    ← after  set_user_input
eval_done     ← after  UserModel::evaluate()
render_ms     ← rAF loop, elapsed time around paint_if_dirty
frame_trace   ← optional IronCanvas::frame_trace() sample while panel is open
```

Differences yield input and evaluation time; `render_ms` is a duration,
not a fourth timestamp, so total is their sum. The panel reads
`last_formula` for the committed text. Frame-trace sampling is gated on
panel visibility and prefixes a paint counter so repeated identical
verdicts are visibly fresh. `now()` wraps `performance.now()`.

## Camera tool

A *camera* is a floating, view-only "live picture" of a cell range — the
spreadsheet snapshot/camera tool. Each one paints into its own pair of
`<canvas>` elements using iron-canvas's **DataGrid** stack *directly*, not
the `IronCanvas` wasm-bindgen facade the worksheet uses:
`Orchestrator<WebSurface>` over a `DataGridModel`
(`components/workbook/camera/canvas.rs`). It is the same composition as the
`DataGridCanvas` facade, but driven natively in Rust — full `CellStyle`
fidelity with no JS wire.

### Pieces

| File                | Role                                                             |
| ------------------- | ---------------------------------------------------------------- |
| `state/camera.rs`   | `CameraSpec { id, source, pos, size, scroll, autosize }` (`Copy`) — the reactive source of truth; `PersistedCamera` is its serde mirror (coord types carry no serde, so the range flattens to ints) |
| `toolbar/camera.rs` | `<InsertCamera>` (View tab) — snapshots the current selection into a new `CameraSpec` |
| `camera/mod.rs`     | `<CameraLayer>` (`inset:0`, `pointer-events:none` overlay with a keyed `<For>` over `cameras`) mounts one `<Camera>` per spec; the widget owns drag/resize/wheel/settings/delete |
| `camera/canvas.rs`  | `CameraCanvas` — the per-widget `Orchestrator<WebSurface>` + `DataGridModel` paint stack |
| `camera/extract.rs` | `extract_grid(model, source) -> DataGrid` — eager headerless snapshot |
| `camera/watch.rs`   | `events_touch_source(source, content, format) -> bool` — re-extract gate |
| `one_shot_raf.rs`   | Shared self-pausing scheduler used by both worksheet and every camera |

`WorkbookState::cameras: Split<Vec<CameraSpec>>` holds the live set; each
`<Camera>` reads its own entry through a `Memo` keyed on `id`, so
drag/resize/persistence all mutate one copy. The settings popover re-points
the source range with a `RangePickerInput` armed via
`RangeCaptureTarget::Camera(id)` — the `id` keys the capture so multiple
open cameras never alias. The layer is `pointer-events:none` so the grid
stays interactive between widgets; each widget re-enables pointer events on
its own chrome.

### Extract and live-update

`extract_grid` walks the source range once and builds a headerless,
selection-overlay-disabled styled `DataGrid`: formatted values plus merged `CellStyle`s, with `Color::Theme`
resolved to CSS **at extract time** (via `iron-canvas-ironcalc`'s
`color_resolver` / `style_to_core`). Because color is baked in, a theme
change must re-extract — a camera is a snapshot, not a live adapter like
`WorksheetModelAdapter`.

Each `<Camera>` owns an `Effect` over the content/format event vectors.
`events_touch_source` decides whether an edit could affect the snapshot; it
is deliberately conservative — any event variant whose locality it cannot
prove disjoint returns `true`, so a camera may over-repaint but never goes
stale. On a hit it re-runs `extract_grid` and hands the fresh grid to
`CameraCanvas::set_grid`. The snapshot is strictly one-way: a camera never
writes back to the model.

Each camera uses `use_one_shot_raf`: it polls only until its canvases can
be constructed, then paints once per explicit `poke` from resize, scroll,
re-extraction, autosize, or font load. Its scope-bound
`document.fonts loadingdone` listener clears both painter measurement
memos through `CameraCanvas::fonts_changed()` and wakes that scheduler.

### Persistence

Two `Effect`s in `<Workbook>` own camera persistence, separate from the
xlsx autosave path: one loads `Vec<PersistedCamera>` from localStorage key
`rustycalc_cameras_{uuid}` when the active workbook changes, the other
re-serializes on any `cameras` mutation. Cameras therefore survive reload
and are scoped per-workbook, but are **not** part of the xlsx document.

## The iron-canvas boundary

RustyCalc is a consumer of iron-canvas's wasm-bindgen facade. The full
contract — `Surface` trait, `Painter` trait, `Chrome` snapshot,
`PaintRegime` dispatch, `FramePath` regimes, `GridSignals` bits,
overlay decoration shape — lives in `iron-canvas/ARCHITECTURE.md`.
What this side needs to know:

- `IronCanvas::create(grid_el, overlay_el)` — once, on the first scheduled
  tick after both canvases mount and the container has nonzero size. The
  grid paints into a detached back canvas and presents to the visible
  canvas; the transparent overlay draws directly.
- `set_model(Rc<dyn CanvasModel>)` — called once with the adapter;
  workbook swaps reuse the same `Rc` because `WorksheetModelAdapter::store`
  is a stable `StoredValue` handle.
- `resize(w, h, dpr)` — on `ResizeObserver` fire. Browser DPR stays
  fractional `f64` through the app, facade, core, and Canvas2D painter.
- `set_overlays(RenderOverlays)` — every reactive-overlay change.
- `setThemeFromElement(<html>)` — only after `data-theme` flipped, gated
  by `theme_dirty`.
- `mark_rows_damaged` vs `mark_content_dirty` vs `request_repaint` vs
  `request_overlay_repaint` — pick by event payload/category (see table above).
- `fonts_changed` — after `document.fonts loadingdone`; clears Canvas2D
  text-measure memos and marks content dirty.
- `paint_if_dirty` — on the next animation frame after `poke`, not from a
  permanent loop.
  Short-circuits when a `PlaybackSession` is loaded (canvases are
  rented by the replay engine).
- `frame_trace` — diagnostic sampling for the open PerfPanel; reports the
  five-regime and per-pane outcome of the last paint.
- `hit_test`, `resize_handle_at`, `cell_rect`, `autofill_handle` — read
  by mouse handlers. All resolve against the last paint's snapshot;
  before first paint they return absent variants.

### Bridge crate

`iron-canvas-ironcalc` (in the iron-canvas workspace) provides
`IronCalcModel<'a>`, a newtype that implements `CanvasModel` for IronCalc's
`UserModel`. The crate exists because Rust's orphan rule prevents
`impl CanvasModel for UserModel` outside the trait-defining crate. It also
hosts the conditional-formatting bridge: `get_extended_cell_style()` surfaces
per-cell CF decorations (data bars, icon sets, color scales) to the paint
pipeline (see `iron-canvas/ARCHITECTURE.md` for paint-pass ordering).

### DataGrid crates (camera tool)

Besides the `iron-canvas-web` wasm-bindgen facade, RustyCalc links two
iron-canvas workspace crates directly as path-deps (`Cargo.toml`):
`iron-canvas-datagrid` (the pure-Rust `DataGrid` / `DataGridModel`) and
`iron-canvas-canvas2d` (the `WebSurface` + `CanvasPainter`). The camera
tool composes them into a native `Orchestrator<WebSurface>` per widget —
the only place RustyCalc drives iron-canvas without crossing the JS
boundary. See "Camera tool".

### Dev-tools API (gated behind `#[cfg(feature = "dev-tools")]`)

**Recording** — captures every paint-level `DrawOp` into an `.icr` file:

- `recordingSupported() -> bool` — whether the build includes the recorder
  (false in prod, true with `--features dev-tools`).
- `startRecording(opts: JsValue)` — begin capture. `opts` is an optional
  `{ layers?: "both"|"gridOnly"|"overlayOnly", skipGroups?: string[] }`
  filter. Forces a synchronous `Fresh` frame 0.
- `stopRecording() -> Uint8Array` — end capture, return serialized `.icr`
  bytes. Hard-cap watchdog (100 MB) auto-stops with `partial: true`.

**Playback** — replays `.icr` recordings onto the live grid + overlay
canvases by suspending the normal paint loop:

- `loadRecording(bytes: &[u8])` — parse `.icr`, resize canvases to
  recording dimensions, paint frame 0. Refuses during active capture.
- `seekRecording(idx: u32)` — jump to frame `idx`, cumulative grid replay
  from most recent `Fresh` anchor. Pauses any active play loop.
- `playRecording(now_ms: f64)` — begin time-accurate playback anchored
  at `performance.now()`. No-op at end-of-recording.
- `pauseRecording()` — halt the play loop (idempotent).
- `tickPlayback(now_ms: f64) -> bool` — drive playback forward; returns
  `true` if the displayed frame changed. Call from rAF loop.
- `exitPlayback()` — drop session, restore canvas CSS dimensions, force
  a `Fresh` repaint so the live worksheet returns next tick.
- `isPlaying() -> bool`, `playbackActive() -> bool`,
  `recordingFrameCount() -> u32`, `recordingCurrentFrame() -> u32` —
  query API for the host's playback controls.

**Export** — render the current sheet to a self-contained document:

- `exportSvg(css_w: f64, css_h: f64) -> String` — always-on. Drives a
  throwaway `Orchestrator<SvgSurface>` against the cached live model
  without re-crossing the JS bridge, discards overlay output, embeds Inter
  Regular, and measures with Inter glyph advances.
- `exportPdf(css_w: f64, css_h: f64) -> Uint8Array` — gated behind
  `#[cfg(feature = "pdf")]` (enabled in RustyCalc via the
  `dev-tools → iron-canvas-web/pdf` feature chain). Same throwaway-
  orchestrator shape; emits only the grid surface, draws base-14
  Helvetica, and measures with matching Helvetica widths.

**Recorder filter** — `RecordingFilter` controls which ops are captured:

- `LayerScope` — `Both` / `GridOnly` / `OverlayOnly`. Disarmed surfaces
  never fork ops (zero per-op cost).
- `skipGroups: HashSet<GroupClass>` — suppress named `begin_group` /
  `end_group` brackets and all ops inside them. `GroupClass` has 15
  variants (`Grid`, `Overlay`, `Cells`, `FrozenSep`, `Headers`, `Corner`,
  `SelectionFill`, `SelectionStroke`, `Autofill`, `Clipboard`,
  `PointMode`, `FormulaRefs`, `ActiveCellRepaint`, `HeaderHighlights`,
  `Custom`).
  Uses a `skip_depth` counter to correctly handle nested groups.

The engine never mutates the model, never owns reactive signals, never
imports anything from `leptos`. Deleting the entire `src/` tree leaves
the iron-canvas workspace compiling and passing its own tests.

## Verification boundary

Native iron-canvas tests run from its workspace. Browser-only facade
tests run through the Chrome wasm runner configured in
`.github/workflows/test.yml`:

```bash
(cd iron-canvas && cargo test --workspace --locked)
(cd iron-canvas && \
  cargo test --target wasm32-unknown-unknown \
    -p iron-canvas-web -p iron-canvas-datagrid-web --locked)
```

Both wasm suites use `wasm_bindgen_test_configure!(run_in_browser)` and
cover fractional-DPR backing sizes. The iron-canvas web suite also compares
raw Canvas2D `ImageData` between retained row repaint/full-fallback output
and forced-fresh output for three border-focused scenarios. This proves
raster equivalence for those cases, not every future retained-pixel path.
