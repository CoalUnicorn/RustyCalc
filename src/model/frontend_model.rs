use ironcalc_base::{
    UserModel, expressions::types::Area, types::HorizontalAlignment, worksheet::NavigationDirection,
};

#[cfg(feature = "dev-tools")]
use leptos::prelude::Set;

use crate::coord::SheetRange;
use crate::model::frontend_types::*;
use crate::state::ModelStore;
use crate::{
    coord::{CellAddress, CellArea, DefinedName},
    input::formula::{FormulaAnalysis, analyze_formula},
};
use iron_canvas_core::geometry::constants::{LAST_COLUMN, LAST_ROW};

use leptos::prelude::UpdateValue;

/// Log a `Navigator` error to the browser console instead of silently
/// discarding it. Mirror of `storage::log_err` for the nav domain.
fn log_nav_err(result: Result<(), String>, ctx: &str) {
    if let Err(e) = result {
        web_sys::console::warn_1(&format!("[rustycalc nav] {ctx}: {e}").into());
    }
}

/// IronCalc's canonical string value for a visible worksheet.
/// Used to guard against silent typos in state comparisons.
pub(crate) const SHEET_STATE_VISIBLE: &str = "visible";

/// Parse formula text in the active cell's context (sheet names, defined
/// names, anchor). Pure read; safe under `with_value`.
pub trait FormulaAnalyzer {
    fn analyze_in_context(&self, text: &str) -> FormulaAnalysis;
}

impl FormulaAnalyzer for UserModel<'_> {
    fn analyze_in_context(&self, text: &str) -> FormulaAnalysis {
        analyze_formula(
            text,
            SheetQuery::active_cell(self),
            &SheetQuery::get_sheet_names(self),
            &DefinedNameManager::get_defined_names(self),
        )
    }
}

/// Workbook defined names: read and CRUD. Every mutating call may change
/// formula evaluation, so wrap call sites in
/// `try_mutate(EvaluationMode::Immediate, …)`. Errors surface verbatim from
/// ironcalc as `Result<_, String>`.
pub trait DefinedNameManager {
    /// Flattened from ironcalc's `DefinedNameS` tuples into our named-field
    /// [`DefinedName`]. Fed to the parser so identifiers like `=my_range`
    /// resolve instead of tripping `NamedVariableKind`.
    fn get_defined_names(&self) -> Vec<DefinedName>;

    /// `Err` if the name is invalid, duplicates an existing name in the same
    /// scope, or the formula won't parse.
    fn create_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        formula: &str,
    ) -> Result<(), String>;

    /// Rename / re-scope / re-formula an existing defined name, identified by
    /// `(old_name, old_scope)`.
    fn rename_defined_name(
        &mut self,
        old_name: &str,
        old_scope: Option<u32>,
        new_name: &str,
        new_scope: Option<u32>,
        new_formula: &str,
    ) -> Result<(), String>;

    /// Delete a defined name. Cells that referenced it surface `#NAME?` after
    /// the next evaluate.
    fn remove_defined_name(&mut self, name: &str, scope: Option<u32>) -> Result<(), String>;
}

impl DefinedNameManager for UserModel<'_> {
    fn get_defined_names(&self) -> Vec<DefinedName> {
        self.get_defined_name_list()
            .into_iter()
            .map(DefinedName::from)
            .collect()
    }

    fn create_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        formula: &str,
    ) -> Result<(), String> {
        self.new_defined_name(name, scope, formula)
    }

    fn rename_defined_name(
        &mut self,
        old_name: &str,
        old_scope: Option<u32>,
        new_name: &str,
        new_scope: Option<u32>,
        new_formula: &str,
    ) -> Result<(), String> {
        self.update_defined_name(old_name, old_scope, new_name, new_scope, new_formula)
    }

    fn remove_defined_name(&mut self, name: &str, scope: Option<u32>) -> Result<(), String> {
        self.delete_defined_name(name, scope)
    }
}

/// Read-only queries against the active workbook / sheet / cell.
pub trait SheetQuery {
    /// Formatting state for the toolbar, derived from the active cell.
    fn toolbar_state(&self) -> ToolbarState;

    /// Number-format code of the active cell (e.g. `"general"`, `"#,##0.00"`).
    #[allow(dead_code)]
    fn active_num_fmt(&self) -> String;

    /// Formatted display string of the active cell (what the user sees in the grid).
    #[allow(dead_code)]
    fn active_cell_display(&self) -> String;

    /// Raw content of the active cell (formula text or literal value).
    fn active_cell_content(&self) -> String;

    /// Position of the active cell.
    fn active_cell(&self) -> CellAddress;

    fn selection(&self) -> Area;

    /// Frozen pane state for the active sheet.
    fn frozen_panes(&self) -> FrozenPanes;

