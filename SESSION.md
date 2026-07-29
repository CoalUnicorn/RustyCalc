# Deferred Issues

## Stale "no render gate to poke" doc-comment in camera/canvas.rs (2026-07-28)
`src/components/workbook/camera/canvas.rs`'s `fonts_changed()` doc-comment says
"the camera rAF loop calls `paint_if_dirty` unconditionally, so marking dirty
is enough — no render gate to poke." This predates the Camera's move to the
demand-driven `use_one_shot_raf` scheduler (`camera/mod.rs:116`): the loop is
one-shot and self-pausing, and `fonts_changed`'s one caller (the `loadingdone`
listener, `camera/mod.rs:121-135`) does call `poke()` right after. Found
while auditing every `CameraCanvas::resize` call site during Task 4 of the
transactional-render-pipeline work (confirming each already re-arms the
scheduler); out of that task's scope to fix the comment.

## Camera-tool rAF loop discards `paint_if_dirty` outcome (2026-07-28)
`src/components/workbook/camera/canvas.rs:110-112`'s `CameraCanvas::paint_if_dirty`
wraps `IronCanvas::paint_if_dirty` (now `JsPaintResult`-returning as of the
transactional-render-pipeline task 3) but discards the result, and
`src/components/workbook/camera/mod.rs:74-83`'s rAF closure always returns
`false` after one paint regardless of outcome. Unlike the worksheet's
`raf_loop.rs` (which now runs `scheduling_after`), a `Retry` (held pane) here
won't keep the camera's one-shot loop armed — a failed camera-canvas repaint
could silently stall until the next external poke. Found during the Task 3
consumer audit; out of that task's scope (only `raf_loop.rs` was named).

## iron-canvas: stale `FramePath::Blit` doc-comments (2026-06-15)
Stage 3 of the Fetched/typed-blit plan removed the `FramePath::Blit` variant
(`FramePath` is now only `Fresh` + `SlotsReuse`; blit dispatches via
`Chrome::next_blit`→`BlitOutcome`). Five doc-comments still name the dead variant:
`crates/iron-canvas-core/src/chrome/{mod.rs:134, mod.rs:357, blit.rs:8, kind.rs:14,
blit_rebuild.rs:4}`. The last is the tell — `blit_rebuild.rs` is a *newer* file
carrying a *stale* reference. Update prose to describe the `Chrome::next_blit`/
`BlitOutcome` dispatch instead.

## CF/NR drawer range-picker — out-of-scope follow-ups (2026-06-05)
Shipped: CF + Named Ranges moved from modal to non-modal right `Drawer`
(`ui/drawer.rs`); grid range-picking via an armed ⊞ button captured reactively
off `events.navigation` (`ui/range_picker.rs` + `RangeCaptureTarget`). Deferred:
- **CF formula/formula2 fields** have no own ⊞ — only the main "Range" field
  range-picks. `RangeCaptureTarget` is ready to extend (add variants) and
  `RangeFormat::QualifiedAbsolute` is already defined (currently `#[allow(dead_code)]`).
  PARTIALLY DONE 2026-06-06: the main "Value / Formula" field now has its own ⊞
  (`RangeCaptureTarget::CfFormula`) with true point-mode *insert-at-caret* via
  `splice_ref` + a remembered `prev_span` (drag grows one ref). Still deferred:
  the formula2 ("And", Between/NotBetween) field has no ⊞; and Named Ranges
  could now adopt the same insert path to retire its whole-formula-replace.
- **CF/NR formula caret is tracked only on textarea `input` events**
  (2026-06-06). `formula_cursor` updates from `read_value_and_cursor` on input,
  but NOT on plain caret moves (click / arrow inside the textarea fire no input
  event). So arming ⊞ after click-repositioning the caret without typing splices
  at a stale offset. The cell editor / formula bar share this limitation. Fix:
  also capture `selectionEnd` on `select`/`keyup`/`click` of the textarea, or
  read it live at splice time from the focused element.
- **NR "Refers to" is whole-formula-replace only.** Arming overwrites the entire
  formula with `=<Sheet!$A$1>`; no mid-formula point-mode ref insertion (would
  need the `DragState::Pointing` splice path, deliberately avoided).
- **Manual browser smoke still pending** for plan risks R2 (focused drawer input
  vs. workbook keydown early-return on grid click) and R3 (right-pinned panel vs.
  grid scrollbars). Compiles + 128 tests green, but reactive capture UX is
  unverified in a live browser.
- **`Modal` retained** — still used elsewhere; not retired.

