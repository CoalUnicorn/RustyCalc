# Conditional Formatting Integration Plan

Status: Engine ready, renderer + UI needed. 2026-05-25.

## 1. What IronCalc Already Provides

IronCalc's conditional formatting subsystem is complete at the engine level.

### Type hierarchy (`base/src/cf_types.rs`, 493 lines)

```
CfRuleInput (user-facing API)
  ├── ColorScale { thresholds }
  ├── CellIs { operator, formula, formula2?, dxf_id, stop_if_true }
  ├── Formula { formula, dxf_id, stop_if_true }
  ├── Text { operator, value, dxf_id, stop_if_true }
  ├── TimePeriod { period, dxf_id, stop_if_true }
  ├── DuplicateValues { dxf_id, stop_if_true }
  ├── UniqueValues { dxf_id, stop_if_true }
  ├── Blanks / NotBlanks / Errors / NoErrors { dxf_id, stop_if_true }
  ├── AboveAverage / BelowAverage { dxf_id, stop_if_true }
  ├── Top10 / Bottom10 { rank, percent, dxf_id, stop_if_true }
  ├── DataBar { cfvo1, cfvo2, color, ... }
  ├── IconSet { thresholds: Vec<IconThreshold> }
  └── IconRating { rating: CfRating, dxf_id, stop_if_true }
      │
      ▼  cf_rule_from_input() adds dxf_id
      │
CfRule (stored, with dxf_id populated)
  └── Same variants minus CfRating (merged into IconSet/Rating)

ConditionalFormatting {
    range: String,       // "A1:C10" (sqref format)
    cf_rule: CfRule,
    priority: u32,       // lower = wins (first evaluated)
}

ExtendedStyle {
    style: Style,        // base + dxf overlay applied
    icon: Option<CfIcon>,
    data_bar: Option<CfDataBar>,
    rating: Option<CfRating>,
}
```

Key design point: `ExtendedStyle` splits **base style** (dxf-applied font/fill/border) from
**decorations** (icons, data bars, ratings). This is the correct split — a data bar's fill
proportion has no home in any `Style` schema.

### Evaluation engine (`base/src/conditional_formatting.rs`, 1475 lines)

```
evaluate_conditional_formatting()
  ├── For each sheet, for each CF rule (priority-sorted):
  │     ├── Parse sqref → [(r1,c1,r2,c2)]
  │     ├── For each cell in range:
  │     │     apply_cf_rule() → CfCellResult
  │     │     If stop_if_true && condition met → skip remaining rules for cell
  │     └── Insert into cf_cache: HashMap<(sheet,row,col), Vec<CfCellResult>>
  ├── Color interpolation for ColorScale rules
  ├── Data bar width computation
  └── Icon threshold matching
```

The `cf_cache` is a per-cell index — LibreOffice Calc's same pattern. It maps
`(sheet, row, col)` to the ordered list of matching CF results. Rules with
`stop_if_true` truncate the list (first match wins, no further evaluation).

### UserModel API (`base/src/user_model/conditional_formatting.rs`, 116 lines)

```rust
// Query
pub fn get_conditional_formatting_list(&self, sheet: u32)
    -> Result<Vec<ConditionalFormatting>, String>;

pub fn get_dxf_for_conditional_formatting(&self, sheet: u32, index: u32)
    -> Result<Option<Dxf>, String>;

pub fn get_extended_style_for_cell(&self, sheet: u32, row: i32, col: i32)
    -> Result<ExtendedStyle, String>;  // ← the renderer's entry point

// Mutation
pub fn add_conditional_formatting(&mut self, sheet: u32, range: &str,
    rule: CfRuleInput) -> Result<(), String>;

pub fn delete_conditional_formatting(&mut self, sheet: u32, index: u32)
    -> Result<(), String>;

pub fn update_conditional_formatting(&mut self, sheet: u32, index: u32,
    new_range: &str, new_rule: CfRuleInput) -> Result<(), String>;
```

All mutations push `Diff` variants for undo/redo. `add_conditional_formatting` reads
back the stored entry to capture the assigned `dxf_id`. `evaluate_if_not_paused()` handles
the pause/resume bracket.

### Other integration points

- **xlsx import:** `xlsx/src/import/worksheets.rs` → `load_conditional_formatting()`
- **xlsx export:** mirror path (round-trip test at `xlsx/tests/test_conditional_formatting.rs`)
- **WASM bridge:** `bindings/wasm/src/lib.rs` exposes all 5 methods to JS (with JsError mapping)
- **Undo/redo:** `Diff::AddConditionalFormatting`, `DeleteConditionalFormatting`, `UpdateConditionalFormatting`
- **Worksheet struct:** `conditional_formatting: Vec<ConditionalFormatting>` (line 120 of types.rs)

## 2. What RustyCalc Needs to Build

### Layer-by-layer

