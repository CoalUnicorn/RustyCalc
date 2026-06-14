use crate::DataGrid;
use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{CanvasModel, CanvasView, CellContentQuery, CellKind, CellStyle, Fetched};

impl CanvasModel for DataGrid {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        let [r1, c1, r2, c2] = self.selection_raw();
        Some(CanvasView {
            sheet: 0,
            row: self.active_row_raw(),
            column: self.active_col_raw(),
            selection: RCRange { r1, c1, r2, c2 },
            top_row: self.top_row_raw(),
            left_column: self.left_col_raw(),
        })
    }
    fn get_frozen_rows_count(&self, _s: u32) -> Option<i32> {
        Some(if self.frozen_header_enabled() { 1 } else { 0 })
    }
    fn get_frozen_columns_count(&self, _s: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _s: u32, _row: i32) -> Option<f64> {
        Some(self.default_row_height())
    }
    fn get_column_width(&self, _s: u32, column: i32) -> Option<f64> {
        if column < 1 {
            return Some(96.0); // row-header gutter — standard column width
        }
        Some(self.column_width_px((column - 1) as usize))
    }
    fn get_show_grid_lines(&self, _s: u32) -> Option<bool> {
        Some(true)
    }
    /// Finite grid: scroll extents, blit rebuilds, and the autofill guard
    /// end at the data. Floored at 1 so an empty grid keeps one addressable
    /// row — the walk never sees a zero-row axis.
    fn last_row(&self, _s: u32) -> i32 {
        (self.row_count() as i32).max(1)
    }
    fn last_column(&self, _s: u32) -> i32 {
        (self.column_count() as i32).max(1)
    }
    fn get_column_header_text(&self, _s: u32, col: i32) -> Option<String> {
        if col < 1 {
            return None;
        }
        self.column_header((col - 1) as usize).map(str::to_owned)
    }
    fn get_row_header_text(&self, _s: u32, _row: i32) -> Option<String> {
        // DataGrid has no row-header customization — always use numeric labels.
        // Override is explicit (instead of relying on the trait default) so the
        // symmetry with `get_column_header_text` is visible at the impl site.
        None
    }
    fn get_show_row_headers(&self, _s: u32) -> Option<bool> {
        Some(self.show_headers_enabled())
    }
    fn get_show_col_headers(&self, _s: u32) -> Option<bool> {
        Some(self.show_headers_enabled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::CanvasModel;

    #[test]
    fn headerless_grid_hides_both_header_strips() {
        let grid = DataGrid::builder().show_headers(false).build();
        assert_eq!(grid.get_show_row_headers(0), Some(false));
        assert_eq!(grid.get_show_col_headers(0), Some(false));
    }

    #[test]
    fn headers_default_on() {
        let grid = DataGrid::builder().build();
        assert_eq!(grid.get_show_row_headers(0), Some(true));
        assert_eq!(grid.get_show_col_headers(0), Some(true));
    }
}

impl CellContentQuery for DataGrid {
    fn get_cell_style(&self, _s: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        if row < 1 || column < 1 {
            return Fetched::Value(CellStyle::default());
        }
        match self.cell_style((row - 1) as usize, (column - 1) as usize) {
            Some(st) => Fetched::Value(st.clone()),
            None => Fetched::Value(self.column_default_style((column - 1) as usize)),
        }
    }
    fn get_cell_type(&self, _s: u32, _row: i32, _col: i32) -> Fetched<CellKind> {
        Fetched::Value(CellKind::Text)
    }
    fn get_formatted_cell_value(&self, _s: u32, row: i32, column: i32) -> Fetched<String> {
        if row < 1 || column < 1 {
            return Fetched::Absent;
        }
        match self.cell_value((row - 1) as usize, (column - 1) as usize) {
            Some(v) => Fetched::Value(v.to_owned()),
            None => Fetched::Absent,
        }
    }

    // Batched overrides. The default `*_in` loop dispatches the single-cell
    // accessor through the trait per cell — a vtable hop on `&dyn CanvasModel`
    // for every visible cell each frame, plus a fresh column-default build per
    // blank cell. These drain grid storage directly and hoist each column's
    // default style out of the row loop. Row-major order (row outer, column
    // inner) mirrors the default impl the renderer indexes against.
    fn get_cell_styles_in(&self, _s: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        out.clear();
        let col_defaults: Vec<CellStyle> = (range.c1..=range.c2)
            .map(|c| {
                if c < 1 {
                    CellStyle::default()
                } else {
                    self.column_default_style((c - 1) as usize)
                }
            })
            .collect();
        for r in range.r1..=range.r2 {
            for (i, c) in (range.c1..=range.c2).enumerate() {
                let style = if r < 1 || c < 1 {
                    CellStyle::default()
                } else {
                    self.cell_style((r - 1) as usize, (c - 1) as usize)
                        .cloned()
                        .unwrap_or_else(|| col_defaults[i].clone())
                };
                // Local storage never bridges: a present cell is `Value`, an
                // out-of-data slot is `Absent` (its column default still rode in
                // above), never `BridgeFailed`.
                out.push(Fetched::Value(style));
            }
        }
    }

    fn get_formatted_cell_values_in(&self, _s: u32, range: RCRange, out: &mut Vec<Fetched<String>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                let value = if r < 1 || c < 1 {
                    Fetched::Absent
                } else {
                    self.cell_value((r - 1) as usize, (c - 1) as usize)
                        .map_or(Fetched::Absent, |v| Fetched::Value(v.to_owned()))
                };
                out.push(value);
            }
        }
    }

    fn get_cell_types_in(&self, _s: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        // DataGrid is text-only, so every in-range cell is the same kind.
        out.clear();
        let cells = (range.r2 - range.r1 + 1).max(0) * (range.c2 - range.c1 + 1).max(0);
        out.resize(cells as usize, Fetched::Value(CellKind::Text));
    }
}