## @src/input/format.rs — `SetBorder` reimplements `make_border_area`'s serde roundtrip (2026-06-01)
The border-migration `FormatAction::SetBorder` arm builds a `BorderArea` from an
inline `serde_json::json!({...})`, while `src/model/clipboard_bridge.rs` already
exposes a purpose-built `make_border_area(kind, style, color)` (parked behind
`#[allow(dead_code)]`). Two serde constructions for the same `pub(crate)`-field
type. Dedup by mapping `BorderSide → BorderKind` and `BorderWeight → BorderStyle`
(e.g. `From` impls next to `BorderKind`) and routing the format arm through
`make_border_area`; then drop the `as_json_str` methods on `BorderSide`/
`BorderWeight` and the `#[allow(dead_code)]` on the bridge helper. Kept inline
here to stay within the reviewed migration plan's scope.

## Pre-existing clippy `collapsible_if` in overflow.rs:55 (2026-06-01)
`cargo clippy --target wasm32-unknown-unknown` flags `if widths.with_value(Vec::is_empty) { if let Some(el) = row_ref.get() { … } }` in `OverflowRow`'s measure Effect as collapsible. Not introduced by the FileBar→toolbar merge (overflow.rs untouched). Collapsing needs a `let`-chain; verify edition support before applying. Two other pre-existing warnings live outside the toolbar: `raf_loop.rs:163` (collapsible_if) and `coord/types.rs:56` (too_many_arguments 8/7).


## @src/components/workbook/mod.rs - copy reuses content regime to repaint marching-ants

`copy_to_app_clipboard` emits `ContentEvent::RangeChanged` purely to wake the
subscribe Effect so `set_overlays` repaints the clipboard border. Copy doesn't
mutate cells, so this over-paints the content layer and trips autosave. The
clean fix is an overlay-only trigger (no `ContentEvent` category exists for the
clipboard overlay today — it rides a non-reactive `StoredValue`). Drained from
`GenericChange` during OVH-1; overlay routing deferred.

## @src-tauri/src/commands/storage.rs - `backup` command is a stub (zip crate missing)

`backup` returns an informative error until `zip = "2"` is added to
`src-tauri/Cargo.toml`. Full implementation is already written in
`docs/superpowers/plans/2026-04-15-tauri-fs-storage.md` (Task 4).

## @src-tauri/src/commands/storage.rs - `save_external` has no parent-dir guard

`save_external` calls `std::fs::write` directly — errors propagate correctly but
no `create_dir_all` guard exists. Add if external-file save-as (rename/move) is
ever supported.


## @src/input/formula_analysis.rs - stack overflow in `formula_analysis_tests`

Tests `test_known_defined_name_resolves_as_valid` and
`test_unknown_name_captured` overflow the default test thread stack
(unrelated to dynamic-freeze work — already present on branch). Raising
`RUST_MIN_STACK` does not help; root cause likely in the AST walker under
DefinedName resolution. Profile the recursion depth before re-enabling.

## @src/components/pin_range_modal.rs - CSS not yet defined

Modal renders with class names `pin-range-modal*` but no stylesheet rules
exist yet. Functional but unstyled until the CSS owner adds a rule block.

## @src/components/worksheet.rs - `startRecording` call lacked `opts` arg

iron-canvas `startRecording(opts: JsValue)` now requires an arg; consumer
was calling with zero args. Patched at line 326 with
`wasm_bindgen::JsValue::UNDEFINED` (engine treats undefined as default
filter). If a non-default `RecordingFilter` is ever wanted from this site,
swap in a `serde_wasm_bindgen::to_value(&filter)?` build.

## Pre-existing clippy errors on `recorder` branch

`cargo clippy -- -D warnings` fails before Stage 3 changes:
- `src/coord.rs:53` — `pub fn range(...)` takes 10 args (clippy ceiling 7).
- `src/input/workbook.rs:20` — `WorkbookAction::Import(UserModel)` is 712 B;
  smaller variant is 16 B; box the import payload.
- `src/components/worksheet.rs:230` — `move |prev: Option<(...)>` parameter
  type is a very complex tuple; extract a `type` alias.

None gate the wasm build; all three are clippy-only.

## @iron-canvas/RECORDING_PLAYER_DESIGN.md superseded by LIVE_PLAYBACK_DESIGN.md

The three-layer analyzer + standalone-viewer design was draft-status; the
implemented design is the live-canvas player. Old doc still on disk for
reference but no longer the plan.

## DEL on active cell — model clears but pixels stay (paint-skip bug)

Repro: select a non-empty cell (e.g. A78 containing `78`), press DEL.
Formula bar updates to empty (`fx` shows nothing) — model state cleared
correctly. The painted cell still shows `78`. Re-scrolling or any
structural change forces a repaint that clears the visible value.

Suspected paths (need to trace, not fix blind):
1. `StructAction::Clear` (or whichever action DEL routes to) may emit
   only `ContentEvent` without the active cell's pane being added to
   `pending_content`. The `SlotsReuse` arm then runs with a `mask` that
   excludes the affected pane → `render_pane` skips the 4-pass walk.
