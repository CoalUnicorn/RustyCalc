use ironcalc_base::{
    types::{CellType, HorizontalAlignment, Style, VerticalAlignment},
    worksheet::NavigationDirection,
    UserModel,
};

use crate::canvas::geometry::{LAST_COLUMN, LAST_ROW};
use crate::model::clipboard_bridge::CellArea;
use crate::model::frontend_types::*;

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

    /// Frozen pane state for the active sheet.
    fn frozen_panes(&self) -> FrozenPanes;

    /// Used data extent of the active sheet (for Ctrl+A, Ctrl+End, etc.).
    fn sheet_dimension(&self) -> SheetDimension;

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

    /// Jump to the edge of the current data region (Ctrl+Arrow).
    fn nav_to_edge(&mut self, dir: ArrowKey);

    /// Select a rectangular range with the active cell at `(row, col)`.
    /// Coordinates are clamped to valid bounds.
    fn nav_select_range(&mut self, row: i32, col: i32, row2: i32, col2: i32);

    /// Expand selection by one cell (Shift+Arrow).
    fn nav_expand_selection(&mut self, dir: ArrowKey);

    /// Move to column 1 of the current row (Home key).
    fn nav_home_row(&mut self);

    // Selection

    /// Active cell range as a `CellArea`.
    fn selected_area(&self) -> CellArea;

    /// Active sheet index.
    fn selected_sheet(&self) -> u32;

    /// Set the selection to `area` (clamped to valid bounds).
    fn set_selected_area(&mut self, area: CellArea);

    // Cell editing

    /// Write a raw value or formula into a cell.
    fn input_cell(&mut self, sheet: u32, row: i32, col: i32, value: &str) -> Result<(), String>;

    /// Clear cell contents (values + formulas) in `area`, preserving formatting.
    fn clear_contents(&mut self, sheet: u32, area: CellArea) -> Result<(), String>;

    /// Clear everything (contents + formatting) in `area`.
    fn clear_all(&mut self, sheet: u32, area: CellArea) -> Result<(), String>;

    /// Clear only formatting in `area`, preserving cell values.
    fn clear_formatting(&mut self, sheet: u32, area: CellArea) -> Result<(), String>;

    /// Apply a style property to `area` (delegates to `update_range_style`).
    fn apply_style(
        &mut self,
        sheet: u32,
        area: CellArea,
        path: &str,
        value: &str,
    ) -> Result<(), String>;

    // Structure

    fn insert_rows(&mut self, sheet: u32, row: i32, count: i32) -> Result<(), String>;
    fn delete_rows(&mut self, sheet: u32, row: i32, count: i32) -> Result<(), String>;
    fn insert_cols(&mut self, sheet: u32, col: i32, count: i32) -> Result<(), String>;
    fn delete_cols(&mut self, sheet: u32, col: i32, count: i32) -> Result<(), String>;

    /// Extend `area` rows downward to `to_row` using autofill.
    fn auto_fill_rows(&mut self, sheet: u32, area: CellArea, to_row: i32) -> Result<(), String>;

    /// Extend `area` columns rightward to `to_col` using autofill.
    fn auto_fill_cols(&mut self, sheet: u32, area: CellArea, to_col: i32) -> Result<(), String>;

    // History

    fn undo(&mut self) -> Result<(), String>;
    fn redo(&mut self) -> Result<(), String>;
    fn can_undo(&self) -> bool;
    fn can_redo(&self) -> bool;

    // Frozen panes

    /// Freeze `count` rows from the top on the active sheet.
    fn set_frozen_rows(&mut self, count: i32) -> Result<(), String>;

    /// Freeze `count` columns from the left on the active sheet.
    fn set_frozen_cols(&mut self, count: i32) -> Result<(), String>;

    // Dimensions & display

    fn row_height(&self, sheet: u32, row: i32) -> f64;
    fn col_width(&self, sheet: u32, col: i32) -> f64;
    fn show_grid_lines(&self) -> bool;
}

// Helper: map font name String -> SafeFontFamily

fn font_family_from_name(name: &str) -> SafeFontFamily {
    if name.is_empty() {
        SafeFontFamily::SystemUi
    } else {
        SafeFontFamily::from(Some(name))
    }
}

