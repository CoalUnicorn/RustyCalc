//! Engine-agnostic tabular model: columns + rows of styled string cells,
//! with sorting, selection, viewport, and live mutation. No IronCalc, no web.

use iron_canvas_core::{Alignment, CellStyle, HAlign};

#[derive(Clone, Debug)]
pub struct Column {
    pub header: String,
    pub width: f64,
    pub align: HAlign,
}

impl Column {
    pub fn new(header: impl Into<String>) -> Self {
        Self {
            header: header.into(),
            width: 96.0,
            align: HAlign::General,
        }
    }
    pub fn width(mut self, w: f64) -> Self {
        self.width = w;
        self
    }
    pub fn align(mut self, a: HAlign) -> Self {
        self.align = a;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub value: String,
    pub style: Option<CellStyle>,
}

impl Cell {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            style: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug)]
struct SortState {
    column: usize,
    dir: SortDirection,
}

pub struct DataGrid {
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>, // INSERTION order, never reordered
    order: Vec<usize>,    // display order: indices into `rows`
    sort: Option<SortState>,
    default_row_h: f64,
    top_row: i32, // viewport + active cell, DISPLAY (1-based)
    left_col: i32,
    active_row: i32,
    active_col: i32,
    sel: [i32; 4], // r1,c1,r2,c2 (1-based, display coords)
    frozen_header: bool,
}

#[derive(Default)]
pub struct DataGridBuilder {
    columns: Vec<Column>,
    rows: Vec<Vec<Cell>>,
    default_row_h: Option<f64>,
    frozen_header: bool,
}

impl DataGrid {
    pub fn builder() -> DataGridBuilder {
        DataGridBuilder::default()
    }
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
    pub fn default_row_height(&self) -> f64 {
        self.default_row_h
    }
    pub fn column_width_px(&self, col: usize) -> f64 {
        self.columns.get(col).map(|c| c.width).unwrap_or(96.0)
    }
    pub fn column_header(&self, col: usize) -> Option<&str> {
        self.columns.get(col).map(|c| c.header.as_str())
    }

    pub fn set_frozen_header(&mut self, on: bool) {
        self.frozen_header = on;
    }
    pub(crate) fn frozen_header_enabled(&self) -> bool {
        self.frozen_header
    }

    pub fn set_column_width(&mut self, col: usize, width: f64) {
        if let Some(c) = self.columns.get_mut(col) {
            c.width = width.max(16.0); // sane minimum so a column can't vanish
        }
    }

    /// Current sort as (0-based column, ascending) or `None`.
    pub fn current_sort(&self) -> Option<(usize, bool)> {
        self.sort
            .map(|s| (s.column, matches!(s.dir, SortDirection::Ascending)))
    }
    pub fn cell_value(&self, disp_row: usize, col: usize) -> Option<&str> {
        let src = *self.order.get(disp_row)?;
        self.rows.get(src)?.get(col).map(|c| c.value.as_str())
    }
    pub fn cell_style(&self, disp_row: usize, col: usize) -> Option<&CellStyle> {
        let src = *self.order.get(disp_row)?;
        self.rows.get(src)?.get(col)?.style.as_ref()
    }

    // Raw field accessors for the CanvasModel bridge (display, 1-based).
    pub(crate) fn selection_raw(&self) -> [i32; 4] {
        self.sel
    }
    pub(crate) fn top_row_raw(&self) -> i32 {
        self.top_row
    }
    pub(crate) fn left_col_raw(&self) -> i32 {
        self.left_col
    }
    pub(crate) fn active_row_raw(&self) -> i32 {
        self.active_row
    }
    pub(crate) fn active_col_raw(&self) -> i32 {
        self.active_col
    }