2. Per-pane fingerprint
   (`render_pane` hashes `styles + values + cell_types`) — unlikely to
   collide between `78` and empty, but worth confirming the bulk-fetch
   reflects the cleared value before the hash runs.
3. The active-cell-repaint hook in `OverlayRenderer::repaint_active_cell`
   may be redrawing cached pixels rather than re-resolving the
   `CellPaint` from the (now empty) model.

Note: distinct from the 2026-05-15 blit+content collision fix
(`project_blit_content_collision_bug.md`) — that case required scrolling.
This one is stationary.

Start trace at `src/input/keyboard.rs` (DEL classification) →
`src/input/structure.rs` (action handler) → emitted event category →
`src/components/worksheet.rs` subscribe-Effect (which IronCanvas setter
fires) → `Orchestrator::decide` regime selection.

## @iron-canvas/crates/iron-canvas-recorder/tests/golden_fixture.rs — version drift after beta.2 bump

`golden_fixture_round_trip_matches_disk` and
`overlay_paint_round_trip_matches_disk` fail on the `rust-2024` branch.
Byte-level diff shows the recorder emits
`"iron_canvas_version":"0.1.0-beta.1"` while the on-disk golden contains
`"0.1.0-beta.2"` (workspace was bumped beta.1 → beta.2 in the same
`Cargo.toml` edit that switched to edition 2024).

Either the recorder hardcodes the version instead of reading
`env!("CARGO_PKG_VERSION")`, or the golden fixture on disk was
regenerated against the bumped version while a stale const was missed.
Locate the version source in `iron-canvas-recorder/src/` and decide
which side to align.

## Deferred (iron-canvas)

- iron-canvas tests/header_visibility.rs: `paint()` and `overlay_paint()` helpers are near-duplicates (differ only in `grid_surface` vs `overlay_surface`). Fine at 2; if a third surface-paint helper appears, parameterise them.

## @iron-canvas/crates/iron-canvas-core/src/chrome/blit.rs — frozen-band blit 1px-narrow when a header is hidden

`frozen_band_x = prev.row_header_thickness + CELL_AREA_INSET` (line ~331) and
`frozen_band_y = prev.col_header_thickness + CELL_AREA_INSET` (line ~385)
reconstruct the cell-area edge from `thickness + INSET` instead of the
authoritative `cell_origin`. With the header-visibility feature, a hidden strip
has `thickness == 0` while `cell_origin == 0`, so the band evaluates to `1`
(=CELL_AREA_INSET) and the frozen-pane blit is 1px too narrow on that edge,
leaving a stale 1px strip.

Triggers only when ALL THREE hold: a header hidden + frozen panes on the cross
axis + a scroll-blit while that state is stable. `screen_for_blit` does NOT
compare header thickness, so the path is reachable once Task 5 wires a stable
toggle (not reachable today — engine has no internal visibility toggler).
Purely cosmetic (1px); `draw_corner_box` is already gated off in that state.

Cleanest fix: base `frozen_band_x/y` on `prev.cell_origin.x/y` instead of
`thickness + CELL_AREA_INSET`. Add a hidden-header × frozen-pane × scroll-blit
test alongside the fix (header_visibility.rs has no blit/frozen coverage today).
Pair this with the Task 5 toggle wiring — that's when it becomes live. See plan
`docs/plans/2026-05-30-header-visibility.md`.

## @src/input/formula/input.rs — sibling slice sites inherit the UTF-16/UTF-8 caret assumption (2026-06-03)

`insert_newline_at_caret` (edit_sync.rs) was fixed to convert the DOM's UTF-16
`selection_end` offset to a UTF-8 byte offset via `utf16_offset_to_byte` before
slicing the value (the "SALES DASHBOARD — FY 2026" em-dash split the year). The
same raw-cursor-as-byte-offset assumption lives in `splice_ref` /
`splice_dragged_ref` / `try_point_move` (input.rs) — they slice formula text at
the cursor to insert refs. Only bites when a multibyte char sits before the
caret (non-ASCII sheet name, unicode in a string literal), so lower-frequency,
but the same class of bug. Route those slice sites through `utf16_offset_to_byte`
too (promote it from `fn` to `pub(crate)` in edit_sync.rs). Consider whether the
fix belongs at the boundary in `read_value_and_cursor` instead — but note the
cursor round-trips *back* to the DOM (`set_selection_range`) in UTF-16, so the
"native" cursor space must stay UTF-16 and conversion happen only at slice sites.

