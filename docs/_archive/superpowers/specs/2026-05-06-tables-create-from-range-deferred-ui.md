# Tables — deferred UI decisions

**Status:** Brainstorming notes for a *future* design pass. Captured as a
sibling to `2026-05-06-tables-create-from-range-design.md` so the v1
spec stays focused on shipping the renderer test rig.
**Trigger to reopen:** when the v1 test rig has shaken out the renderer
work and the next question becomes "how should real users customize a
table's appearance?"
**Recommended skill for the reopen:** `superpowers:brainstorming` plus
the `frontend-design` skill (and the visual companion if you want to
prototype the picker in a browser).

---

## Why these are deferred

V1 ships a **fixed pastel-blue palette, no banding, no picker**. That's
the smallest surface that lets the renderer be tested. But every "no
picker / no toggle / one preset" choice in v1 leaves a real UX question
on the table for later. This doc collects them in one place so the next
design pass doesn't have to re-discover them.

Each deferred decision below records:
- **The question** — what UX are we choosing between?
- **Options** — the candidate shapes (with rough trade-offs)
- **Constraints inherited from v1** — anything in the v1 spec that
  pre-commits the answer or rules options out

---

## D1. Style picker shape

**Question:** When a user inserts a table, how do they pick its visual
style?

### Options

- **A. No picker.** Always pastel blue. *(v1 ships this.)* Zero UX
  burden, zero customization.
- **B. Preset dropdown.** A handful of named presets: `Pastel Blue`,
  `Pastel Green`, `Pastel Gray`, maybe `High-Contrast`. Each preset is
  a hardcoded palette + border rule. Excel-style "table styles gallery"
  but minimal.
- **C. Excel-parity gallery.** Visual thumbnails of ~60 named styles
  (`TableStyleLight1..21`, `Medium1..28`, `Dark1..11`) — what
  `Table_Styling_Spec.md` already enumerates. Implies the layered
  styling pipeline (Tasks 1-8 of the styling spec) is complete.
