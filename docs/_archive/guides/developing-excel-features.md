# Developing Excel Features for RustyCalc

Every Excel feature follows a five-layer pipeline. This guide covers the
pipeline, the decision tree for where each feature lives, and the step-by-step
process for adding a new feature from OOXML parse to user interaction.

## 1. The Five-Layer Pipeline

```
xlsx file        [L1] Parse      ironcalc xlsx import (or sidecar)
       │                         OOXML XML → typed Rust structs
       ▼
workbook model   [L2] Store      ironcalc Workbook (engine data)
                  │              + RustyCalc sidecar (presentation data)
       ▼
reactive layer   [L3] React      Leptos signals derived from model on diff
       ▼
canvas/DOM       [L4] Paint      iron-canvas (grid cells)
                  │              + DOM overlays (charts, dropdowns, comments)
       ▼
user input       [L5] Interact   keyboard/mouse → mutate or sidecar API
                                 → emit diff → L3 observes → L4 repaints
```

Each layer is a separate concern. A feature is "supported" only when every
layer handles it. Missing one layer means the feature breaks at that boundary:
parsed but invisible, painted but unclickable, stored but lost on save.

## 2. Decision Tree: Engine or Sidecar?

The first question for any feature: does it affect formula evaluation?

```
Feature affects what formulas compute?
├── YES → belongs in IronCalc engine (upstream PR)
│         - Conditional formatting (rules can reference formulas)
│         - Data validation (rules can reference formulas)
│         - AutoFilter (filters rows; affects COUNT/SUM visible)
│         - Pivot tables (aggregate formulas)
│         - Tables (structured references are formula syntax)
│
└── NO → belongs in RustyCalc sidecar (this repo)
          - Charts (read values from cells, don't write back)
          - Sparklines (display only)
          - Comments (text annotations)
          - Hyperlinks (navigation metadata)
          - Form controls (buttons, checkboxes — UI only)
          - Drawings (images, shapes — layout only)
```

The rule: IronCalc owns the formula/value substrate. Everything that sits on
top of that substrate — rendering, interaction, metadata — is RustyCalc's
territory. Charts don't enter the evaluation graph and shouldn't.

### What "sidecar" means

A sidecar is a separate data structure that:

1. Lives alongside the `UserModel` (not inside it)
2. Survives xlsx round-trips (parsed on import, serialized on export)
3. Diffs alongside IronCalc's model diffs (can reuse undo plumbing)
4. Is accessible to the renderer through a read-only trait

Example pattern:

```rust
pub struct WorkbookSidecar {
    pub comments: CommentStore,
    pub charts: ChartStore,
    pub drawings: DrawingStore,
    pub data_validations: ValidationStore,
}

impl WorkbookSidecar {
    pub fn from_xlsx(archive: &ZipArchive) -> Self { /* ... */ }
    pub fn to_xlsx(&self, archive: &mut ZipWriter) { /* ... */ }

    // Read-only access for the renderer
    pub fn comments_at(&self, sheet: u32, row: i32, col: i32) -> Option<&Comment> { /* ... */ }
    pub fn chart_at(&self, sheet: u32, anchor: &Anchor) -> Option<&Chart> { /* ... */ }
}
```

## 3. Per-Feature Layer Mapping

| Feature | L1 Part | L2 Home | L4 Home | L5 Entry |
|---|---|---|---|---|
| Data validation | `dataValidations` in `sheet*.xml` | Engine + sidecar | DOM `<select>` over canvas cell rect | Cell-edit interceptor |
| Conditional formatting | `conditionalFormatting` in `sheet*.xml` | Engine (drives cell style) | iron-canvas CellPaint pass | Ribbon dialog → `try_mutate` |
| Charts | `xl/charts/chart*.xml` + `xl/drawings/drawing*.xml` | Sidecar | Separate `<canvas>` or SVG layer above iron-canvas | DOM hit-test + right-click |
| Sparklines | `extLst/x14:sparklineGroups` in sheet | Sidecar | iron-canvas inline (CellPaint extension) | Ribbon |
| AutoFilter | `autoFilter` element in `sheet*.xml` | Engine (row visibility) | iron-canvas filter glyph in header | Header click |
| Hyperlinks | `hyperlinks` element + `_rels/*.rels` | Sidecar | iron-canvas glyph + cursor change | Ctrl+click |
| Comments | `xl/comments*.xml` + `xl/threadedComments/` | Sidecar | DOM bubble + iron-canvas red-corner glyph | Right-click |
| Form controls | `xl/drawings/vmlDrawing*.vml` | Sidecar | DOM `<input>` / `<button>` layer | DOM events |
| Pivot tables | `xl/pivotTables/`, `xl/pivotCache/` | Engine (large extension) | iron-canvas (cells) + DOM filter chrome | Field-list panel |
| Tables | `xl/tables/table*.xml` | Engine (structured refs) | iron-canvas (header formatting + autofilter glyphs) | Ribbon |
| Drawings/images | `xl/drawings/drawing*.xml` + `xl/media/` | Sidecar | Separate `<img>` or `<canvas>` layer | Click + drag resize |