RESOLVED 2026-06-16 — fixed at a *third* boundary the note didn't weigh:
`sync_edit` (not `read_value_and_cursor`, not the slice sites). All five
`sync_edit` callers pass the raw UTF-16 `selection_end`; one
`utf16_offset_to_byte(&value, cursor)` there makes the *stored*
`EditingCell.cursor` canonically a byte offset, so every slice site
(`splice_ref`/`splice_dragged_ref`/`try_point_move`), plus `is_in_reference_mode`
and `refs_at_cursor`, and the latent twin in `mouse/click.rs:124,164`, all
receive bytes with no per-site conversion. The note's stated blocker — "the
cursor round-trips back to the DOM in UTF-16" — was verified FALSE for the
*stored* cursor: both `set_selection_range` sites (edit_sync.rs:180,
formula_text_area.rs:57) use a *local* value, never `edit.cursor`.
`read_value_and_cursor` stays UTF-16 (so `insert_newline_at_caret`'s DOM restore
+ local slice are unaffected), and `utf16_offset_to_byte` stayed private (same
module — no `pub(crate)` promotion). Regression test `src/test/edit_sync.rs`
(red `left:2 right:3` → green; 59/59 headless Chrome). Optional further hardening
(deferred): a `ByteOffset`/`Utf16Offset` newtype to make the unit a compile-time
invariant — large blast radius (state structs, all mouse files, `TextRef`), out
of scope for the formula fix.

## @iron-canvas/crates/iron-canvas-core — CF uses per-cell get_extended_cell_style, no batched accessor (2026-06-05)

CF integration (Phase B) added only the singular `CanvasModel::get_extended_cell_style`.
The pane render loop fetches plain styles in one batch (`get_cell_styles_in`) and then
calls `get_extended_cell_style` per painted cell to derive CF fill/font + decoration.
For the `rusty-calc` app path (`WorksheetModelAdapter → UserModel`) this is an in-process
Rust call, so cost is comparable to `get_cell_style` — acceptable. BUT the `JsBackedModel`
(pure-wasm/JS bridge) path would pay one JS round-trip per visible cell. If that path is
ever used for CF, add a batched `get_extended_cell_styles_in` (mirror `get_cell_styles_in`)
and switch `render_pane`/`render_pane_strip` to it. (Plan Risk 4.)

## @iron-canvas CF B3 design note — cf_fill_color/cf_font_color fields dropped (2026-06-05)

The plan's B3 listed three `CellPaint` fields (cf_fill_color, cf_font_color, cf_decoration).
Implemented with only `cf_decoration: Option<CfDecorationPaint>` — CF fill/font ride the
existing `style` field, which B5 sources from `ExtendedStyle::style` (base style with the CF
dxf overlay already applied). Matches the old `conditional-fmt` branch's CellPaint shape.

## @iron-canvas CF B4 — canvas/SVG painters never visually draw decorations (2026-06-05)

B4 added `Painter::paint_cf_decoration(&self, rect, deco)` as a **default no-op** plus a
`RecorderPainter` op (`DrawOp::CfDecoration`) — a faithful port of `conditional-fmt`, which
left `iron-canvas-web/src/canvas_painter.rs` and `iron-canvas-export/src/svg/painter.rs`
riding the no-op default. Consequence: CF data-bars/icons/ratings are *recorded* but never
*rendered on screen* (CF fill/font DO render — they ride `style`). The plan's B4 table lists
canvas/svg impls, but the reference branch never wrote them, so there is no source-of-truth
code to port — implementing them is net-new design (data-bar rect, icon glyph/emoji via
`fill_text`, rating stars). DECISION NEEDED before the feature is user-visible: implement the
Canvas-2D `paint_cf_decoration` (and optionally SVG) or defer decorations to a follow-up and
ship CF fill/font only. RESOLVED 2026-06-05: decorations DEFERRED (user choice). B5 ships CF
fill/font (renders) + resolves `cf_decoration` + calls the no-op `paint_cf_decoration`.

## @iron-canvas CF B5/B6 — pane fingerprint does not include CF extended style (2026-06-05)

B5 sources each cell's paint from `model.get_extended_cell_style(...)`'s `extended.style` (CF
dxf overlay baked in) when present, in both `render_pane` and `render_pane_strip`. BUT the
paint-skip fingerprint (`compute_pane_fingerprint`) hashes only the *plain* `pane_styles` +
`pane_values` + `pane_cell_types` — NOT the CF extended style. Value-driven CF reflows fine
(the value change moves the fingerprint). A CF *rule edit* that changes no plain value/style/
type would NOT move the fingerprint → a `reuses_slots()` frame could paint-skip and show stale
CF. B6 must guarantee CF mutations force a NON-`reuses_slots` (full) redraw so the skip is
bypassed. RESOLVED 2026-06-05 — the format-event path already does exactly this (see B6 note).

## @rusty-calc CF B6 — NO cf_version signal; reuse the format-event → request_repaint path (2026-06-05)

