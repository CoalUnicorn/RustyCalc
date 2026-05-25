# Tables — create from selected range (v1, renderer test rig)

**Status:** **Blocked on upstream `UserModel::insert_table`.** Brainstormed and
approved as a v1 test rig, but implementation discovered that
`UserModel.model` is `pub(crate)` (IronCalc/`base/src/user_model/common.rs:259`),
so the spec's "direct mutate `workbook.tables`" path does not compile from
outside the crate. Spec is parked; see **Unblock criteria** below for the
one-line trigger that lets it become implementable. The decisions, palette,
validation rules, and test list are still correct — they're waiting for one
upstream API to exist.
**Scope:** RustyCalc only. UI = `Ctrl+M` opens a small modal that turns the
current selection into a `Table` record in `workbook.tables`, applies a
fixed pastel-blue formatting to the cells, and exposes the table to the
renderer.
**Out of scope:** XLSX round-trip persistence (no `xlsx/src/export/tables.rs`
upstream); user-customizable styling (presets, color pickers, banding) — see
sibling deferred-UI doc; full `Table_Styling_Spec.md` rendering pipeline
(layered named-style resolution); table edit / delete / rename.

---

## One-line purpose

When the user presses `Ctrl+M` with a non-trivial selection, RustyCalc opens
a small dialog that — on submit — applies pastel-blue formatting to the
selected cells and registers a `Table` record in `workbook.tables`, so that
structured-reference formulas (`Table1[Column1]`) parse and the visual
result is visible in iron-canvas without any new renderer plumbing.

---

## Background

`/home/mmm/01_Dev/IronCalc/base/src/types.rs:240` defines a fully public
`Table` struct (and `TableColumn`, `TableStyleInfo`), and
`Workbook.tables: HashMap<String, Table>` at line 52 is `pub`. **However:**

- No `pub fn create_table` / `insert_table` / `add_table` exists anywhere
  in `UserModel` or `Model` (verified by exhaustive `pub fn ... table` grep
  in `/home/mmm/01_Dev/IronCalc/base/src/`).
- `user_model/history.rs` exposes `pub enum DiffType { Undo, ... }` — there
  are **no** `TableAdded` / `TableRemoved` / `TableEdited` variants.
- `xlsx/src/import/tables.rs` exists, but `xlsx/src/export/` does **not**
  contain a tables file. Tables imported from XLSX render but cannot
  round-trip on Save-As.

This means a "real" implementation has to land an upstream patch on the
user's IronCalc fork (`CoalUnicorn/IronCalc`, `branch: fix-col-iterations`)
adding `UserModel::insert_table(...)` and matching `Diff` variants.

The user has explicitly chosen to **defer** that upstream work and ship a
test-rig MVP that mutates `workbook.tables` directly. The hack is contained
to **one method** in `frontend_model.rs` so the migration to a proper API
is a one-line change later.

---

## Decisions (the brainstorm record)

| # | Decision | Choice |
|---|----------|--------|
| 1 | IronCalc API path | **B** — direct mutate `workbook.tables`; defer upstream patch |
| 2 | UX shape | **B** — small modal, exposes the levers the renderer cares about |
| 3 | Trigger | **A** — keyboard shortcut only (no toolbar button) |
| 4 | CRUD scope (v1) | **A** — create only |
| 5 | Header behavior | **C** — Excel parity: ☑ reads row 1, ☐ inserts blank `Column1..N` row above and shifts data down |
| — | Shortcut key | **`Ctrl+M`** (avoids browser `Ctrl+T`/`Ctrl+L` clashes; layout-independent) |
| — | Modal backdrop | Transparent (CSS substitution) — lets the user keep the table visible while iterating |
| — | Re-render trigger | **None added** — default formatting writes go through `set_cell_style`, which already emits format-damage that the renderer reacts to |
| — | Error surface | **Existing status bar** (`StatusMessage::Error`); no in-form banner. Success closes the modal silently — no `Info` variant added (matches `named_ranges/form.rs` convention) |
| — | Visual styling | **Hardcoded pastel-blue palette** (no picker in v1; deferred to sibling UI doc) |

---

## Architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│                          USER PRESSES CTRL+M                           │
└────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│  src/input/keyboard.rs::classify_key                                   │
│    ("m", KeyMod::ctrl()) → SpreadsheetAction::Structure(               │
│        StructAction::ToggleTableModal)                                 │
└────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│  src/state.rs                                                          │
│    pub(crate) tables_modal_open: Split<bool>                           │
│    (mirrors named_ranges_modal_open exactly)                           │
└────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│  src/components/insert_table/                                          │
│    mod.rs            — <InsertTableDialog/>  (mounted via <Show>)      │
│    form.rs           — <InsertTableForm/>    (the actual fields)       │
│  Reuses src/components/modal.rs::Modal<Small>                          │
└────────────────────────────────────────────────────────────────────────┘
                                    │  on submit
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│  src/model/frontend_model.rs                                           │
│    pub fn insert_table_from_selection(                                 │
│        &mut self, request: InsertTableRequest,                         │
│    ) -> Result<String, TableInsertError>                               │
│                                                                        │
│    1. read selection via get_selected_view()                           │
│    2. validate_request(...) + ensure_no_overlap(...)                   │
│       + ensure_unique_name(...) (case-insensitive)                     │
│    3. if !request.has_headers:                                         │
│         self.user_model.insert_rows(sheet, r1, 1)?  ← real Diff        │
│         range = range with r2 += 1                                     │
│    4. build_columns(...) — read row 1 (or auto-generate)               │
│    5. apply_pastel_blue(sheet, range, header_rows, totals_rows)?       │
│       ← real Diff entries via set_cell_style                           │
│    6. self.user_model.workbook.tables.insert(name, table)              │
│       // TODO(upstream): replace with UserModel::insert_table          │
│    7. Ok(name)                                                         │
└────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│  Renderer (iron-canvas) — no new plumbing                              │
│    The set_cell_style writes in step 5 emit normal format-damage       │
│    events; the existing renderer pipeline picks them up on the next    │
│    frame. The Table record sits in workbook.tables waiting for the     │
│    full styling pipeline (covered by Table_Styling_Spec.md).           │
└────────────────────────────────────────────────────────────────────────┘
```

---

## File-level changes

| Module | Status | Purpose |
|--------|--------|---------|
| `src/components/insert_table/mod.rs` | NEW | `<InsertTableDialog/>` — wraps `Modal<Small>`, owns close-flow |
| `src/components/insert_table/form.rs` | NEW | `<InsertTableForm/>` — the actual fields, submit handler |
| `src/model/table_insert.rs` | NEW | `InsertTableRequest` struct, `TableInsertError` enum, pure helpers (`next_table_name`, `validate_request`, `ensure_no_overlap`, `ensure_unique_name`, `build_columns`, `format_a1_range`) |
| `src/state.rs` | EXTEND | Add `tables_modal_open: Split<bool>` next to `named_ranges_modal_open` |
| `src/input/keyboard.rs` | EXTEND | Add `("m", KeyMod::ctrl())` arm in the Ctrl combo block; suppress in editing mode (consistent with existing Ctrl+c/v/z/a suppression) |
| `src/input/structure.rs` | EXTEND | Add `StructAction::ToggleTableModal` variant |
| `src/model/frontend_model.rs` | EXTEND | Add `insert_table_from_selection(req) -> Result<String, TableInsertError>` method (the *only* place that touches `workbook.tables` directly) |
| `src/components/workbook.rs` | EXTEND | Mount `<InsertTableDialog/>` next to `<n />` (named-ranges dialog) |
| `styles/insert_table.css` | NEW | Form layout (label/input grid, dropdown styling) |
| `styles/modal.css` | EDIT | One-line substitution: `background: rgba(0, 0, 0, 0.45)` → `background: transparent` (user owns the actual CSS edit per `feedback_no_css_deletions`) |

---

## Component shapes

### Form / Dialog

```rust
// src/components/insert_table/mod.rs
#[component]
pub fn InsertTableDialog() -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState provided");
    let is_open = state.tables_modal_open.read();
    let on_close = Callback::new(move |_| {
        state.tables_modal_open.write().set(false);
        state.refocus_formula_input();
    });

    view! {
        <Show when=move || is_open.get()>
            <Modal title="Insert Table" on_close=on_close size=ModalSize::Small>
                <InsertTableForm on_close=on_close />
            </Modal>
        </Show>
    }
}

