use iron_canvas_core::{Alignment, CellStyle, HAlign, HitTest, ResizeTarget};
use iron_canvas_datagrid::{Cell, Column, DataGrid};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridDataWire {
    #[serde(default)]
    pub columns: Vec<ColumnWire>,
    #[serde(default)]
    pub rows: Vec<RowWire>,
    pub row_height: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnWire {
    pub header: String,
    pub width: Option<f64>,
    pub align: Option<String>, // "left" | "center" | "right"
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowWire {
    #[serde(default)]
    pub cells: Vec<CellWire>,
    pub height: Option<f64>, // reserved; per-row height (later)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CellWire {
    pub value: String,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub color: Option<String>, // CSS text color
    pub fill: Option<String>,  // CSS background
    pub align: Option<String>,
}

// Stage D result mirrors: engine enums are tuple-variant, so serialize
// through `{kind, ...}` mirrors. All coords emitted to JS are 0-based. ---

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HitTestWire {
    Cell { row: i32, col: i32 },
    RowHeader { row: i32 },
    ColumnHeader { col: i32 },
    Corner,
    Outside,
}

impl From<HitTest> for HitTestWire {
    fn from(h: HitTest) -> Self {
        match h {
            HitTest::Cell { row, column } => HitTestWire::Cell {
                row: row - 1,
                col: column - 1,
            },
            HitTest::AutofillHandle { row, column } => HitTestWire::Cell {
                row: row - 1,
                col: column - 1,
            },
            HitTest::RowHeader(r) => HitTestWire::RowHeader { row: r - 1 },
            HitTest::ColumnHeader(c) => HitTestWire::ColumnHeader { col: c - 1 },
            HitTest::Corner => HitTestWire::Corner,
            HitTest::FormulaRef { .. } | HitTest::Outside => HitTestWire::Outside,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResizeTargetWire {
    RowEdge { index: i32 },
    ColumnEdge { index: i32 },
}

impl From<ResizeTarget> for ResizeTargetWire {
    fn from(t: ResizeTarget) -> Self {
        match t {
            ResizeTarget::RowEdge(i) => ResizeTargetWire::RowEdge { index: i - 1 },
            ResizeTarget::ColumnEdge(i) => ResizeTargetWire::ColumnEdge { index: i - 1 },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortWire {
    pub column: usize,
    pub ascending: bool,
}

fn halign(s: &str) -> HAlign {
    match s {
        "left" => HAlign::Left,
        "center" => HAlign::Center,
        "right" => HAlign::Right,
        _ => HAlign::General,
    }
}

impl GridDataWire {
    pub fn into_model(self) -> DataGrid {
        let mut b = DataGrid::builder();
        if let Some(h) = self.row_height {
            b = b.default_row_height(h);
        }
        for c in self.columns {
            let mut col = Column::new(c.header);
            if let Some(w) = c.width {
                col = col.width(w);
            }
            if let Some(a) = c.align.as_deref() {
                col = col.align(halign(a));
            }
            b = b.column(col);
        }
        for r in self.rows {
            let cells = r
                .cells
                .into_iter()
                .map(|cw| Cell {
                    value: cw.value.clone(),
                    style: cell_style(&cw),
                })
                .collect();
            b = b.styled_row(cells);
        }
        b.build()
    }
}

// Returns `None` when a cell carries no styling so it falls back to the
// column default style in the model.
fn cell_style(c: &CellWire) -> Option<CellStyle> {
    let styled = c.bold.is_some()
        || c.italic.is_some()
        || c.color.is_some()
        || c.fill.is_some()
        || c.align.is_some();
    if !styled {
        return None;
    }
    let mut st = CellStyle::default();
    if let Some(true) = c.bold {
        st.font.bold = true;
    }
    if let Some(true) = c.italic {
        st.font.italic = true;
    }
    if let Some(col) = &c.color {
        st.font.color = Some(col.clone());
    }
    if let Some(f) = &c.fill {
        st.fill_color = Some(f.clone());
    }
    if let Some(a) = c.align.as_deref() {
        st.alignment
            .get_or_insert_with(Alignment::default)
            .horizontal = halign(a);
    }
    Some(st)
}