    /// Used data extent of the active sheet (for Ctrl+A, Ctrl+End, etc.).
    fn sheet_dimension(&self) -> CellArea;

    fn get_sheet_name(&self, sheet_idx: usize) -> String;

    fn get_sheet_visible(&self) -> Vec<(u32, u32)>;

    fn get_sheet_tab_color(&self, sheet_idx: usize) -> Option<String>;

    fn get_sheet_visible_count(&self) -> usize;

    fn get_sheet_all(&self) -> Vec<(u32, String, String)>;

    fn get_sheet_names(&self) -> Vec<(u32, String)>;
}

/// Active-cell / selection mutation. Infallible: invalid coordinates are clamped.
pub trait Navigator {
    /// Move the active cell one step. No-op at sheet edges.
    fn nav_arrow(&mut self, dir: ArrowKey);

    /// Move one page up or down.
    fn nav_page(&mut self, dir: PageDir);

    /// Set active cell. Coordinates clamped to valid range - never fails.
    fn nav_set_cell(&mut self, row: i32, col: i32);

    /// Select an entire column (header click).
    fn nav_select_column(&mut self, col: i32);

    /// Select an entire row (header click).
    fn nav_select_row(&mut self, row: i32);

    /// Select the whole sheet (Ctrl+A).
    fn nav_select_all(&mut self);

    /// Extend selection during mouse drag.
    fn nav_extend_selection(&mut self, row: i32, col: i32);

    /// Shift+click on a column header: extend the full-column selection from
    /// the anchor column to `col`, without scrolling to LAST_ROW.
    fn nav_extend_column_selection(&mut self, col: i32);

    /// Shift+click on a row header: extend the full-row selection from the
    /// anchor row to `row`, without scrolling to LAST_COLUMN.
    fn nav_extend_row_selection(&mut self, row: i32);

    /// Jump to the edge of the current data region (Ctrl+Arrow).
    fn nav_to_edge(&mut self, dir: ArrowKey);

    /// Select a rectangular range with the active cell at `(row, col)`.
    /// Coordinates are clamped to valid bounds.
    fn nav_select_range(&mut self, area: CellArea);

    /// Expand selection by one cell (Shift+Arrow).
    fn nav_expand_selection(&mut self, dir: ArrowKey);

    /// Move to column 1 of the current row (Home key).
    fn nav_home_row(&mut self);

    /// Set the selection to `area` (clamped to valid bounds).
    fn set_selected_area(&mut self, area: CellArea);
}

// Helper: map font name String -> SafeFontFamily

fn font_family_from_name(name: &str) -> SafeFontFamily {
    if name.is_empty() {
        SafeFontFamily::SystemUi
    } else {
        SafeFontFamily::from(Some(name))
    }
}