The plan's B6 said add `cf_version: RwSignal<u64>` to app_state.rs (mirror `registry_version`).
DECISION: don't. The codebase already has the right mechanism, and a pre-existing (currently
`#[allow(dead_code)]`) `FormatEvent::ConditionalFormattingChanged { sheet }` variant proves it
was the intended design. Chain:
  `state.emit_event(SpreadsheetEvent::Format(FormatEvent::ConditionalFormattingChanged{sheet}))`
  → `worksheet/subscribe.rs` sees `has_format` → `ic.request_repaint()`
  → `Orchestrator::request_repaint` sets `last_frame = None` (orchestrator.rs:153)
  → `decide()` can no longer take SlotsReuse/Viewport (they gate on `last_frame.is_some()`)
  → falls to the **Fresh** full regime → slot vecs re-walked from the model → CF re-resolved
    via `get_extended_cell_style` → fingerprint paint-skip is bypassed.
So B6 adds NO app_state signal and NO new effect read. The only wiring is the *emit*, which
lives in B7's CF mutation handlers (add/update/delete a rule). When B7 emits it, REMOVE the
`#[allow(dead_code)]` on `FormatEvent::ConditionalFormattingChanged` (events/format.rs).

## @src/components/workbook/worksheet/autofit.rs — row-fit emits one undo step per row (2026-06-03)

`install_autofit_effect` dispatches `SetRowHeight` via `execute(...)` per affected
row, each landing as a separate undo entry *after* the content commit. A multi-row
paste → several row-height entries; Ctrl+Z peels them off one-by-one before the
actual content change. Excel coalesces row-fit into the originating edit. Also
confirm the effect doesn't storm on workbook *load* if load emits per-cell
`ContentEvent::CellChanged`. Not a correctness bug; UX-granularity only.

## @iron-canvas/crates/iron-canvas-core/src/style.rs — `IconSpec = String` is a v1 placeholder (EXT-5 Stage 0, 2026-06-06)

`IconSpec` is a bare `type IconSpec = String` alias — no compile-time constraint,
so callers will pattern-match on raw strings once any backend reads it. Harmless
today because `paint_cf_decoration` is a no-op. Before a backend actually renders
icons, promote to a newtype (`pub struct IconSpec(pub String)`) so the icon
vocabulary is type-checked rather than stringly-typed. Surfaced by the Stage 0
code-quality review.

## @iron-canvas/crates/iron-canvas-web/src/wasm/mod.rs — `JsBackedModel` has no CF decorations (EXT-5 Stage 3, 2026-06-06)

`JsBackedModel::get_extended_cell_style` falls back to the `CanvasModel` trait
default (`None`), so the pure-wasm/JS bridge path renders NO conditional-format
decorations (data-bars/icons/ratings). `IronCalcModel` (in-process Rust path)
maps them via `cell_decoration_from_extended`. To enable CF decorations through
the JS handle: add a `getExtendedCellStyle` JS bridge method and convert its
payload to a core `CellDecoration`. Mirrors the existing batched-accessor
deferral above (CF uses per-cell `get_extended_cell_style`).

## RustyCalc app + iron-canvas-web duplicate `ic::Style → CellStyle` conversion (EXT-5 Stage 3, 2026-06-06)

The IronCalc→core style conversion now lives in three places: the canonical
`iron-canvas-ironcalc::convert::*_to_core` free fns, plus a private
`ic_style_to_core` copy in `iron-canvas-web/src/wasm/mod.rs` (the web crate does
not depend on the bridge). The RustyCalc app's `WorksheetModelAdapter` was
migrated to use the bridge directly (app now deps `iron-canvas-ironcalc`). If
`iron-canvas-web` ever takes the bridge as a dep, delete its local
`ic_style_to_core` and route through `convert::style_to_core`.

## Deferred (Stage D)
- iron-canvas-datagrid `model.rs:155` `set_cell`: clippy `collapsible_if` on nested `if let` (pre-existing, untouched by Stage D). Collapse with let-chains or `and_then` when that function is next edited.