## 4. OOXML Survival Kit

### The minimum XML format for each feature

Every feature has a minimum set of elements needed to survive a round-trip.
Start with the simplest valid XML for each:

**Data validation (SpreadsheetML §18.3.1.32):**
```xml
<dataValidations count="1">
  <dataValidation type="list" allowBlank="1" showDropDown="0"
                  sqref="A1:A10">
    <formula1>"Item 1,Item 2,Item 3"</formula1>
  </dataValidation>
</dataValidations>
```

> `showDropDown="0"` means SHOW the dropdown. Yes, it's inverted.
> Fix at parse boundary so the rest of the system never sees the XML convention.

**Conditional formatting (SpreadsheetML §18.3.1.18):**
```xml
<conditionalFormatting sqref="A1:A10">
  <cfRule type="cellIs" operator="greaterThan" dxfId="0" priority="1">
    <formula>5</formula>
  </cfRule>
</conditionalFormatting>
```

**Hyperlink (SpreadsheetML §18.3.1.47):**
```xml
<hyperlinks>
  <hyperlink ref="A1" r:id="rId1" display="Click here"/>
</hyperlinks>
```
The relationship `rId1` points to the target URL in `xl/worksheets/_rels/sheet1.xml.rels`.

**Comment (SpreadsheetML §18.7):**
```xml
<!-- xl/comments1.xml -->
<comments>
  <authors><author>User</author></authors>
  <commentList>
    <comment ref="A1" authorId="0">
      <text><r><t>Hello</t></r></text>
    </comment>
  </commentList>
</comments>
```

### Part paths and relationship wiring

OOXML is a ZIP of XML files connected by `.rels` files:

```
archive.xlsx
├── [Content_Types].xml     ← "what content types exist?"
├── _rels/.rels             ← "where's the workbook?"
├── xl/workbook.xml         ← "what sheets exist?"
├── xl/_rels/workbook.xml.rels  ← "where are the sheet files?"
├── xl/worksheets/sheet1.xml    ← "the actual data"
├── xl/worksheets/_rels/sheet1.xml.rels  ← "what's linked to this sheet?"
│       ├── rId1 → ../comments1.xml       (comments)
│       ├── rId2 → ../tables/table1.xml   (tables)
│       └── rId3 → ../drawings/vmlDrawing1.vml  (form controls)
├── xl/comments1.xml
├── xl/drawings/drawing1.xml  ← "what shapes/charts are on sheet1?"
├── xl/charts/chart1.xml     ← "chart definition (series, axes, title)"
└── xl/tables/table1.xml     ← "structured table definition"
```

To add a feature to the import path:
1. Parse `[Content_Types].xml` → find the part name
2. Parse `xl/_rels/workbook.xml.rels` → find the relationship
3. Parse `xl/worksheets/_rels/sheet1.xml.rels` → find sheet-scoped relationships
4. Parse the feature XML → your typed structs
5. Serialize in reverse on export

### Key OOXML references

- **ECMA-376** (free download, ~5000 pages) — Part 1 §18 covers SpreadsheetML.
  The relevant sections:
  - §18.3.1 — worksheet elements (dataValidations, conditionalFormatting, autoFilter, hyperlinks, sparklineGroups via extLst)
  - §18.7 — comments / threadedComments
  - §18.10 — tables
  - §19 — DrawingML (charts live here via `chartSpace`)
  - §17.16 — form controls (shared with WordprocessingML)
- **Open XML SDK docs** (Microsoft) — schema diagrams, easier than ECMA prose
  https://learn.microsoft.com/en-us/office/open-xml/open-xml-sdk
