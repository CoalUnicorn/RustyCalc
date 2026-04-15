use ironcalc_base::{
    expressions::types::Area,
    types::{CellType, HorizontalAlignment, VerticalAlignment},
    worksheet::NavigationDirection,
    UserModel,
};

use leptos::prelude::Set;

use crate::coord::{CellAddress, CellArea};
use crate::model::frontend_types::*;
use crate::state::ModelStore;
use crate::{
    canvas::geometry::{LAST_COLUMN, LAST_ROW},
    coord::SheetArea,
};

use leptos::prelude::UpdateValue;

/// IronCalc's canonical string value for a visible worksheet.
/// Used to guard against silent typos in state comparisons.
pub(crate) const SHEET_STATE_VISIBLE: &str = "visible";

pub trait FrontendModel {
    // Query

    /// Fully resolved style for one cell.
    ///
    /// `default_text_color` is the theme's text color (differs in dark mode);
    /// the renderer passes `self.theme.default_text_color`, the toolbar passes `"#000000"`.
    fn cell_style(
        &self,
        sheet: u32,
        row: i32,
        col: i32,
        default_text_color: &str,
    ) -> ResolvedCellStyle;

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
    // Navigation (infallible)

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

impl FrontendModel for UserModel<'_> {
    fn cell_style(
        &self,
        sheet: u32,
        row: i32,
        col: i32,
        default_text_color: &str,
    ) -> ResolvedCellStyle {
        let style = self.get_cell_style(sheet, row, col).unwrap_or_default();
        let cell_type = self
            .get_cell_type(sheet, row, col)
            .unwrap_or(CellType::Text);

        let text_color = match style.font.color.as_deref() {
            None | Some("#000000") => CssColor::new(default_text_color),
            Some(c) => CssColor::new(c),
        };

        // Font
        let size_px = style.font.sz as f64;
        let bold = style.font.b;
        let italic = style.font.i;
        let family = font_family_from_name(&style.font.name);
        let css = ResolvedFont::build(size_px, bold, italic, &family);
        let font = ResolvedFont {
            size_px,
            underline: style.font.u,
            strikethrough: style.font.strike,
            css,
        };

        // Alignment
        let alignment = style.alignment.as_ref();
        let h_align = match alignment.map(|a| &a.horizontal) {
            Some(HorizontalAlignment::Right) => HorizontalAlignment::Right,
            Some(HorizontalAlignment::Center) | Some(HorizontalAlignment::CenterContinuous) => {
                HorizontalAlignment::Center
            }
            Some(HorizontalAlignment::Left) | Some(HorizontalAlignment::Fill) => {
                HorizontalAlignment::Left
            }
            Some(HorizontalAlignment::Justify) | Some(HorizontalAlignment::Distributed) => {
                // Canvas 2D has no justify/distributed - fall back to left.
                HorizontalAlignment::Left
            }
            // General or unset: numbers right, everything else left.
            None | Some(HorizontalAlignment::General) => match cell_type {
                CellType::Number => HorizontalAlignment::Right,
                _ => HorizontalAlignment::Left,
            },
        };
        let v_align = alignment
            .map(|a| a.vertical.clone())
            .unwrap_or(VerticalAlignment::Bottom);
        let wrap_text = alignment.map(|a| a.wrap_text).unwrap_or(false);

        ResolvedCellStyle {
            text_color,
            font,
            h_align,
            v_align,
            wrap_text,
        }
    }

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

        let h_align = style
            .alignment
            .as_ref()
            .map(|a| a.horizontal.clone())
            .unwrap_or(HorizontalAlignment::General);

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
        SheetArea::from_view(self).to_ironcalc_area()
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

    // Navigation

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
        let _ = self.set_selected_cell(row, col);
    }

    fn nav_select_column(&mut self, col: i32) {
        let _ = self.set_selected_cell(1, col);
        let _ = self.set_selected_range(1, col, LAST_ROW, col);
    }

    fn nav_select_row(&mut self, row: i32) {
        let _ = self.set_selected_cell(row, 1);
        let _ = self.set_selected_range(row, 1, row, LAST_COLUMN);
    }

    fn nav_select_all(&mut self) {
        let _ = self.set_selected_cell(1, 1);
        let _ = self.set_selected_range(1, 1, LAST_ROW, LAST_COLUMN);
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
        let _ = self.set_selected_range(1, c1, LAST_ROW, c2);
    }

    fn nav_extend_row_selection(&mut self, row: i32) {
        let view = self.get_selected_view();
        let anchor = view.row;
        let (r1, r2) = if anchor <= row {
            (anchor, row)
        } else {
            (row, anchor)
        };
        let _ = self.set_selected_range(r1, 1, r2, LAST_COLUMN);
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
        let _ = self.set_selected_cell(row, col);
        let _ = self.set_selected_range(row, col, row2, col2);
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
        let _ = self.set_selected_cell(row, 1);
    }

    fn set_selected_area(&mut self, area: CellArea) {
        let _ = self.set_selected_cell(area.r1, area.c1);
        let _ = self.set_selected_range(area.r1, area.c1, area.r2, area.c2);
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
    let mut outcome: Result<(), E> = Ok(());
    model.update_value(|m| {
        m.pause_evaluation();
        outcome = f(m);
        m.resume_evaluation();
        if outcome.is_ok() && matches!(evaluate, EvaluationMode::Immediate) {
            m.evaluate();
        }
    });
    outcome
}