## Deferred (docs — stale "blanket impl on UserModel")
- The `impl CanvasModel for UserModel` blanket was removed in EXT-5 (IronCalc
  access now goes through the `IronCalcModel` newtype in `iron-canvas-ironcalc`).
  Two docs still claim the old blanket exists: `canvas-patterns` SKILL.md
  (`core/model_adapter.rs` row — "blanket impl on `UserModel`") and
  `iron-canvas/ARCHITECTURE.md:6` (intro — "blanket-impl'd over IronCalc's
  `UserModel`"). Not Fetched-related, so left untouched during the A-3 Phase-4
  doc pass; fix when next editing those docs.

## Deferred (IronCalc Color/Theme migration — see docs/plans/2026-06-10-ironcalc-color-theme-migration-plan.md)
- ColorPicker cannot author theme colors: UI writes only `Color::Rgb`; upstream
  supports `"[idx, tint]"` via `Color::from_param` + `getThemeList`. Follow-up:
  theme palette section in ColorPicker.
- No theme UI / repaint wiring: nothing calls `set_theme` yet. When a theme
  switcher lands, a theme change invalidates every resolved color, so the paint
  gate must treat it as content-dirty (same class as the blit/content collision
  bug) — and the wasm host must call `themeChanged()` on the canvas.
- Pre-existing lint/fmt drift found during the Step-5 sweep (untouched by the
  migration, left alone for scope): iron-canvas native clippy warns in
  `iron-canvas-datagrid-web/tests/interactive_native.rs` (4) and
  `iron-canvas-core/tests/paint_skip.rs` (1); `cargo fmt --all --check` diffs
  in `iron-canvas-core/src/renderer/cell/{mod,paint}.rs`,
  `iron-canvas-export/src/svg/painter.rs`, `iron-canvas-recorder/src/lib.rs`.
  RustyCalc clippy: `toolbar/overflow.rs`, `raf_loop.rs` (collapsible-if),
  `coord/types.rs` (8-arg fn).
- CF rule list: keyed `<For>` (key = priority) keeps the DOM node when a rule
  is updated in place, so the displayed label/range text can go stale after an
  edit that changes the range. Display-only (index/cf lookups now resolve at
  click time); fix would be a composite key or reactive item fields.

## Deferred (Camera tool — see docs/plans/2026-06-11-camera-tool.md)
- Named-range dropdown for camera re-pointing needs an A1→`SheetRange` parser;
  none exists in `src/coord/`. Re-point currently captures the selection
  structurally while the RangePicker is armed; hand-typed text is not parsed.
- Per-row heights inside a camera: `DataGrid` has only a single grid-wide
  `default_row_h`; the camera uses the source's first-row height. Real fix is
  per-row heights in iron-canvas-datagrid.
- Camera widget structural styles are inline `style=` strings in
  `camera/mod.rs`; only the popover buttons use stylesheet classes
  (`styles/worksheet/camera.css`, `cam-` prefix). Migrate the rest when the
  widget design settles.
- `watch.rs` has a private `overlaps()` AABB helper; the natural home is a
  `CellArea::intersects` method in `src/coord/types.rs` next to `contains`.
- `FormatEvent::DocumentColorsChanged` maps to `false` in
  `events_touch_source` — safe only because nothing emits it today. If
  document colors ever feed resolved cell styles, route it via the theme bus
  (which re-extracts unconditionally) or flip to `true`.
- `DataGridCanvas::resize` in `iron-canvas/crates/iron-canvas-datagrid-web/src/lib.rs:98`
  has the same missing `request_repaint()` after `orch.resize` (black canvas after
  JS-driven resize). Out of scope for the camera follow-up plan. RESOLVED
  2026-07-29 — the transactional-render-pipeline work made
  `Orchestrator::resize` itself self-invalidating (drops `last_frame`, raises
  `STRUCTURAL | OVERLAY`, no-ops only when size *and* DPR are both unchanged),
  so the missing-`request_repaint()` gap closed at the engine level with no
  change to `DataGridCanvas::resize` itself, which only ever forwards to
  `orch.resize`. Does not touch the *separate*, still-open gap where
  `CameraCanvas`'s own rAF wrapper discards its `paint_if_dirty` outcome (see
  the two 2026-07-28 camera-tool entries above).
- Re-pointing a camera (settings → Apply) keeps the old widget size for the new
  range; it could set `spec.autosize = true` so the next frame re-fits. Today the
  user must double-click the resize handle. UX rough edge, not a correctness bug.
- Blit refactor (`9d67b79`) left two stale `FramePath::Blit` references in
  `iron-canvas/crates/iron-canvas-core/src/chrome/mod.rs` (the `stale_panes`
  field doc ~L137 and a comment ~L370). That variant was removed — `FramePath`
  is now `Fresh`/`SlotsReuse` only, and the blit path moved to
  `Chrome::next_blit` → `BlitOutcome`. Doc-only comment drift; the
  ARCHITECTURE.md prose is already corrected (anchor e639070).

## @IronCalc(fork)/xlsx/src/export/worksheets.rs — external hyperlink export emits `r:id="TODO-ext-…"` placeholder (2026-07-12)

On the `drawing-dream` branch, the hyperlinks export section writes a literal
`r:id="TODO-ext-{cell_ref}"` for external links (`hl.target.is_some()`) and
never emits the matching relationship in `sheetN.xml.rels` — the exported xlsx
is invalid for external links (internal `location=` links are fine). Marked
`TODO` in the code. Fix is the `RelsBuilder` in stage E1 of
`docs/designs/2026-07-12-drawing-dream-canvas-integration.md`; drawings export
(E3) reuses the same builder, so fix this first.

## Orchestrator: blit-abort does not hold `last_frame` (2026-07-23, from Fix Wave 1 / Fix B)