| Layer | Status | What to build |
|---|---|---|
| **Parse (L1)** | Done | IronCalc xlsx import handles CF |
| **Store (L2)** | Done | `Worksheet.conditional_formatting` in engine |
| **React (L3)** | TODO | `cf_version` signal bump on add/update/delete/evaluate |
| **Paint (L4)** | TODO | CanvasModel exposure, CellPaint extension, Painter trait method |
| **Interact (L5)** | TODO | CF side panel, rule editor dialog, range picker |

### L3: React

```rust
// In src/state.rs or events.rs:
// Bump this signal on: add_conditional_formatting, delete_conditional_formatting,
// update_conditional_formatting, after workbook-switch, after xlsx import.
pub struct WorkbookState {
    // ...
    pub cf_version: RwSignal<u64>,
}
```

The `cf_version` signal is read by the canvas Effect. When it changes, the viewport
rebuilds `CellPaint` entries that may carry `cf_decoration`.

### L4: Paint (the main work)

**Step 1: Extend CanvasModel**

```rust
// In iron-canvas-core/src/model_adapter.rs:
pub trait CanvasModel {
    // ... existing 13 methods ...

    /// Returns the CF-extended style for a cell, or the base style if no CF applies.
    /// The renderer uses this instead of get_cell_style when painting cell backgrounds,
    /// text colors, icons, and data bars.
    fn get_extended_cell_style(&self, sheet: u32, row: i32, col: i32)
        -> Option<ExtendedStyle>;
}
```

The `UserModel` impl calls `self.get_extended_style_for_cell(sheet, row, col).ok()`.
The `JsBackedModel` impl calls the WASM bridge method `getExtendedStyleForCell`.

**Step 2: Add CfDecorationPaint type**

```rust
// In iron-canvas-core/src/types/mod.rs (or new cf_types.rs):

pub struct CfIconPaint {
    pub group: IconGroup,       // Arrows, TrafficLights, Shapes, Ratings, ...
    pub icon_index: u8,         // which icon in the set
    pub color_rgb: [u8; 3],
    pub position: IconPosition, // Left of text, Centered, Right
}

pub struct CfDataBarPaint {
    pub fill_color_rgb: [u8; 3],
    pub border_color_rgb: [u8; 3],
    pub fill_fraction: f64,     // 0.0 = min, 1.0 = full cell width
    pub direction: DataBarDirection, // LeftToRight, RightToLeft
}

pub enum CfDecorationPaint {
    Icon(CfIconPaint),
    DataBar(CfDataBarPaint),
    Rating { stars: u8, filled: u8 },  // 3/5 stars filled
}

pub struct CellPaint {
    // ... existing fields: style, text, alignment, ...
    pub cf_decoration: Option<CfDecorationPaint>,
    pub cf_fill_color: Option<[u8; 3]>,  // color scale / dxf fill override
    pub cf_font_color: Option<[u8; 3]>,  // dxf font color override
}
```

**Step 3: Grow the Painter trait**

```rust
// In iron-canvas-core/src/painter/mod.rs:
pub trait Painter {
    // ... existing methods: paint_cell, paint_text, paint_border, ...

    /// Paint a conditional formatting decoration on an already-painted cell rect.
    /// Called after paint_cell (so the data bar layers over the fill, and the icon
    /// sits in the right gutter independently).
    fn paint_cf_decoration(&mut self, rect: Rect, deco: &CfDecorationPaint);
}
```

Backends:
- `CanvasPainter`: draws data bars with `fill_rect`, icons as emoji/SVG glyphs
  in the cell's right gutter
- `SvgPainter`: emits `<rect>` for bars, `<text>` for icons
- `RecorderPainter`: records `DrawOp::CfDecoration { rect, deco }`

**Step 4: Resolve CF in CellPaint builder**

In the `CellPaint` construction pass (currently in `renderer/cell/`), for each
visible cell:

```rust
let extended = model.get_extended_cell_style(sheet, row, col)
    .unwrap_or_default();

let cf_fill_color = extended.style.fill.fg_color
    .and_then(|c| parse_hex_color(&c))
    .or(base_fill);

let cf_decoration = match (&extended.icon, &extended.data_bar) {
    (Some(icon), _) => Some(CfDecorationPaint::from_icon(icon)),
    (_, Some(bar)) => Some(CfDecorationPaint::from_data_bar(bar)),
    _ => None,
};
```

### L5: Interact (UI)

**Side panel** — lists rules for the active sheet (read from `get_conditional_formatting_list`).
Each row shows: range, rule type, format preview, priority.

**Rule editor dialog** — creates/edits a single rule:
- Rule type dropdown (CellIs, ColorScale, DataBar, IconSet, etc.)
- Range picker (uses existing cell selection)
- Format preview (font, fill, border — reuses existing toolbar style picker)
- Formula input (for Formula/Text/CellIs rules)
- Priority up/down buttons