- **D. Per-element color picker.** No presets — instead, expose the
  primitives directly: header bg color, body bg color, border color,
  band-A bg, band-B bg, font weight toggles. User builds the look they
  want. *(This is what the user gestured at: "more options via
  colorpicker".)*

### Constraints inherited from v1

- The `apply_pastel_blue` helper in `table_insert.rs` already writes
  concrete `Style` values to cells. Whatever picker we choose, it will
  drive the *same* helper — just with different inputs. The helper
  stays; the inputs become parameters.
- No `Table.style_info.name` is set in v1 (left at `None`). If we go
  with C, we need to start populating it so the layered styling
  pipeline can resolve.
- `feedback_no_css_deletions` — anything visual must be done via CSS
  *substitution* and additions, not deletions.

### Trade-offs at a glance

| | A (none) | B (presets) | C (Excel gallery) | D (color picker) |
|--|---|---|---|---|
| Implementation | Done | Days | Weeks (depends on layering pipeline) | Days |
| Discoverability for users | N/A | High | High | Medium (more cognitive load) |
| Visual variety | None | Low | High | Unbounded |
| Couples to layered renderer? | No | No | **Yes** | No |
| "Test multiple tables on one sheet" use case | Hard (same color) | Easy | Easy | Easy |

---

## D2. Banded rows / banded columns

**Question:** Should body rows alternate colors? Should body columns?

### Options

- **A. No banding.** Every body cell gets the same fill. *(v1 ships
  this.)*
- **B. Always banded.** Hardcoded alternating rows (`#DDEBF7` /
  `#FFFFFF` or `#DDEBF7` / `#BFD7EE`). Implies one extra cell write per
  row in `apply_pastel_blue`.
- **C. Banding toggles in the form.** Two checkboxes: ☐ Show row
  stripes, ☐ Show column stripes. Map to `Table.style_info.show_row_stripes`
  / `show_column_stripes`.
  - In a "direct cell write" model (v1's approach), the toggles drive
    which cells get which fill — the visual is baked into the cell
    `Style`.
  - In a "layered styling" model (`Table_Styling_Spec.md`), the toggles
    only need to set the `style_info` flags; the renderer paints them.
- **D. Banding is a property of the chosen style preset (D1.B/C/D).**
  No separate toggle — `Pastel Blue Banded` and `Pastel Blue Solid` are
  two different presets in the gallery.

### Constraints inherited from v1

- v1 hardcodes `style_info` to `TableStyleInfo::default()` — all
  toggles `false`. Either D1 or D2 will need to start setting these
  fields.
- The `Table_Styling_Spec.md` already has `show_row_stripes` /
  `show_column_stripes` as part of its rendering pipeline. Choosing D2.C
  with the layered model is "free" once the pipeline lands.

---

## D3. First / last column emphasis

**Question:** Should the leftmost / rightmost data column get extra
visual weight (bold, accent fill)?

### Options

- **A. No emphasis.** *(v1 ships this — `style_info.show_first_column`
  and `show_last_column` both `false`.)*
- **B. Toggles in the form.** Two checkboxes; behave like D2.C —
  either bake into cell styles or flip `style_info` flags depending on
  rendering model.
- **C. Inferred from style preset.** Some presets set them, some don't.
  Pairs with D1.B/C/D.

### Constraints inherited from v1

- Same as D2 — `style_info` is `default()` today.

---

## D4. Custom palette persistence

**Question:** If we ship D1.D (per-element color picker), where do
custom palettes live?

### Options

- **A. Stateless — picker resets each time the modal opens.** Simplest;
  user must reconfigure for every new table. Painful if iterating.
- **B. Per-session (signal-only).** `state` holds the last-used
  palette; modal opens prefilled with it. Lost on page reload.
- **C. Per-workbook.** Stored in the workbook somehow (a
  `WorkbookSetting` extension?). Survives Save / Load. Complicated by
  the fact that `WorkbookSettings` upstream is `{ tz, locale }` — no
  obvious extension point.
- **D. Per-user (LocalStorage).** Favourite palettes stored in browser
  LocalStorage. Survives reload, doesn't pollute the workbook file.

### Constraints inherited from v1

- v1 doesn't touch palette state at all — picker doesn't exist.
- LocalStorage / settings storage exists in the codebase
  (`src/storage.rs`) and could be extended.

---

## D5. Modal vs. inline-on-canvas creation

**Question:** Is a modal the right surface at all? Excel and Google
Sheets both use the modal pattern, but a "convert selection to table
inline, with a popover toolbar for tweaks" is also viable.

### Options

- **A. Modal.** *(v1 ships this.)* Discoverable, uniform, follows
  `named_ranges` precedent.
- **B. Inline conversion + floating styling toolbar.** `Ctrl+M`
  immediately creates a default table; a small floating toolbar
  appears anchored to the table's top-right with style/banding/headers
  controls. Faster for power users; harder to discover.
- **C. Hybrid.** Modal for create (parameters that change behavior:
  has-headers, totals); floating toolbar for cosmetic tweaks after.

### Constraints inherited from v1

- Modal infrastructure is already in place and proven (`named_ranges`
  uses it).
- No floating-toolbar pattern exists in the codebase yet — would be
  net-new infrastructure.

---

## D6. Edit / delete / rename UX (out of v1 scope)

**Question:** When v2 adds CRUD, where does each operation live?

### Options for delete

- **A. Right-click on table cell → "Delete table".** Adds a context
  menu entry to `context_menu.rs`.
- **B. Reopen `Ctrl+M` on a cell that's inside a table → modal becomes
  mode-aware ("Edit / Delete" buttons).** Single keyboard surface; modal
  has two modes.
- **C. Toolbar button in a Tables pane.** Implies a left-drawer
  Tables list (parallel to a future Defined Names list).

### Options for rename

- **A. Inline in the modal (B above).**
- **B. Dedicated Rename input in a list view (C above).**
- **C. Status-bar prompt** (mirrors how some sheet-rename UIs work).

### Options for "change style of existing table"

- Tightly coupled to D1 — whatever picker shape we choose for create,
  reuse it for edit.

---

## How to use this doc

When the next design pass opens, `superpowers:brainstorming` should:

1. Read this doc to surface the open questions.
2. Pick **one decision at a time** (D1 first — most foundational), use
   the visual companion if the option involves visual choices.
3. Cross off each decision in this file as it's resolved (or move it
   into the v2 spec doc and delete the entry here).

The goal is not to answer everything — it's to make sure the next
brainstorm starts with a complete map of the territory, not a blank
page.