Fix B made `LayerBase::paint_grid_blit` a no-op frame when the revealed-strip
preflight (`prefetch_blit_strips`) hits a `BridgeFailed` — correctly leaving
the screen showing the un-scrolled pixels. But `Orchestrator::paint_viewport_regime`
still advances `last_frame` to the (un-painted) scrolled `frame1` afterwards,
so a subsequent frame diffs against a geometry that was never actually drawn.
The brief scoped Fix Wave 1 to core/layer/cell only and explicitly deferred any
fallback-full-repaint / reconciliation to a future frame. Follow-up: have the
Viewport arm detect the no-op (paint_grid_blit could report it) and NOT advance
`last_frame`, so the next paint re-attempts the scroll once the bridge recovers.

RESOLVED 2026-07-29 — `LayerBase::paint_grid_blit` now returns
`BlitPaint::{Painted, Held}` (the "could report it" mechanism this entry
asked for). On `Held`, `Orchestrator::paint_viewport_regime` restores
`last_frame` to a `Clone` of the pre-attempt `Chrome` taken before the
attempt, presents nothing, and returns `PaintResult::Retry`; the caller
re-raises the drained `GridSignals` so the next tick re-attempts the scroll
with no new external signal once the bridge recovers — exactly the
follow-up this entry asked for. Covered by
`held_viewport_presents_nothing_and_keeps_query_geometry` and
`held_viewport_retries_after_bridge_recovery` in
`crates/iron-canvas-core/tests/held_frame.rs`.

## @iron-canvas/crates/iron-canvas-core/src/renderer/cell/mod.rs — `unshiftable_pane_is_safe` has no test (2026-07-24)

The blit preflight only staged strips for panes classifying as
`PaneShiftPrep::Shifted`. Panes that classify `MissingCache` /
`IncompatibleRange` / zero-delta were skipped, but `paint_grid_blit` shifts
their pixels anyway and `render_grid_blit` only then routes them to the full
`render_pane` — which on a `Blitted` frame (`reuses_slots() == true`) holds the
prior buffers and returns WITHOUT painting on `BridgeFailed`. Stale, misplaced
pixels: the same failure the preflight exists to prevent, via a second door.

Closed by `unshiftable_pane_is_safe`, which bridge-validates that pane's own
full-range fetch and aborts the frame only if it actually fails. (A blanket
abort was rejected: `IncompatibleRange` is the ordinary page-down verdict, so
it would drop a frame on every jump scroll.)

No test covers it. Needs a fixture where a pane is in `stale_panes` with a cold
cache AND the bridge fails on the same frame — closest template is
`blit_preflight_bridge_failure_aborts_frame_without_shifting` in
`tests/scroll_blit.rs`. Note the abort interacts with the `last_frame` entry
above: both take the no-op-frame path.

RESOLVED 2026-07-29 — added `cold_cache_bridge_failure_holds_the_whole_blit_frame`
in `crates/iron-canvas-core/tests/scroll_blit.rs`, using exactly the fixture
this entry asked for: `BottomRight` invalidated to a cold (`None`) cached
range inside `stale_panes`, then a bulk bridge failure on the same frame.
Asserts `prefetch_blit_strips` returns `false`, the grid layer emits zero new
`DrawOp`s, and the pane's cached range is left untouched — the whole-frame
abort `unshiftable_pane_is_safe` exists to guarantee.

## @iron-canvas/crates/iron-canvas-core/src/renderer/cell/mod.rs — fingerprint skip never saves the model round-trip (2026-07-24)

Recorded as invariant I1 in
`iron-canvas/docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md`.

`render_pane` bulk-fetches all four buffers BEFORE it rebuilds the scratch tree
and before `plan_pane_repaint` returns any verdict. So a `Skip` saves the
five-pass walk but never the four `*_in` calls — on `JsBackedModel` that is four
bridge crossings per pane per frame regardless of outcome. Plausible floor under
the observed 20–50 ms baseline (the paint-skip work targeted the 60 ms spikes,
which are a different mechanism entirely — see the same doc).

Inherent to the current shape: you must fetch to know whether content changed.
Escaping it means trusting the CONTENT signal instead of verifying it, and
`decide()` currently falls back to `PaneRegionMask::ALL` whenever
`pending_content` is empty (`orchestrator.rs:524-528`), so an unqualified
CONTENT raise refetches every pane. Would need the signal/mask plumbing to carry
real per-pane information first. Explicitly a non-goal of the 2026-07-24 design
— larger blast radius, and the fingerprint's own doc argues the unconditional
refetch is what catches callers who forget to raise CONTENT.

## @src/components/workbook/worksheet/subscribe.rs — recalc poisons the Damage regime on every cell edit (2026-07-25)

Measured with the new `FrameTrace` readout, one-cell edit, four visible panes:

```
SlotsReuse[CONTENT | OVERLAY] tl:skip tr:skip bl:skip br:rows1/1 fetched=2052
```