/// Timed variant of [`try_mutate`]: records phase timestamps into [`PerfTimings`].
///
/// Sets `commit_start` before the closure, `input_done` after it, and `eval_done`
/// after `evaluate()`. The caller sets `last_formula` before calling (context-specific).
/// `render_done` is set separately by the canvas render effect in `worksheet.rs`.
#[allow(dead_code)]
pub fn try_mutate_timed<E>(
    model: ModelStore,
    evaluate: EvaluationMode,
    perf: crate::perf::PerfTimings,
    f: impl FnOnce(&mut UserModel<'static>) -> Result<(), E>,
) -> Result<(), E> {
    let mut outcome: Result<(), E> = Ok(());
    model.update_value(|m| {
        perf.commit_start.set(Some(crate::perf::now()));
        m.pause_evaluation();
        outcome = f(m);
        perf.input_done.set(Some(crate::perf::now()));
        m.resume_evaluation();
        if outcome.is_ok() && matches!(evaluate, EvaluationMode::Immediate) {
            m.evaluate();
            perf.eval_done.set(Some(crate::perf::now()));
        }
    });
    outcome
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a minimal empty workbook model for testing.
    #[allow(clippy::expect_used)]
    fn make_model() -> UserModel<'static> {
        UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("failed to create test model")
    }

    #[test]
    fn cell_style_defaults_for_empty_cell() {
        let m = make_model();
        // Empty cell should have sensible defaults
        let style = m.cell_style(0, 1, 1, "#000000");
        // assert!(style.bg_color.is_none());
        assert_eq!(style.text_color.as_str(), "#000000");
        // Empty/missing cells return CellType::Number from the base library,
        // so General alignment resolves to Right (no visible effect since
        // empty cells produce no rendered text).
        assert_eq!(style.h_align, HorizontalAlignment::Right);
    }

    #[test]
    fn cell_style_uses_theme_color_for_automatic() {
        let m = make_model();
        // Empty cell style - should fall back to theme color
        let style = m.cell_style(0, 1, 1, "#FFFFFF");
        assert_eq!(style.text_color.as_str(), "#ffffff");
    }

    #[test]
    fn toolbar_state_reflects_active_cell() {
        let m = make_model();
        let ts = m.toolbar_state();
        assert!(!ts.format.bold);
        assert!(!ts.format.italic);
        assert!(ts.style.font_size > 0.0);
    }

    #[test]
    fn nav_arrow_down_moves_selection() {
        let mut m = make_model();
        let before = m.get_selected_view().row;
        m.nav_arrow(ArrowKey::Down);
        let after = m.get_selected_view().row;
        assert_eq!(after, before + 1);
    }

    #[test]
    fn nav_set_cell_clamps_out_of_range() {
        let mut m = make_model();
        m.nav_set_cell(-1, 0);
        let v = m.get_selected_view();
        assert_eq!(v.row, 1);
        assert_eq!(v.column, 1);
    }

    #[test]
    fn nav_select_range_sets_active_cell_and_range() {
        let mut m = make_model();
        m.nav_select_range(CellArea {
            r1: 2,
            c1: 3,
            r2: 5,
            c2: 7,
        });
        let v = m.get_selected_view();
        assert_eq!(v.row, 2);
        assert_eq!(v.column, 3);
        assert_eq!(v.range, [2, 3, 5, 7]);
    }

    #[test]
    fn nav_expand_selection_extends_range() {
        let mut m = make_model();
        // Start at (1,1), expand down: range should cover row 1..2
        m.nav_expand_selection(ArrowKey::Down);
        let v = m.get_selected_view();
        let r_min = v.range[0].min(v.range[2]);
        let r_max = v.range[0].max(v.range[2]);
        assert_eq!(r_min, 1);
        assert_eq!(r_max, 2);
    }

    #[test]
    fn nav_home_row_moves_to_column_1() {
        let mut m = make_model();
        m.nav_set_cell(5, 10);
        m.nav_home_row();
        let v = m.get_selected_view();
        assert_eq!(v.row, 5);
        assert_eq!(v.column, 1);
    }

    #[test]
    fn nav_select_column_sets_full_range() {
        let mut m = make_model();
        m.nav_select_column(3);
        let v = m.get_selected_view();
        assert_eq!(v.column, 3);
        assert_eq!(v.range[1], 3);
        assert_eq!(v.range[3], 3);
    }

    #[test]
    fn sheet_dimension_empty_sheet() {
        let m = make_model();
        let d = m.sheet_dimension();
        // Empty sheet defaults to (1,1,1,1).
        assert_eq!(d.r1, 1);
        assert_eq!(d.c1, 1);
        assert_eq!(d.r2, 1);
        assert_eq!(d.c2, 1);
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn sheet_dimension_after_input() {
        let mut m = make_model();
        m.set_user_input(0, 5, 3, "hello").unwrap();
        m.evaluate();
        let d = m.sheet_dimension();
        assert!(d.r2 >= 5);
        assert!(d.c2 >= 3);
    }
}