impl SheetQuery for UserModel<'_> {
    fn toolbar_state(&self) -> ToolbarState {
        let view = self.get_selected_view();
        let style = self
            .get_cell_style(view.sheet, view.row, view.column)
            .unwrap_or_default();

        let text_color = match style.font.color.as_deref() {
            None | Some("#000000") => CssColor::new("#000000"),
            Some(c) => CssColor::new(c),
        };
        let bg_color = style
            .fill
            .fg_color
            .as_deref()
            .filter(|c| !c.is_empty())
            .map(CssColor::new);

        let alignment = style.alignment.as_ref();
        let h_align = alignment
            .map(|a| a.horizontal.clone())
            .unwrap_or(HorizontalAlignment::General);
        let v_align = alignment.map(|a| a.vertical.clone()).unwrap_or_default();

        ToolbarState {
            format: TextFormat {
                bold: style.font.b,
                italic: style.font.i,
                underline: style.font.u,
                strikethrough: style.font.strike,
            },

            style: TextStyle {
                font_size: style.font.sz as f64,
                font_family: font_family_from_name(&style.font.name),
                h_align,
                v_align,
                text_color,
                bg_color,
            },
        }
    }

    fn active_num_fmt(&self) -> String {
        let view = self.get_selected_view();
        self.get_cell_style(view.sheet, view.row, view.column)
            .map(|s| s.num_fmt)
            .unwrap_or_else(|_| "general".to_owned())
    }

    fn active_cell_display(&self) -> String {
        let view = self.get_selected_view();
        self.get_formatted_cell_value(view.sheet, view.row, view.column)
            .unwrap_or_default()
    }

    fn active_cell_content(&self) -> String {
        let view = self.get_selected_view();
        self.get_cell_content(view.sheet, view.row, view.column)
            .unwrap_or_default()
    }

    fn active_cell(&self) -> CellAddress {
        let view = self.get_selected_view();
        CellAddress {
            sheet: view.sheet,
            row: view.row,
            column: view.column,
        }
    }

    // TODO: rename this, it returns ironcalc Area type
    // atm only added to input/format.rs:91
    // below is selection_area returns CellArea
    fn selection(&self) -> Area {
        SheetRange::from_view(self).to_ironcalc_area()
    }

    fn frozen_panes(&self) -> FrozenPanes {
        let sheet = self.get_selected_sheet();
        FrozenPanes {
            rows: self.get_frozen_rows_count(sheet).unwrap_or(1),
            cols: self.get_frozen_columns_count(sheet).unwrap_or(1),
        }
    }

    fn sheet_dimension(&self) -> CellArea {
        let sheet = self.get_selected_sheet();
        match self.get_model().workbook.worksheet(sheet) {
            Ok(ws) => {
                let d = ws.dimension();
                CellArea {
                    r1: d.min_row,
                    c1: d.min_column,
                    r2: d.max_row,
                    c2: d.max_column,
                }
            }
            Err(_) => CellArea {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1,
            },
        }
    }

    fn get_sheet_name(&self, sheet_idx: usize) -> String {
        self.get_worksheets_properties()
            .get(sheet_idx)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    fn get_sheet_visible(&self) -> Vec<(u32, u32)> {
        self.get_worksheets_properties()
            .into_iter()
            .enumerate()
            .filter(|(_, s)| s.state == SHEET_STATE_VISIBLE)
            .map(|(idx, s)| (s.sheet_id, idx as u32))
            .collect::<Vec<_>>()
    }

    fn get_sheet_visible_count(&self) -> usize {
        self.get_worksheets_properties()
            .iter()
            .filter(|s| s.state == SHEET_STATE_VISIBLE)
            .count()
    }

    fn get_sheet_tab_color(&self, sheet_idx: usize) -> Option<String> {
        self.get_worksheets_properties()
            .get(sheet_idx)
            .and_then(|s| s.color.clone())
    }

    fn get_sheet_all(&self) -> Vec<(u32, String, String)> {
        self.get_worksheets_properties()
            .into_iter()
            .enumerate()
            .map(|(idx, s)| (idx as u32, s.name.clone(), s.state.clone()))
            .collect::<Vec<_>>()
    }

    // used by analyze_formula
    fn get_sheet_names(&self) -> Vec<(u32, String)> {
        self.get_sheet_all()
            .into_iter()
            .map(|(idx, name, _)| (idx, name))
            .collect()
    }
}

impl Navigator for UserModel<'_> {
    fn nav_arrow(&mut self, dir: ArrowKey) {
        let _ = match dir {
            ArrowKey::Up => self.on_arrow_up(),
            ArrowKey::Down => self.on_arrow_down(),
            ArrowKey::Left => self.on_arrow_left(),
            ArrowKey::Right => self.on_arrow_right(),
        };
    }

    fn nav_page(&mut self, dir: PageDir) {
        let _ = match dir {
            PageDir::Up => self.on_page_up(),
            PageDir::Down => self.on_page_down(),
        };
    }

    fn nav_set_cell(&mut self, row: i32, col: i32) {
        let row = row.clamp(1, LAST_ROW);
        let col = col.clamp(1, LAST_COLUMN);
        log_nav_err(self.set_selected_cell(row, col), "nav_set_cell");
    }

    fn nav_select_column(&mut self, col: i32) {
        log_nav_err(self.set_selected_cell(1, col), "nav_select_column");
        log_nav_err(
            self.set_selected_range(1, col, LAST_ROW, col),
            "nav_select_column_range",
        );
    }

    fn nav_select_row(&mut self, row: i32) {
        log_nav_err(self.set_selected_cell(row, 1), "nav_select_row");
        log_nav_err(self.set_selected_range(row, 1, row, LAST_COLUMN), "nav_select_row_range");
    }

    fn nav_select_all(&mut self) {
        log_nav_err(self.set_selected_cell(1, 1), "nav_select_all");
        log_nav_err(self.set_selected_range(1, 1, LAST_ROW, LAST_COLUMN), "nav_select_all_range");
    }

    fn nav_extend_selection(&mut self, row: i32, col: i32) {
        let _ = self.on_area_selecting(row, col);
    }

    fn nav_extend_column_selection(&mut self, col: i32) {
        let view = self.get_selected_view();
        let anchor = view.column;
        let (c1, c2) = if anchor <= col {
            (anchor, col)
        } else {
            (col, anchor)
        };
        log_nav_err(self.set_selected_range(1, c1, LAST_ROW, c2), "nav_extend_col");
    }

    fn nav_extend_row_selection(&mut self, row: i32) {
        let view = self.get_selected_view();
        let anchor = view.row;
        let (r1, r2) = if anchor <= row {
            (anchor, row)
        } else {
            (row, anchor)
        };
        log_nav_err(self.set_selected_range(r1, 1, r2, LAST_COLUMN), "nav_extend_row");
    }

    fn nav_to_edge(&mut self, dir: ArrowKey) {
        let nd = match dir {
            ArrowKey::Up => NavigationDirection::Up,
            ArrowKey::Down => NavigationDirection::Down,
            ArrowKey::Left => NavigationDirection::Left,
            ArrowKey::Right => NavigationDirection::Right,
        };
        let _ = self.on_navigate_to_edge_in_direction(nd);
    }

    fn nav_select_range(&mut self, area: CellArea) {
        let row = area.r1.clamp(1, LAST_ROW);
        let col = area.c1.clamp(1, LAST_COLUMN);
        let row2 = area.r2.clamp(1, LAST_ROW);
        let col2 = area.c2.clamp(1, LAST_COLUMN);
        log_nav_err(self.set_selected_cell(row, col), "nav_select_range");
        log_nav_err(self.set_selected_range(row, col, row2, col2), "nav_select_range_area");
    }

    fn nav_expand_selection(&mut self, dir: ArrowKey) {
        let key = match dir {
            ArrowKey::Up => "ArrowUp",
            ArrowKey::Down => "ArrowDown",
            ArrowKey::Left => "ArrowLeft",
            ArrowKey::Right => "ArrowRight",
        };
        let _ = self.on_expand_selected_range(key);
    }

    fn nav_home_row(&mut self) {
        let row = self.get_selected_view().row;
        log_nav_err(self.set_selected_cell(row, 1), "nav_home_row");
    }

    fn set_selected_area(&mut self, area: CellArea) {
        log_nav_err(self.set_selected_cell(area.r1, area.c1), "set_selected_area");
        log_nav_err(self.set_selected_range(area.r1, area.c1, area.r2, area.c2), "set_selected_area_range");
    }
}