The paint is already minimal — three panes fingerprint-skip, the fourth repaints
exactly one row band. The regime is not: this should have been `Damage`, which
strip-fetches only the damaged band. Instead 2052 cell slots (513 cells x 4 bulk
accessors) cross the bridge to repaint one row.

Mechanism, confirmed in source: an edit emits `CellChanged` (rowed) AND
`CalculationUpdated` (un-rowed) in the same batch. The first routes to
`mark_rows_damaged`, the second to `mark_content_dirty` — which calls
`pending_damage.poison()` (`orchestrator.rs:390-394`). `decide()`'s `Damage` arm
then fails its `CellDamage::Rows` guard and falls through to `SlotsReuse`
(`orchestrator.rs:510-521`).

The poison itself is correct in general: a recalc can change arbitrary cells, so
a rowed damage claim is not safe to keep alongside an un-rowed raise. The
question is whether `CalculationUpdated` can carry its affected cells, letting
the consumer raise rows for it too — if so, the ordinary edit path reaches
`Damage` and drops per-edit bridge traffic by roughly the pane/band ratio. Check
whether the IronCalc event exposes the recalculated addresses before assuming it
can. If it cannot, this is a permanent tax on every edit and belongs in the I1
discussion above rather than as a bug.

Note this is a *different* cost from the 60 ms scroll spike that
`iron-canvas/docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md`
targets; that design's premise (post-blit `SlotsReuse` reporting `Full`) is still
unconfirmed in the browser — no `Viewport` frame has been observed in a trace yet.

## @iron-canvas/crates/iron-canvas-core/src/renderer/cache/pane_cache.rs — the 60 ms scroll spike is an IncompatibleRange blit, not a post-blit repaint (2026-07-25)

Measured in the browser with `FrameTrace`, arrow-scrolling:

```
Draw: 55.0ms  #45 Viewport[OVERLAY] tl:- tr:- bl:- br:FULL fetched=4256
```

The frame DID choose the blit regime. It then abandoned the strip path and ran a
whole-pane `render_pane` anyway — `br:FULL` on a `Viewport` frame. `fetched=4256`
is 532 cells x 4 accessors x **2**: `unshiftable_pane_is_safe` validates the full
range, then `render_pane` fetches the identical range again moments later.

Mechanism: `shift_is_safe` (`pane_cache.rs:322-331`) requires an identical
visible row count on a row scroll — `(new.r2 - new.r1) == (prev.r2 - prev.r1)`.
A partially-visible edge row that tips the count by one yields
`IncompatibleRange`, which disqualifies the strip path. Matches the reported
symptom shape exactly: most single-row scrolls stay cheap, then one goes full.

Note `prepare_shift` also clears the pane range on `IncompatibleRange`
(`pane_cache.rs:200`), so the following frame can additionally see
`MissingCache`. A `blit_fallback` field was added to `FrameTrace` to tell the two
apart — the readout now ends `unshift(BottomRight,range)` or `unshift(...,cold)`.
Confirm which before fixing.

Remaining: let `shift_is_safe` accept a row-count change of +/-1 by treating the
extra or missing edge row as part of the revealed strip. That removes the
fallback at its source rather than making it cheaper, but it changes strip
computation and needs its own tests. Gated on the browser trace reporting
`unshift(BottomRight,range)` rather than `,cold` — if the cache is going cold
instead, the row-count theory is wrong and this fix targets nothing.

(The double-fetch half of this entry is closed: the fallback pane now adopts the
preflight's validated buffers. The pane still repaints in full.)

This supersedes the premise of
`iron-canvas/docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md`,
which targeted the frame AFTER the blit. That frame is real (proven by
`frame_trace_names_the_post_blit_slots_reuse_paint_as_full`) but is not where the
observed 55 ms lands.

## @src/components/workbook/camera — scroll dead zone when a camera pane is active (2026-07-25)

Reported while tracing paint regimes: with a camera pane active, scrolling does
not begin at the viewport edge. It activates offscreen, displaced by roughly the
height of the pane — as if the scroll trigger geometry is offset by the pane's
extent rather than clipped to it. Not investigated.

## @src/input/mouse/click.rs — click-away during edit discards the buffer instead of committing (2026-07-28)

With an edit active (`editing_cell` set) and point-mode not eligible, clicking
another cell falls through to `click.rs:200` `editing_cell.set(None)` — the
typed buffer is dropped: no commit, no event. Excel commits the pending edit on
click-away (both Enter and Edit modes). The keyboard path is safe by
construction (`classify.rs:44` routes everything through the editing arm while
`editing_cell` is set); this is mouse-path only. Surfaced by the 2026-07-28
interaction-model trace for the render-pipeline revision. Fix direction: route
click-away through the same commit path as Enter before adopting the new
selection, keeping the `may_point` guard (`click.rs:113`) intercepting first.
