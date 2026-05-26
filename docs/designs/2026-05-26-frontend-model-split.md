# FrontendModel Trait Split — Design Spec

**Date:** 2026-05-26
**Status:** Draft — Phase 3 prep (2 traits already extracted by Claude in Phase 2 spillover)
**Plan:** `docs/plans/2026-05-26-audit-remediation.md` Phase 3

## Problem

FrontendModel has 28 methods spanning two concerns: sheet/state queries (14 methods) and
navigation (14 methods). LOCK-1 (IronCalc type leakage) and PERF-3 (clone-heavy return
types) are symptoms of these methods living in one trait. Two traits have already been
carved out (FormulaAnalyzer, DefinedNameManager) leaving the core monolith.

Every component that needs `active_cell()` also gets `nav_to_edge()` transitively.
Adding a navigation method touches the trait + the `FrontendModel` impl + every
component's import surface.

## Current State

After Claude's Phase 2 spillover, the trait structure is:

```
FormulaAnalyzer          — 1 method  (analyze_in_context)          ← extracted
DefinedNameManager       — 4 methods (CRUD defined names)          ← extracted
FrontendModel            — 28 methods (14 query + 14 navigation)   ← TO SPLIT
```

## Design

Split the 28 remaining methods into two focused traits — no new abstractions, just
partitioning what already exists.

### SheetQuery (read-only)

```rust
pub trait SheetQuery {
    // Display / toolbar
    fn toolbar_state(&self) -> ToolbarState;
    fn active_num_fmt(&self) -> String;
    fn active_cell_display(&self) -> String;
    fn active_cell_content(&self) -> String;

    // Position
    fn active_cell(&self) -> CellAddress;
    fn selection(&self) -> Area;   // ← LOCK-1 leak: raw IronCalc Area

    // Sheet info
    fn frozen_panes(&self) -> FrozenPanes;
    fn sheet_dimension(&self) -> CellArea;
    fn get_sheet_name(&self, sheet_idx: usize) -> String;
    fn get_sheet_visible(&self) -> Vec<(u32, u32)>;
    fn get_sheet_tab_color(&self, sheet_idx: usize) -> Option<String>;
    fn get_sheet_visible_count(&self) -> usize;
    fn get_sheet_all(&self) -> Vec<(u32, String, String)>;
    fn get_sheet_names(&self) -> Vec<(u32, String)>;
}
```

### Navigator (mut-only)

```rust
pub trait Navigator {
    fn nav_arrow(&mut self, dir: ArrowKey);
    fn nav_page(&mut self, dir: PageDir);
    fn nav_set_cell(&mut self, row: i32, col: i32);
    fn nav_select_column(&mut self, col: i32);
    fn nav_select_row(&mut self, row: i32);
    fn nav_select_all(&mut self);
    fn nav_extend_selection(&mut self, row: i32, col: i32);
    fn nav_extend_column_selection(&mut self, col: i32);
    fn nav_extend_row_selection(&mut self, row: i32);
    fn nav_to_edge(&mut self, dir: ArrowKey);
    fn nav_select_range(&mut self, area: CellArea);
    fn nav_expand_selection(&mut self, dir: ArrowKey);
    fn nav_home_row(&mut self);
    fn set_selected_area(&mut self, area: CellArea);
}
```

### Why two traits, not four?

The component import map shows natural groupings:

| Component | Needs |
|-----------|-------|
| worksheet.rs (canvas bridge) | SheetQuery + Navigator |
| toolbar/mod.rs | SheetQuery only |
| sheet_tabs.rs | SheetQuery only |
| formula_bar.rs | SheetQuery only |
| keyboard.rs | SheetQuery + Navigator |
| mouse.rs | SheetQuery + Navigator |
| left_drawer.rs | SheetQuery only (sheet names) |
| file_bar.rs | Navigator (nav after file ops) |

Splitting further (CellEditor, FormatProvider, SelectionState) would create traits with
3-5 methods each — more import boilerplate than it saves. The SheetQuery/Navigator
split follows the Rust convention of read-only vs mutation traits.

## File Impact

| File | Action | What changes |
|---|---|---|
| `src/model/frontend_model.rs` | Modify | Replace one 28-method trait with SheetQuery + Navigator |
| `src/model/mod.rs` | Modify | Re-export SheetQuery, Navigator |
| `src/components/worksheet.rs` | Modify | Import SheetQuery + Navigator |
| `src/components/toolbar/mod.rs` | Modify | Import SheetQuery only |
| `src/components/sheet_tabs.rs` | Modify | Import SheetQuery only |
| `src/components/formula_bar.rs` | Modify | Import SheetQuery only |
| `src/input/keyboard.rs` | Modify | Import SheetQuery + Navigator |
| `src/input/mouse.rs` | Modify | Import SheetQuery + Navigator |
| `src/components/left_drawer.rs` | Modify | Import SheetQuery only |
| `src/components/file_bar.rs` | Modify | Import Navigator only |
| `src/input/nav.rs` | Modify | Import Navigator only |

10 files total. Most changes are single-line import swaps (`FrontendModel` → `SheetQuery`
or `SheetQuery + Navigator`).

## Trade-offs

- **Gain**: Each component only depends on the trait it actually uses. Adding a
  navigation method no longer touches toolbar/formula-bar/sheet-tabs files.
- **Gain**: LOCK-1 partially addressed — components importing only `SheetQuery` don't
  transitively depend on navigator methods that return IronCalc types.
- **Gain**: PERF-3 partially addressed — future return-type changes (`String` → `Cow<'_, str>`)
  affect only `SheetQuery`, not the whole codebase.
- **Lose**: Minor import ceremony (one extra `use` in 4 files that need both traits).
  Worth it for the dependency clarity.
- **Alternative considered**: 5-trait split (CellEditor, SheetManager, FormatProvider,
  SelectionState, Navigator). Rejected because 3-5 method traits just add import boilerplate
  without reducing coupling further than the 2-trait split.

## What stays unchanged

- `FrontendModel` continues to exist as a supertrait or blanket impl for backwards compat.
  Components not yet migrated keep working.
- `FormulaAnalyzer` and `DefinedNameManager` stay separate (already extracted).
- The `impl SheetQuery for UserModel<'_>` and `impl Navigator for UserModel<'_>` are
  trivially derived from the existing `impl FrontendModel for UserModel<'_>` body.
- GAP-1 (`selection()` returns raw IronCalc `Area`): NOT addressed in this split.
  Filed as a follow-up.

## Tests

No new tests needed — this is a pure refactor. Existing tests verify behavior.
Run full test suite after split to catch any wiring regressions.

```bash
cargo test -p rusty-calc
cargo clippy
```

## Review Checklist

- [ ] No parallel types (using CellAddress, SheetRange, CellArea)
- [ ] Correct EvaluationMode (existing callers unchanged)
- [ ] Painter-only paint path (unchanged)
- [ ] Exhaustive match on PaneRegion / BorderEdge / Axis / HitTest (unchanged)
- [ ] Area converted at ironcalc boundary only (unchanged — GAP-1 filed separately)
- [ ] SheetQuery methods are all `&self`; Navigator methods are all `&mut self`
- [ ] All 28 methods accounted for (14 in SheetQuery, 14 in Navigator)
- [ ] Backwards-compat: `FrontendModel` blanket impl or supertrait preserved