**Interaction flow:**
```
User clicks "Add Rule" → dialog opens → user picks type + range + format
  → try_mutate(Immediate, |m| m.add_conditional_formatting(sheet, range, rule))
  → cf_version signal bumps
  → canvas repaints with new CF
```

## 3. Architecture Decisions

### Don't: Invent a RustyCalc-side CF layer

IronCalc owns: evaluation, cf_cache, priority sorting, stop_if_true, dxf application.
RustyCalc's job is paint-layer work — extend `CellPaint` with a decoration field and
add a painter method. Two sources of truth for CF rules = drift.

### Don't: Bake CF results into the base Style

`data_bar`, `icon`, and `rating` have no home in Excel's `Style` schema. Baking them
into `Style` would require shadowing IronCalc types (violating the "IronCalc is first-class
citizen" rule). IronCalc's `ExtendedStyle` already splits base style from decorations —
consume it directly.

### Do: Follow the decoration overlay pattern

Handsontable's approach: base cell render → user overrides → CF decorations overlay.
The Painter trait method `paint_cf_decoration` is called after `paint_cell`, so the
data bar layers over the fill and the icon sits in the right gutter independently.
This is the same pattern iron-canvas already uses for the selection overlay and
formula-ref decorations.

### Do: Use per-cell resolved style, not a rule→cell index at paint time

LibreOffice uses `ScCondFormatList::GetCondFormatData(cell)` for O(1) lookup.
IronCalc's `cf_cache: HashMap<(sheet,row,col), Vec<CfCellResult>>` is the same
structure. The renderer calls `get_extended_style_for_cell(sheet, row, col)` —
one call, pre-resolved. No rule iteration during paint.

### Do: Bump a cf_version signal for reactivity

The canvas Effect reads `cf_version`. On bump (after add/delete/update/evaluate),
the viewport `CellPaint` slice rebuilds with fresh `cf_decoration` fields. No
fine-grained per-cell invalidation needed — CF rules can affect any cell in their
range, so whole-viewport rebuild is correct and cheap (the slot vecs are reused).

## 4. Implementation Order

1. **CanvasModel extension** — add `get_extended_cell_style` to the trait + all 3 impls
   (UserModel, JsBackedModel, TestModel). Regression test: call the method on a
   cell with/without CF, verify it returns the correct `ExtendedStyle`.

2. **CfDecorationPaint type** — define in `iron-canvas-core/src/types/`.
   Conversion from `ironcalc_base::cf_types::CfIcon/CfDataBar/CfRating`.
   Keep it paint-only: pre-parsed colors, pixel widths, glyph ids.

3. **CellPaint extension** — add `cf_fill_color`, `cf_font_color`, `cf_decoration`
   fields. Resolve CF in the `CellPaint` builder for visible cells.

4. **Painter trait** — add `paint_cf_decoration` method. Implement on `CanvasPainter`
   (data bars as filled rects, icons as text glyphs), `SvgPainter` (SVG rect + text),
   `RecorderPainter` (DrawOp variant).

5. **cf_version signal** — add to `WorkbookState`. Wire into the canvas Effect.
   Bump on CF mutations.

6. **Render integration** — in the `CellPaint` pass, call `get_extended_cell_style`
   for each visible cell. In the paint loop, call `paint_cf_decoration` after
   `paint_cell` for cells with decorations.

7. **UI side panel** — list rules, open editor dialog, wire mutations to IronCalc API.

## 5. How Other Apps Handle This

| App | CF storage | Render-time consumption | Icon/bars |
|---|---|---|---|
| **LibreOffice Calc** | `ScConditionalFormatList` + cell→rule index | `ScOutputData` queries overlay at paint, never mutates base style | ScPatternAttr overlay |
| **OnlyOffice** | Per-range list → per-cell merged cache | Eagerly baked into render cache | Pure canvas |
| **Handsontable** | Plugin hooks into `beforeRenderer` | Renderer chain: base → user → CF | Separate decoration renderer |
| **Apache POI** | OOXML mirror (read-only) | N/A (no renderer) | dxf indirection |

**Steal from:** LibreOffice's cell→rule index (IronCalc's `cf_cache`), Handsontable's
decoration overlay pattern (separate `paint_cf_decoration`).

**Avoid:** OnlyOffice's eager merge into one cache — collapses dxf priority and forces
full rebuild on any rule edit.

## References

- IronCalc CF types: `../IronCalc/base/src/cf_types.rs`
- IronCalc CF engine: `../IronCalc/base/src/conditional_formatting.rs`
- IronCalc UserModel CF API: `../IronCalc/base/src/user_model/conditional_formatting.rs`
- IronCalc xlsx import: `../IronCalc/xlsx/src/import/worksheets.rs`
- IronCalc WASM bindings: `../IronCalc/bindings/wasm/src/lib.rs` (lines 967-1023)
- Developing guide: `docs/guides/developing-excel-features.md`
- Claude skill: `.claude/skills/ironcalc-patterns/references/`