- **Existing ironcalc xlsx import** at `../IronCalc/xlsx/src/import/` —
  shows what's already parsed (good gap inventory)
- **`mc:AlternateContent`** pattern — XML markup compatibility.
  `<mc:AlternateContent><mc:Choice Requires="x14">...<mc:Fallback>...`
  Used for features introduced in Excel 2010+ (sparklines, threaded comments)

## 5. DOM-over-Canvas Pattern

iron-canvas paints grid cells. For features that need interactive HTML elements
(dropdowns, comment bubbles, chart controls), use DOM elements positioned
ON TOP of the canvas:

```
┌─────────────────────────────────┐
│  <canvas id="iron-canvas">      │  ← iron-canvas grid (cells, selection)
│    (2D context, 60fps paint)    │
├─────────────────────────────────┤
│  <div id="overlays">            │  ← absolutely positioned, transparent bg
│    <select>   (validation dropdown)  │     coordinate-aligned via cellRect
│    <div>      (comment bubble)       │
│    <canvas>   (chart rendering)      │
│    <a>        (hyperlink cursor)     │
│  </div>                               │
└─────────────────────────────────┘
```

Coordinate alignment: `iron-canvas` already exposes `cellRect(row, col): { x, y, w, h }`
through the WASM bridge. Use this to position DOM overlays at exact cell bounds.
On scroll/zoom, reposition overlays in the rAF loop alongside the canvas paint.

When to use DOM vs canvas:
- **DOM:** interactive controls (dropdowns, inputs, buttons), rich text (comments),
  complex layouts (pivot field list), accessibility (screen readers need real DOM)
- **Canvas:** grid cells, selection, formula-ref overlays, sparklines,
  conditional formatting fills — anything that changes per-frame and benefits from
  batched 2D drawing

## 6. Round-Trip Checklist

The minimum acceptance bar for any feature. Before claiming a feature is done,
verify:

1. **Import:** open a real `.xlsx` with the feature → the feature appears on screen.
   Use Excel Online or LibreOffice to create test files.
2. **Edit:** mutate the feature through RustyCalc's UI → the change is visible
   immediately (L3→L4) and doesn't crash.
3. **Export:** save → re-open in Excel Online → the feature is intact and editable.
4. **Undo:** mutate → undo → feature reverts to pre-mutation state.
5. **Round-trip identity:** save without changes → diff input .xlsx vs output .xlsx →
   differences are only in non-semantic XML (whitespace, namespace prefix choice,
   element ordering within unordered containers).
6. **Round-trip no-op on unsupported:** open a file with a feature RustyCalc
   doesn't support yet → save → the unsupported feature is still in the file
   (not silently dropped). Use `mc:AlternateContent` or opaque byte preservation.

## 7. Worked Example: Data Validation (List Dropdown)

The simplest end-to-end feature. Walk through all five layers:

### L1: Parse

```rust
// In ironcalc xlsx import (or RustyCalc sidecar):
fn parse_data_validations(reader: &XmlReader) -> Vec<DataValidation> {
    // Parse <dataValidations> → Vec of <dataValidation>
    // For each: type, allowBlank, showDropDown (!inverted), sqref, formula1
}
```

### L2: Store

```rust
// In WorkbookSidecar:
pub struct ValidationStore {
    validations: HashMap<u32, Vec<DataValidation>>,  // sheet → validations
}

impl ValidationStore {
    pub fn for_cell(&self, sheet: u32, row: i32, col: i32) -> Option<&DataValidation> {
        self.validations.get(&sheet)?
            .iter()
            .find(|dv| dv.applies_to(row, col))
    }
}
```

### L3: React

```rust
// In the Leptos component that handles cell editing:
let validation = model.with_value(|m| {
    sidecar.validations.for_cell(m.get_selected_sheet(), row, col)
});

// If validation is Some(type="list"), show dropdown on Enter/F2
```

### L4: Paint

```rust
// In the cell editor component (DOM, not canvas):
view! {
    <Show when=move || validation.is_some()>
        <select on:change=move |ev| {
            let value = event_target_value(&ev);
            // Commit the selected value
        }>
            <For each=move || validation.unwrap().list_items()
                 key=|item| item.clone()
                 children=move |item| view! { <option>{item}</option> }
            />
        </select>
    </Show>
}
```