// impl

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

        // Text color
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
            // family,
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

    fn frozen_panes(&self) -> FrozenPanes {
        let sheet = self.get_selected_sheet();
        FrozenPanes {
            rows: self.get_frozen_rows_count(sheet).unwrap_or(1),
            cols: self.get_frozen_columns_count(sheet).unwrap_or(1),
        }
    }

    fn sheet_dimension(&self) -> SheetDimension {
        let sheet = self.get_selected_sheet();
        match self.get_model().workbook.worksheet(sheet) {
            Ok(ws) => {
                let d = ws.dimension();
                SheetDimension {
                    min_row: d.min_row,
                    min_column: d.min_column,
                    max_row: d.max_row,
                    max_column: d.max_column,
                }
            }
            Err(_) => SheetDimension {
                min_row: 1,
                min_column: 1,
                max_row: 1,
                max_column: 1,
            },
        }
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

    fn nav_to_edge(&mut self, dir: ArrowKey) {
        let nd = match dir {
            ArrowKey::Up => NavigationDirection::Up,
            ArrowKey::Down => NavigationDirection::Down,
            ArrowKey::Left => NavigationDirection::Left,
            ArrowKey::Right => NavigationDirection::Right,
        };
        let _ = self.on_navigate_to_edge_in_direction(nd);
    }

    fn nav_select_range(&mut self, row: i32, col: i32, row2: i32, col2: i32) {
        let row = row.clamp(1, LAST_ROW);
        let col = col.clamp(1, LAST_COLUMN);
        let row2 = row2.clamp(1, LAST_ROW);
        let col2 = col2.clamp(1, LAST_COLUMN);
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

    // Selection

    fn selected_area(&self) -> CellArea {
        CellArea::from(self.get_selected_view().range)
    }

    fn selected_sheet(&self) -> u32 {
        self.get_selected_view().sheet
    }

    fn set_selected_area(&mut self, area: CellArea) {
        let _ = self.set_selected_cell(area.r1, area.c1);
        let _ = self.set_selected_range(area.r1, area.c1, area.r2, area.c2);
    }

    // Cell editing

    fn input_cell(&mut self, sheet: u32, row: i32, col: i32, value: &str) -> Result<(), String> {
        self.set_user_input(sheet, row, col, value)
    }

    fn clear_contents(&mut self, sheet: u32, area: CellArea) -> Result<(), String> {
        self.range_clear_contents(&area.to_area(sheet))
    }

    fn clear_all(&mut self, sheet: u32, area: CellArea) -> Result<(), String> {
        self.range_clear_all(&area.to_area(sheet))
    }

    fn clear_formatting(&mut self, sheet: u32, area: CellArea) -> Result<(), String> {
        self.range_clear_formatting(&area.to_area(sheet))
    }

    fn apply_style(
        &mut self,
        sheet: u32,
        area: CellArea,
        path: &str,
        value: &str,
    ) -> Result<(), String> {
        self.update_range_style(&area.to_area(sheet), path, value)
    }

    // Structure

    fn insert_rows(&mut self, sheet: u32, row: i32, count: i32) -> Result<(), String> {
        UserModel::insert_rows(self, sheet, row, count)
    }

    fn delete_rows(&mut self, sheet: u32, row: i32, count: i32) -> Result<(), String> {
        UserModel::delete_rows(self, sheet, row, count)
    }

    fn insert_cols(&mut self, sheet: u32, col: i32, count: i32) -> Result<(), String> {
        self.insert_columns(sheet, col, count)
    }

    fn delete_cols(&mut self, sheet: u32, col: i32, count: i32) -> Result<(), String> {
        self.delete_columns(sheet, col, count)
    }

    fn auto_fill_rows(&mut self, sheet: u32, area: CellArea, to_row: i32) -> Result<(), String> {
        UserModel::auto_fill_rows(self, &area.to_area(sheet), to_row)
    }

    fn auto_fill_cols(&mut self, sheet: u32, area: CellArea, to_col: i32) -> Result<(), String> {
        self.auto_fill_columns(&area.to_area(sheet), to_col)
    }

    // History

    fn undo(&mut self) -> Result<(), String> {
        UserModel::undo(self)
    }

    fn redo(&mut self) -> Result<(), String> {
        UserModel::redo(self)
    }

    fn can_undo(&self) -> bool {
        UserModel::can_undo(self)
    }

    fn can_redo(&self) -> bool {
        UserModel::can_redo(self)
    }

    // Frozen panes

    fn set_frozen_rows(&mut self, count: i32) -> Result<(), String> {
        let sheet = self.get_selected_sheet();
        self.set_frozen_rows_count(sheet, count)
    }

    fn set_frozen_cols(&mut self, count: i32) -> Result<(), String> {
        let sheet = self.get_selected_sheet();
        self.set_frozen_columns_count(sheet, count)
    }

    // Dimensions & display

    fn row_height(&self, sheet: u32, row: i32) -> f64 {
        self.get_row_height(sheet, row).unwrap_or(20.0)
    }

    fn col_width(&self, sheet: u32, col: i32) -> f64 {
        self.get_column_width(sheet, col).unwrap_or(100.0)
    }

    fn show_grid_lines(&self) -> bool {
        let sheet = self.get_selected_sheet();
        self.get_show_grid_lines(sheet).unwrap_or(true)
    }
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
        assert_eq!(style.text_color.as_str(), "#FFFFFF");
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
        m.nav_select_range(2, 3, 5, 7);
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
        assert_eq!(d.min_row, 1);
        assert_eq!(d.min_column, 1);
        assert_eq!(d.max_row, 1);
        assert_eq!(d.max_column, 1);
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn sheet_dimension_after_input() {
        let mut m = make_model();
        m.set_user_input(0, 5, 3, "hello").unwrap();
        m.evaluate();
        let d = m.sheet_dimension();
        assert!(d.max_row >= 5);
        assert!(d.max_column >= 3);
    }
}