/// Whether `mutate` should recalculate formulas after applying the closure.
///
/// Pass `EvaluationMode::Immediate` when the mutation may change formula results
/// (cell writes, row/column inserts/deletes).
/// Pass `EvaluationMode::Deferred` for pure navigation, selection, or formatting changes.
#[derive(Clone, Copy)]
pub enum EvaluationMode {
    Immediate,
    Deferred,
}

/// Run `f` on the model, optionally call `evaluate`.
///
/// **PERFORMANCE OPTIMIZED:** Many `UserModel` methods call `evaluate()` internally.
/// We pause evaluation before `f` so the model is evaluated at most once - after
/// all mutations are done. This prevents double evaluation and can halve execution time.
/// See docs/performance-evaluation.md for details.
///
/// **CALLER RESPONSIBILITY:** This function no longer automatically triggers redraws.
/// The caller must emit appropriate events using `state.emit_event()`.
///
pub fn mutate(
    model: ModelStore,
    evaluate: EvaluationMode,
    f: impl FnOnce(&mut UserModel<'static>),
) {
    model.update_value(|m| {
        m.pause_evaluation();
        f(m);
        m.resume_evaluation();
        if matches!(evaluate, EvaluationMode::Immediate) {
            m.evaluate();
        }
    });
    // No automatic redraw - caller must emit specific events
}

/// Fallible variant of [`mutate`]: the closure returns `Result<(), E>`.
///
/// `resume_evaluation()` always runs to leave the model in a consistent state.
/// `evaluate()` is skipped when the closure returns `Err`.
pub fn try_mutate<E>(
    model: ModelStore,
    evaluate: EvaluationMode,
    f: impl FnOnce(&mut UserModel<'static>) -> Result<(), E>,
) -> Result<(), E> {
    // Phase timestamps under the `recorder` feature only. PerfTimings is
    // pulled from Leptos context (provided in app.rs); if it isn't there
    // — e.g. unit tests outside a runtime — the writes are silently skipped.
    #[cfg(feature = "dev-tools")]
    let perf = leptos::prelude::use_context::<crate::perf::PerfTimings>();
    let mut outcome: Result<(), E> = Ok(());
    model.update_value(|m| {
        #[cfg(feature = "dev-tools")]
        if let Some(p) = perf {
            p.commit_start.set(Some(crate::perf::now()));
            // Arm the paint-duration capture: the rAF loop's
            // `if render_ms.is_none()` guard writes the next paint's
            // duration and then leaves it alone until the next commit.
            p.render_ms.set(None);
        }
        m.pause_evaluation();
        outcome = f(m);
        #[cfg(feature = "dev-tools")]
        if let Some(p) = perf {
            p.input_done.set(Some(crate::perf::now()));
        }
        m.resume_evaluation();
        if outcome.is_ok() && matches!(evaluate, EvaluationMode::Immediate) {
            m.evaluate();
            #[cfg(feature = "dev-tools")]
            if let Some(p) = perf {
                p.eval_done.set(Some(crate::perf::now()));
            }
        }
    });
    outcome
}

// Tests