Position the `<select>` using `cellRect(row, col)` from the iron-canvas bridge,
absolutely positioned over the cell rectangle.

### L5: Interact

The dropdown appears when the user enters edit mode (Enter/F2) on a validated cell.
Selecting an item commits the value via `try_mutate(Immediate, ...)`. The dropdown
closes. The grid repaints with the new value.

Keyboard: Escape cancels (close dropdown, no commit). Tab/Enter commits the
current selection and moves to the next cell.

## 8. Per-Feature Appendix (Stubs)

These are cross-linked to the skill references. Each gets a short page when
the feature is actively developed.

### Data Validation
- OOXML: `<dataValidations>` in sheet XML (SpreadsheetML §18.3.1.32)
- Types: list, whole, decimal, date, time, textLength, custom
- Inverted convention: `showDropDown="0"` → show dropdown
- Interaction: dropdown on cell-edit, error alert on invalid input

### Conditional Formatting
- OOXML: `<conditionalFormatting>` + `<cfRule>` (SpreadsheetML §18.3.1.18)
- Types: cellIs, expression, colorScale, dataBar, iconSet, top10, uniqueValues
- Paint: iron-canvas `CellPaint` pass reads `Style` derived from active rules
- Interaction: Ribbon dialog → `try_mutate(Immediate)` on rule change

### Charts
- OOXML: `xl/charts/chart*.xml` + DrawingML anchor (Part 1 §19, §21)
- Store: sidecar (separate model, not in engine)
- Paint: separate `<canvas>` element above iron-canvas grid
- Interaction: hit-test via anchor position, resize handles, right-click context menu

### AutoFilter
- OOXML: `<autoFilter>` element in sheet (SpreadsheetML §18.3.1.2)
- Store: engine (affects row visibility)
- Paint: filter dropdown glyph in column headers (iron-canvas header strip)
- Interaction: click header → show filter dropdown (DOM, positioned over header cell)
- Coordinate trap: `colId` in autoFilter is relative to `autoFilter@ref`'s first
  column, NOT worksheet column A

### Hyperlinks
- OOXML: `<hyperlinks>` + `_rels` (SpreadsheetML §18.3.1.47)
- Store: sidecar
- Paint: underline text style + cursor change on hover (iron-canvas CellPaint)
- Interaction: Ctrl+click to follow (browser security: must be Ctrl+click)

### Comments
- OOXML: `xl/comments*.xml` (SpreadsheetML §18.7.1.4) + VML shape for position
- Store: sidecar
- Paint: red triangle glyph in cell corner (iron-canvas CellPaint) +
  DOM comment bubble on hover/click
- Interaction: right-click → insert/edit/delete comment

### Sparklines
- OOXML: `<x14:sparklineGroups>` in `extLst` (SpreadsheetML §18.3.1.92)
- Store: sidecar
- Paint: inline mini-chart in cell background (iron-canvas CellPaint pass)
- Interaction: Ribbon to create, click to select group, right-click to edit range

## 9. Skill Structure

The Claude skill for feature development (`.claude/skills/excel-features/`):

```
excel-features/
├── SKILL.md                    ← decision tree + pipeline overview
└── references/
    ├── pipeline.md             ← 5-layer pipeline detail
    ├── ooxml-anatomy.md        ← parts, rels, mc:AlternateContent, extLst
    ├── sidecar-store.md        ← sidecar pattern, diff integration
    ├── dom-over-canvas.md      ← coordinate alignment, when DOM vs canvas
    ├── round-trip.md           ← acceptance checklist
    ├── data-validation.md      ← per-feature deep dives
    ├── conditional-formatting.md
    ├── charts.md
    ├── autofilter.md
    ├── hyperlinks.md
    ├── comments.md
    ├── sparklines.md
    ├── form-controls.md
    └── pivot-tables.md
```

One umbrella skill with per-feature references. Don't split into separate
skills — they share the pipeline; duplication risk > divergence risk.

## References

- ECMA-376: https://www.ecma-international.org/publications-and-standards/standards/ecma-376/
- Open XML SDK: https://learn.microsoft.com/en-us/office/open-xml/open-xml-sdk
- IronCalc xlsx import: `../IronCalc/xlsx/src/import/`
- IronCalc OOXML crate plan: `.claude/skills/ironcalc-patterns/references/`
- iron-canvas AGENTS.md: `iron-canvas/AGENTS.md`