    // Style fallback when a cell carries no explicit style: honor the column's
    // declared horizontal alignment so unstyled cells still align per-column.
    pub(crate) fn column_default_style(&self, col: usize) -> CellStyle {
        let horizontal = self
            .columns
            .get(col)
            .map(|c| c.align)
            .unwrap_or(HAlign::General);
        CellStyle {
            alignment: Some(Alignment {
                horizontal,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    // --- Mutation API (B.3): edits write through display order to source rows ---

    pub fn set_cell(&mut self, disp_row: usize, col: usize, value: impl Into<String>) {
        if let Some(&src) = self.order.get(disp_row)
            && let Some(cell) = self.rows.get_mut(src).and_then(|r| r.get_mut(col))
        {
            cell.value = value.into();
        }
    }

    pub fn append_row(&mut self, cells: Vec<String>) {
        let idx = self.rows.len();
        self.rows.push(cells.into_iter().map(Cell::text).collect());
        self.order.push(idx);
        self.resort(); // keep display order consistent if a sort is active
    }

    pub fn set_data(&mut self, columns: Vec<Column>, rows: Vec<Vec<String>>) {
        self.columns = columns;
        self.rows = rows
            .into_iter()
            .map(|r| r.into_iter().map(Cell::text).collect())
            .collect();
        self.order = (0..self.rows.len()).collect();
        self.sort = None;
        self.clamp_view();
    }

    // --- Sorting (B.4): permutes `order`, never `rows` (insertion order kept) ---

    pub fn sort_by(&mut self, col: usize, dir: SortDirection) {
        self.sort = Some(SortState { column: col, dir });
        self.resort();
    }

    pub fn clear_sort(&mut self) {
        self.sort = None;
        self.order = (0..self.rows.len()).collect();
    }

    // Numeric compare when both cells parse as f64, else lexicographic.
    fn resort(&mut self) {
        let Some(SortState { column, dir }) = self.sort else {
            return;
        };
        let rows = &self.rows;
        self.order.sort_by(|&a, &b| {
            let va = rows
                .get(a)
                .and_then(|r| r.get(column))
                .map(|c| c.value.as_str())
                .unwrap_or("");
            let vb = rows
                .get(b)
                .and_then(|r| r.get(column))
                .map(|c| c.value.as_str())
                .unwrap_or("");
            let ord = match (va.parse::<f64>(), vb.parse::<f64>()) {
                (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                _ => va.cmp(vb),
            };
            match dir {
                SortDirection::Ascending => ord,
                SortDirection::Descending => ord.reverse(),
            }
        });
    }

    // --- Selection + viewport mutators (B.5): all 1-based DISPLAY coords ---

    pub fn set_selection(&mut self, r1: i32, c1: i32, r2: i32, c2: i32) {
        self.sel = [r1, c1, r2, c2];
        self.clamp_view();
    }

    pub fn set_active(&mut self, row: i32, col: i32) {
        self.active_row = row;
        self.active_col = col;
        self.clamp_view();
    }

    pub fn set_scroll(&mut self, top_row: i32, left_col: i32) {
        self.top_row = top_row;
        self.left_col = left_col;
        self.clamp_view();
    }

    pub fn scroll_by(&mut self, d_rows: i32, d_cols: i32) {
        self.top_row += d_rows;
        self.left_col += d_cols;
        self.clamp_view();
    }

    // Keep viewport + selection inside the populated region. On an empty grid
    // (0 rows / 0 cols) clamping leaves the 1-based anchors at 1 without ever
    // computing a negative bound (max with 1 guards the usize->i32 cast).
    fn clamp_view(&mut self) {
        let max_row = self.row_count().max(1) as i32;
        let max_col = self.column_count().max(1) as i32;
        let clamp = |v: i32, hi: i32| v.clamp(1, hi);

        self.top_row = clamp(self.top_row, max_row);
        self.left_col = clamp(self.left_col, max_col);
        self.active_row = clamp(self.active_row, max_row);
        self.active_col = clamp(self.active_col, max_col);
        self.sel = [
            clamp(self.sel[0], max_row),
            clamp(self.sel[1], max_col),
            clamp(self.sel[2], max_row),
            clamp(self.sel[3], max_col),
        ];
    }
}

impl DataGridBuilder {
    pub fn column(mut self, c: Column) -> Self {
        self.columns.push(c);
        self
    }
    pub fn row(mut self, cells: Vec<String>) -> Self {
        self.rows.push(cells.into_iter().map(Cell::text).collect());
        self
    }
    pub fn styled_row(mut self, cells: Vec<Cell>) -> Self {
        self.rows.push(cells);
        self
    }
    pub fn default_row_height(mut self, h: f64) -> Self {
        self.default_row_h = Some(h);
        self
    }
    pub fn frozen_header(mut self, on: bool) -> Self {
        self.frozen_header = on;
        self
    }
    pub fn build(self) -> DataGrid {
        let order = (0..self.rows.len()).collect();
        let mut grid = DataGrid {
            columns: self.columns,
            rows: self.rows,
            order,
            sort: None,
            default_row_h: self.default_row_h.unwrap_or(22.0),
            top_row: 1,
            left_col: 1,
            active_row: 1,
            active_col: 1,
            sel: [1, 1, 1, 1],
            frozen_header: self.frozen_header,
        };
        grid.clamp_view(); // keep viewport + selection valid for an empty grid
        grid
    }
}