// src/components/insert_table/form.rs
#[component]
pub fn InsertTableForm(on_close: Callback<()>) -> impl IntoView {
    // signals (snapshotted at mount): name, has_headers, header_rows,
    // totals_rows, range_preview
    // submit: build InsertTableRequest, call model.update(insert_table_from_selection)
    // on Ok:  on_close.run(())  (silent — matches named_ranges precedent)
    // on Err: status -> Error(e.to_string());  modal stays open
}
```

### Request / Error contract

```rust
// src/model/table_insert.rs

pub struct InsertTableRequest {
    pub name: String,           // empty → auto-name via next_table_name
    pub has_headers: bool,      // ☑ checkbox, default true
    pub header_row_count: u32,  // form input, default 1
    pub totals_row_count: u32,  // form input, default 0
}

pub enum TableInsertError {
    EmptySelection,
    WholeSheetSelection,
    NoDataRows,
    HeaderRowsExceedSelection { rows_in_selection: u32, header_rows: u32 },
    TotalsRowsExceedSelection { rows_in_selection: u32, header_rows: u32, totals_rows: u32 },
    OverlapsExistingTable { name: String },
    NameAlreadyTaken { name: String },
    InvalidName { reason: &'static str },
    InsertRowsFailed(String),     // wraps IronCalc's insert_rows error
    ApplyStyleFailed(String),     // wraps IronCalc's set_cell_style error
}

impl Display for TableInsertError {
    // single-line, status-bar-friendly messages.
    // Each variant carries the data needed for a useful message; no
    // flat strings, no `_` arms (per feedback_ironcalc_first_class).
    // Example: HeaderRowsExceedSelection { rows_in_selection: 2, header_rows: 3 }
    //   → "Header rows (3) exceeds selection size (2)"
}
```

---

## Validation rules

| Check | Rule | Variant |
|-------|------|---------|
| Range size | `r2 ≥ r1 && c2 ≥ c1` | `EmptySelection` |
| Whole-sheet selection | reject if `r2 == LAST_ROW \|\| c2 == LAST_COLUMN` | `WholeSheetSelection` |
| Min height with headers | if `has_headers && (r2 - r1 + 1) < 2` | `NoDataRows` |
| Header rows fit | `header_row_count ≤ (r2 - r1 + 1)` | `HeaderRowsExceedSelection` |
| Header + totals fit | `header_row_count + totals_row_count ≤ (r2 - r1 + 1)` | `TotalsRowsExceedSelection` |
| Overlap with existing table | walk `workbook.tables` on this sheet, check range intersection | `OverlapsExistingTable { name }` |
| Name uniqueness | case-insensitive (Excel parity) | `NameAlreadyTaken { name }` |
| Name format | non-empty after auto-fill, no spaces, starts with letter or `_` | `InvalidName { reason }` |

**Selection is snapshotted at modal open**, not re-read at submit. Reasons:
the user's mental model is "I selected this, then opened the dialog";
background selection changes shouldn't surprise them; the range preview
text would otherwise lie.

**No client-side disable on the Insert button.** Validation lives in **one**
place — the model — and the model owns the error message that ends up in
the status bar. Disabled buttons that don't tell you *why* they're
disabled are an anti-pattern; this design replaces them with a tight
"click → status-bar-feedback" loop.

---

## Default formatting (pastel blue, hardcoded)

Applied directly to cells via `set_cell_style` (real IronCalc Diff
entries), which means the renderer's existing format-damage detection
handles repaints with zero new event plumbing.

| Element | Hex | Notes |
|---------|-----|-------|
| Header background | `#9DC3E6` | Excel "Blue, Accent 1, Lighter 40%" |
| Header text | `#1F3864` | Dark navy; ~7:1 contrast on `#9DC3E6` |
| Header font weight | bold | |
| Body background | `#DDEBF7` | Excel "Blue, Accent 1, Lighter 80%" |
| Body text | (cell default) | Inherits `--text-color` |
| Border color | `#5B9BD5` | Excel "Blue, Accent 1" |
| Cell grid (inside table) | 1 px solid `#5B9BD5` | All four edges of every cell |
| Header bottom border | 2 px solid `#5B9BD5` | Emphasises header/data boundary |
| Outer table border | 1 px solid `#5B9BD5` | Quiet outline, same color as grid |

**Banded rows: NO** in v1. Every body row uses `#DDEBF7`. (Banding,
multi-preset picker, and per-element color customization are deferred —
see sibling `2026-05-06-tables-create-from-range-deferred-ui.md`.)

---

## UI lifecycle

```
state.tables_modal_open.set(false)             ← initial
                ▲                              │
       Esc /    │                              │ Ctrl+M action
   Cancel /     │                              ▼
   submit OK    │                  state.tables_modal_open.set(true)
                │                              │
                │                              ▼
                │                ┌─ <Show when=is_open> ─────────┐
                │                │ <InsertTableDialog/>          │
                │                │   takes selection snapshot    │
                │                │   focuses Name input on mount │
                │                └───────────────────────────────┘
                │                              │
                │           on:submit          ▼
                │              ┌─ insert_table_from_selection ──┐
                │              │  Ok(name) → close modal        │
                │              │             (silent success)   │
                │              │  Err(e)   → status: Error(...) │
                │              │             keep modal open    │
                │              └────────────────────────────────┘
                ▼
       (modal unmounts, focus returns to grid)
```

**Form layout:**

```
┌─ Insert Table ──────────────────────────── ✕ ┐
│                                              │
│   Range:    A1:D6   (Sheet1)                 │
│                                              │
│   Name:     [ Table1                ]        │
│                                              │
│   ☑ My table has headers                     │
│                                              │
│   Header rows:  [ 1 ▾ ]  Totals: [ 0 ▾ ]    │
│                                              │
│            [ Cancel ]  [   Insert   ]        │
└──────────────────────────────────────────────┘
```

`size = ModalSize::Small` (360 px). Defaults: name = `next_table_name(...)`,
has_headers = ☑, header_rows = 1, totals_rows = 0. Auto-focus the Name
input on mount; Esc closes; Enter in any input submits.

---

## Testing strategy

### Pure unit tests (`src/model/table_insert.rs`)

| Target | Cases |
|--------|-------|
| `next_table_name` | `[]` → `"Table1"`; `["Table1"]` → `"Table2"`; `["Table1","Table3"]` → `"Table2"` (fills gap); `["Table1","Table10","Table2"]` → `"Table3"` (numeric, not lex) |
| `validate_request` | each `TableInsertError` variant has a triggering case; valid case returns `Ok(())` |
| `ensure_no_overlap` | disjoint ok; touching-not-overlapping (A1:B2 vs C1:D2) ok; corner overlap → reject; one-inside-other → reject |
| `ensure_unique_name` | case-insensitive: existing `"Sales"`, attempt `"sales"` → reject |
| `build_columns_from_header_row` | empty cells fall back to `Column1..N`; duplicate names get `_2`/`_3` suffix; mixed case duplicates collapse |
| `format_a1_range` | round-trip with parser; handful of hand-picked cases |

### Integration tests (`src/model/frontend_model.rs::tests`)

Following the existing `m.set_user_input(...)` pattern (`frontend_model.rs:563+`):

| Test | Asserts |
|------|---------|
| `inserts_table_with_headers` | A1:D6 with header text in row 1 → `workbook.tables["Table1"]` exists, `reference == "A1:D6"`, `columns[0].name` matches header text. Row contents unchanged. |
| `inserts_blank_header_row_when_unchecked` | A1:D6 with data in every row, request `{ has_headers: false }` → data shifted to A2:D7, row 1 blank, `reference == "A1:D7"`, columns named `Column1..4`. Verify A2 holds what was in A1. |
| `applies_pastel_blue_formatting` | After insertion, header cell has `bg = "#9DC3E6"`, `font.bold = true`; body cell has `bg = "#DDEBF7"`; cells have appropriate borders. |
| `rejects_overlapping_range` | Pre-insert Table1 at A1:D6, attempt second insert with C5:F10 → `Err(OverlapsExistingTable { name: "Table1" })`; workbook unchanged. |
| `rejects_whole_sheet_selection` | Select A1:LAST_ROW/LAST_COLUMN → `Err(WholeSheetSelection)`. |
| `rejects_header_overflow` | 1-row selection with `header_row_count = 1, has_headers = true` → `Err(NoDataRows)`. |
| `auto_names_after_gap` | Pre-insert "Table1" and "Table3", insert third → name = "Table2". |
| `case_insensitive_name_conflict` | Pre-insert "Sales", attempt "sales" → `Err(NameAlreadyTaken)`. |
| `formatting_undo_works_table_record_does_not` | Insert → `model.undo()` → cell styles revert; `workbook.tables["Table1"]` is **still present**. Test name and inline comment document the v1 limitation; this test is the trip-wire for the upstream patch later. |

### Component tests (`src/components/insert_table/form.rs::tests`)

Follow `named_ranges/form.rs::tests` pattern — mount form in isolation
(no `Modal` wrapper), drive signals, observe state:

| Test | Asserts |
|------|---------|
| `default_name_is_next_free` | Mount with `workbook.tables = {"Table1"}` → name input shows `"Table2"`. |
| `submit_calls_model_with_form_state` | Drive form, click Insert; mocked `insert_table_from_selection` receives the right `InsertTableRequest`. |
| `submit_failure_keeps_modal_open` | Mock returns `Err(NameAlreadyTaken { name: "Foo" })` → `tables_modal_open` still `true`; `state.status` holds `Error("Name 'Foo' is already taken")`. |
| `submit_success_closes_modal` | Mock returns `Ok("Table1")` → `tables_modal_open == false`; `state.status` is untouched by the success path (silent close). |
| `escape_closes_modal_without_calling_model` | Esc → `tables_modal_open == false`; mock never called. |

### Keyboard-classifier tests (`src/input/keyboard.rs::tests`)

Add to the existing Ctrl-combos test block (around `keyboard.rs:511`):

```rust
assert_eq!(
    classify_key("m", KeyMod::ctrl(), None),
    Some(SpreadsheetAction::Structure(StructAction::ToggleTableModal))
);
// In editing mode, Ctrl+M is suppressed (consistent with existing
// editing-mode-suppression cases at lines 738-741):
assert_eq!(classify_key("m", KeyMod::ctrl(), Some(&edit_state)), None);
```

### Deliberately not tested in v1

- **Renderer output** — covered by the sibling `Table_Styling_Spec.md`
  Tasks 1-8. This spec only asserts that the model writes the right cell
  `Style` and inserts a `Table` record; whether the canvas paints the
  resolved layered styling correctly is the renderer's contract.
- **XLSX round-trip** — out of scope until upstream
  `xlsx/src/export/tables.rs` exists.
- **Property tests for color contrast** — palette is a hardcoded constant.

---

## Known limitations & migration path

### v1 limitations (acknowledged, not bugs)

1. **Undo skips the `Table` record.** Direct `workbook.tables` mutation
   bypasses IronCalc's `Diff` history. After insertion, `model.undo()`
   reverts the formatting (cells go back to default) but leaves an
   orphaned `Table` entry in `workbook.tables`. Documented in test
   `formatting_undo_works_table_record_does_not`.
2. **No XLSX export.** Upstream `xlsx/src/export/` has no tables file.
   Tables are lost on Save-As. Acceptable for an in-memory test rig.
3. **No edit / delete / rename.** Create-only by design (Q4 = A). To
   recreate with different settings: pick a non-overlapping range, or
   reload the workbook.
4. **`Ctrl+M` is layout-independent but not Excel-standard.** Excel uses
   `Ctrl+T`; that clashes with the browser. Reasonable trade.

### Migration to upstream API

When `UserModel::insert_table(...)` lands in IronCalc, the change to this
codebase is mechanical:

1. In `frontend_model::insert_table_from_selection` step 6, replace
   ```rust
   self.user_model.workbook.tables.insert(name.clone(), table);
   ```
   with
   ```rust
   self.user_model.insert_table(table)?;
   ```
2. Update the limitation test `formatting_undo_works_table_record_does_not`
   to assert the *new* behavior (table record IS removed on undo).
3. Remove the `// TODO(upstream): ...` marker.

Everything else — modal, action, request struct, error enum, formatting
helper, validation — stays identical.

---

## Assumptions

1. `IronCalc::UserModel::insert_rows(sheet, row, count)` exists, is
   public, and properly shifts formula references through the normal
   reference-update machinery. — *Impact: HIGH for the ☐-headers branch
   (Q5 = C). Verify in implementation phase before writing the row-shift
   code; if missing, escalate to upstream patch alongside `insert_table`.*
2. `IronCalc::UserModel::set_cell_style(...)` (or equivalent) is the right
   API for writing borders + fill + font into a cell. — *Impact: MEDIUM.
   If the actual API is `set_cell_fill_color` + `set_cell_border` etc.,
   the `apply_pastel_blue` helper just becomes more verbose.*
3. ~~`Workbook.tables: HashMap<String, Table>` remains `pub` upstream until
   the proper API lands.~~ — **FALSIFIED at implementation time.** The field
   is `pub`, but `UserModel.model: Model<'a>` is `pub(crate)`, and there is
   no public accessor for the workbook on `UserModel`. From outside the
   `ironcalc_base` crate the chain `user_model.workbook.tables` does not
   resolve. *Impact: HIGH — blocks the v1 test rig entirely. See
   "Unblock criteria" above.* This is exactly the kind of two-hop reach
   check that the original spec self-review missed: the leaf field's
   visibility is necessary but not sufficient.
4. `Ctrl+M` is unreserved across Chrome / Firefox / Safari on every host
   OS. — *Impact: LOW. Verified for current versions; revisit only if a
   browser update grabs it.*
5. `state.refocus_formula_input()` is the right post-close focus target.
   — *Impact: LOW. Mirrors `NamedRangesDialog`; if grid focus is wanted
   instead, swap the call.*

---

## Unblock criteria

This spec becomes implementable when **either** of the following lands on
the IronCalc fork (`CoalUnicorn/IronCalc`, `branch: fix-col-iterations`):

- **Preferred — full upstream API.** A public method:
  ```rust
  // ironcalc_base/src/user_model/common.rs
  pub fn insert_table(&mut self, table: Table) -> Result<(), String>
  ```
  with a matching `Diff::TableAdded { table }` variant pushed onto the
  history stack and an undo branch that removes the entry from
  `workbook.tables`. With this in place the spec's
  `frontend_model::insert_table_from_selection` calls `insert_table(table)?`
  directly, the `// TODO(upstream)` marker is deleted, and the limitation
  test `formatting_undo_works_table_record_does_not` is rewritten to
  assert that the table record IS removed on undo.

- **Minimum-viable — accessor only.** A public mutable getter:
  ```rust
  // ironcalc_base/src/user_model/common.rs
  pub fn workbook_mut(&mut self) -> &mut Workbook { &mut self.model.workbook }
  ```
  This is enough to get the v1 test rig moving. Undo would still skip the
  `Table` record (the limitation already documented in the test
  `formatting_undo_works_table_record_does_not`). Migration to the full
  API later is a one-line change at the call site.

When one of these lands, the next pickup should:

1. Re-read this spec — the design is unchanged.
2. Update **Status** at the top from "Blocked" to "Approved, ready for
   `writing-plans`."
3. Update **Assumption #3** (or remove it — the assumption is now satisfied).
4. Invoke the `writing-plans` skill with this spec.

## Sibling docs

- `iron-canvas/table_spec/Table_Styling_Spec.md` — the layered-styling
  rendering pipeline (Tasks 1-8). Out of scope for *creation*; in scope
  for full Excel-parity painting once tables exist.
- `docs/superpowers/specs/2026-05-06-tables-create-from-range-deferred-ui.md`
  — the deferred UI questions (style picker shape, color customization,
  banding toggles).
