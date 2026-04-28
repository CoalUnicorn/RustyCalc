//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component

use std::collections::HashMap;
use std::ops::RangeInclusive;

use ironcalc_base::types::{
    BorderItem, BorderStyle, CellType, HorizontalAlignment, Style, VerticalAlignment,
};
use web_sys::CanvasRenderingContext2d;

use crate::model::{CellAddress, CssColor, FormulaRef, RCRange, SheetArea};
use crate::renderer::{
    AutofillTarget, CanvasRenderer, VisibleRegion, MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH,
    THICK_BORDER_WIDTH,
};
use crate::style::FontStyle;
use crate::theme::CanvasTheme;
use crate::{CanvasModel, Point};

use super::geometry::{
    col_width, row_height, CanvasSize, FrozenRC, PixelRect, HEADER_COL_WIDTH, HEADER_OFFSET,
    HEADER_ROW_HEIGHT,
};

//  Shared axis - row-vs-column symmetry

/// Horizontal vs vertical axis.
///
/// Shared across viewport offset math (`cell_offset` dispatches on axis) and
/// header rect building (`Axis::header_rect`). Carries no payload - the
/// row/column index travels as a separate parameter so the same enum value
/// can be used across call sites that don't care about a specific index.
#[derive(Copy, Clone)]
pub(crate) enum Axis {
    Row,
    Column,
}

impl Axis {
    /// Rect that pins a header cell to the corresponding header strip.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for
    /// cols). The cross-axis extent is always the header strip width/height.
    pub(crate) fn header_rect(self, along: f64, height: f64) -> PixelRect {
        match self {
            Axis::Row => PixelRect {
                top_left: Point {
                    x: HEADER_OFFSET,
                    y: along,
                },
                width: HEADER_COL_WIDTH,
                height,
            },
            Axis::Column => PixelRect {
                top_left: Point {
                    x: along,
                    y: HEADER_OFFSET,
                },
                width: height,
                height: HEADER_ROW_HEIGHT,
            },
        }
    }

    /// Extent of the row/column at `index` on `sheet` (row height or column width).
    pub(crate) fn extent(self, model: &dyn CanvasModel, index: i32) -> f64 {
        match self {
            Axis::Row => row_height(model, index),
            Axis::Column => col_width(model, index),
        }
    }

    /// Pixel position where the header strip begins along this axis,
    /// offset by HEADER_OFFSET `0.5` for crisp integer-coordinate strokes.
    pub(crate) fn strip_start(self) -> f64 {
        match self {
            Axis::Row => HEADER_ROW_HEIGHT + HEADER_OFFSET,
            Axis::Column => HEADER_COL_WIDTH + HEADER_OFFSET,
        }
    }

    /// Visible scrollable band in this axis, drawn from `VisibleRegion`.
    pub(crate) fn visible_band(self, vis: &VisibleRegion) -> RangeInclusive<i32> {
        match self {
            Axis::Row => vis.first.row..=vis.last.row,
            Axis::Column => vis.first.column..=vis.last.column,
        }
    }

    /// Inclusive `(start, end)` of the user's selection along this axis,
    /// read from ironcalc's `SelectedView.range` array laid out as
    /// `[row1, col1, row2, col2]`. Rows live at indices 0/2; columns at 1/3.
    pub(crate) fn selection_range(self, view_range: &[i32; 4]) -> (i32, i32) {
        let (start, end) = match self {
            Axis::Row => (
                view_range[0].min(view_range[2]),
                view_range[0].max(view_range[2]),
            ),
            Axis::Column => (
                view_range[1].min(view_range[3]),
                view_range[1].max(view_range[3]),
            ),
        };
        (start, end)
    }
}

//  Pane rendering

/// Describes one of the four frozen-pane quadrants for `render_pane`.
///
/// Build with a named constructor so the quadrant name appears at the call site:
/// ```text
/// render_pane(model, sheet, &mut texts, PaneRegion::top_left(&frc));
/// render_pane(model, sheet, &mut texts, PaneRegion::bottom_right(&frc, &vis));
/// ```
#[derive(Clone)]
pub(crate) struct PaneRegion {
    pub rows: RangeInclusive<i32>,
    pub cols: RangeInclusive<i32>,
    pub origin: Point,
    /// Rightmost column that draws its right border.
    pub last_col: i32,
    /// Bottommost row that draws its bottom border.
    pub last_row: i32,
}

/// Outer edge of a cell rect that may be forced to stroke a border because
/// the cell sits on a pane boundary. Only `Right` and `Bottom` are valid -
/// left/top are inner edges resolved against neighbour cells inside the
/// pane.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum OuterEdge {
    Right,
    Bottom,
}

impl PaneRegion {
    /// Frozen rows x frozen cols - top-left quadrant.
    pub(crate) fn top_left(frc: &FrozenRC) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_row: *rows.end(),
            last_col: *cols.end(),
            rows,
            cols,
            origin: Point {
                x: HEADER_COL_WIDTH + HEADER_OFFSET,
                y: HEADER_ROW_HEIGHT + HEADER_OFFSET,
            },
        }
    }

    /// Frozen rows x scrollable cols - top-right quadrant.
    pub(crate) fn top_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_row: *rows.end(),
            rows,
            cols: vis.first.column..=vis.last.column,
            origin: Point {
                x: frc.offset.origin.x,
                y: HEADER_ROW_HEIGHT + HEADER_OFFSET,
            },
            last_col: vis.last.column,
        }
    }

    /// Scrollable rows x frozen cols - bottom-left quadrant.
    pub(crate) fn bottom_left(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_col: *cols.end(),
            rows: vis.first.row..=vis.last.row,
            cols,
            origin: Point {
                x: HEADER_COL_WIDTH + HEADER_OFFSET,
                y: frc.offset.origin.y,
            },
            last_row: vis.last.row,
        }
    }

    /// Scrollable rows x scrollable cols - main area.
    pub(crate) fn bottom_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        PaneRegion {
            rows: vis.first.row..=vis.last.row,
            cols: vis.first.column..=vis.last.column,
            origin: Point {
                x: frc.offset.origin.x,
                y: frc.offset.origin.y,
            },
            last_col: vis.last.column,
            last_row: vis.last.row,
        }
    }

    /// Outer borders this `(row, col)` must draw because it sits on a pane
    /// boundary. Empty slice for interior cells. Static slices - no
    /// allocation per cell.
    pub(crate) fn outer_edges_at(&self, row: i32, col: i32) -> &'static [OuterEdge] {
        match (col == self.last_col, row == self.last_row) {
            (true, true) => &[OuterEdge::Right, OuterEdge::Bottom],
            (true, false) => &[OuterEdge::Right],
            (false, true) => &[OuterEdge::Bottom],
            (false, false) => &[],
        }
    }

    /// Walk every visible cell in this pane, yielding pixel rect + outer
    /// edges per cell. Replaces the open-coded row/col iteration that used
    /// to live in `render_pane` / `render_pane_row`. The caller passes the
    /// canvas size so the walker can early-break past the canvas edge.
    pub(crate) fn cells<'a>(
        &'a self,
        model: &'a dyn CanvasModel,
        canvas: CanvasSize,
    ) -> PaneCells<'a> {
        PaneCells {
            pane: self,
            model,
            sheet: model.get_selected_sheet(),
            canvas,
            current_row: None,
            row_iter: self.rows.clone(),
            row_top: self.origin.y,
            col_iter: self.cols.clone(),
            col_left: self.origin.x,
        }
    }
}

pub(crate) struct CellPaint {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub bg: String, // CSS colour, always set
    pub borders: BordersPaint,
}

pub(crate) struct BordersPaint {
    pub left: BorderPaint,          // always drawn (grid or border)
    pub top: BorderPaint,           // always drawn
    pub right: Option<BorderPaint>, // only at pane edge or explicit border
    pub bottom: Option<BorderPaint>,
}

pub(crate) struct BorderPaint {
    pub color: String,
    pub stroke: BorderStroke, // local enum — see below
}

pub(crate) struct BorderStroke {
    pub width_px: f64,
    pub double: bool, // double-line styles render as two parallel strokes
}

pub(crate) struct TextPaint {
    pub clip: PixelRect,
    pub font_css: String,
    pub font_size_px: f64,
    pub color: String,
    pub underline: bool,
    pub strike: bool,
    pub lines: Vec<TextLine>, // existing struct from text.rs, moves to types.rs
}

/// One visual line of text inside a cell, positioned for center-aligned rendering.
pub struct TextLine {
    pub text: String,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
}

/// One cell yielded by a `PaneCells` walk: the address, its pixel rect at
/// the current scroll, and any outer pane-boundary borders the renderer
/// must force-draw on it.
#[derive(Clone, Copy)]
pub(crate) struct CellSlot {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub outer_edges: &'static [OuterEdge],
}

#[derive(Clone, Default)]
struct RowStrip {
    row: i32,
    height: f64,
}

/// Stateful walk over the cells of a `PaneRegion`. Caches per-pane column
/// widths once, threads a row-top accumulator across rows, and skips
/// hidden rows/columns as well as cells that fall off the canvas. Replaces
/// the parameter cluster that used to feed `render_pane_row`.
pub(crate) struct PaneCells<'a> {
    pane: &'a PaneRegion,
    model: &'a dyn CanvasModel,
    sheet: u32,
    canvas: CanvasSize,
    current_row: Option<RowStrip>,
    row_iter: RangeInclusive<i32>,
    row_top: f64,
    col_iter: RangeInclusive<i32>,
    col_left: f64,
}

impl<'a> Iterator for PaneCells<'a> {
    type Item = CellSlot;

    fn next(&mut self) -> Option<CellSlot> {
        loop {
            if self.current_row.is_none() {
                if self.row_top >= self.canvas.h {
                    return None;
                }
                let row = self.row_iter.next()?;
                let height = row_height(self.model, row);
                if height <= 0.0 {
                    continue;
                }
                self.current_row = Some(RowStrip { row, height });
                self.col_iter = self.pane.cols.clone();
                self.col_left = self.pane.origin.x;
            }
            let row_strip = self.current_row.clone().unwrap_or_default();

            let Some(col) = self.col_iter.next() else {
                self.row_top += row_strip.height;
                self.current_row = None;
                continue;
            };

            let width = col_width(self.model, col);
            if width <= 0.0 {
                continue;
            }
            if self.col_left >= self.canvas.w {
                self.row_top += row_strip.height;
                self.current_row = None;
                continue;
            }

            let rect = PixelRect {
                top_left: Point {
                    x: self.col_left,
                    y: self.row_top,
                },
                width,
                height: row_strip.height,
            };
            self.col_left += width;

            return Some(CellSlot {
                addr: CellAddress {
                    sheet: self.sheet,
                    row: row_strip.row,
                    column: col,
                },
                rect,
                outer_edges: self.pane.outer_edges_at(row_strip.row, col),
            });
        }
    }
}

//  Cell paint resolution
//
//  The iterator + free fns below turn raw IronCalc styles into renderer-ready
//  `CellPaint` data. Once resolved the renderer never queries the model.

const CELL_PADDING: f64 = 4.0;
const CHAR_WIDTH_FACTOR: f64 = 0.6;
const LINE_HEIGHT_FACTOR: f64 = 1.5;
const TEXT_V_INSET_PX: f64 = 4.0;

/// Below this rect width or height even a single glyph would overflow the cell;
/// text resolution short-circuits and yields no `TextPaint`.
const MIN_TEXT_DIM_PX: f64 = 10.0;

/// Translates IronCalc's `BorderStyle` enum into the renderer's local
/// `BorderStroke`. Verbatim mapping of today's `draw_border` switch
/// (cells.rs); dashed/dotted line patterns degrade to solid 1px in v1.
fn stroke_from_style(s: &BorderStyle) -> BorderStroke {
    match s {
        BorderStyle::Medium
        | BorderStyle::MediumDashed
        | BorderStyle::MediumDashDot
        | BorderStyle::MediumDashDotDot => BorderStroke {
            width_px: MEDIUM_BORDER_WIDTH,
            double: false,
        },
        BorderStyle::Thick => BorderStroke {
            width_px: THICK_BORDER_WIDTH,
            double: false,
        },
        BorderStyle::Double => BorderStroke {
            width_px: STANDARD_BORDER_WIDTH,
            double: true,
        },
        // Thin / Dotted / SlantDashDot / etc.
        _ => BorderStroke {
            width_px: STANDARD_BORDER_WIDTH,
            double: false,
        },
    }
}

/// Resolve a left/top edge - the only edges whose colour can be borrowed
/// from a neighbour's opposite border or fill. Encodes the fallback chain:
/// own -> neighbour-side -> own-bg -> neighbour-bg -> grid.
fn resolve_inner_paint(
    own: Option<&BorderItem>,
    neighbour_side: Option<&BorderItem>,
    neighbour_bg: Option<&str>,
    own_bg_set: bool,
    bg: &str,
    grid_color: &str,
) -> BorderPaint {
    if let Some(b) = own {
        return BorderPaint {
            color: b.color.as_deref().unwrap_or(grid_color).to_owned(),
            stroke: stroke_from_style(&b.style),
        };
    }
    if let Some(b) = neighbour_side {
        return BorderPaint {
            color: b.color.as_deref().unwrap_or(grid_color).to_owned(),
            stroke: stroke_from_style(&b.style),
        };
    }
    if own_bg_set {
        return BorderPaint {
            color: bg.to_owned(),
            stroke: stroke_from_style(&BorderStyle::Thin),
        };
    }
    if let Some(nbg) = neighbour_bg {
        return BorderPaint {
            color: nbg.to_owned(),
            stroke: stroke_from_style(&BorderStyle::Thin),
        };
    }
    BorderPaint {
        color: grid_color.to_owned(),
        stroke: stroke_from_style(&BorderStyle::Thin),
    }
}

/// Resolve a right/bottom edge - simpler than inner edges: only drawn
/// when explicit or at a pane boundary, no neighbour fallback.
fn resolve_outer_paint(own: Option<&BorderItem>, grid_color: &str) -> BorderPaint {
    if let Some(b) = own {
        BorderPaint {
            color: b.color.as_deref().unwrap_or(grid_color).to_owned(),
            stroke: stroke_from_style(&b.style),
        }
    } else {
        BorderPaint {
            color: grid_color.to_owned(),
            stroke: stroke_from_style(&BorderStyle::Thin),
        }
    }
}

/// Break `text` into render-ready lines: split on `\n` always, then word-wrap
/// within each split when `wrap` is on and the cell has width. `approx_char_w`
/// is the fallback glyph width when `measure_text` fails; biases the wrap
/// point slightly but never loses characters.
fn layout_lines(
    ctx: &CanvasRenderingContext2d,
    text: &str,
    wrap: bool,
    usable_w: f64,
    approx_char_w: f64,
) -> Vec<String> {
    if !wrap || usable_w <= 0.0 {
        return text.split('\n').map(str::to_owned).collect();
    }
    let mut result: Vec<String> = Vec::new();
    for raw_line in text.split('\n') {
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            let w = ctx
                .measure_text(&candidate)
                .map(|m| m.width())
                .unwrap_or(candidate.len() as f64 * approx_char_w);
            if w <= usable_w || current.is_empty() {
                current = candidate;
            } else {
                result.push(current);
                current = word.to_owned();
            }
        }
        result.push(current);
    }
    result
}

/// Per-cell text styling resolved from the model's raw `Style`. A private
/// step inside `resolve_text_paint`; not surfaced beyond `crate::types`.
struct CellStyle {
    text_color: String,
    font: FontStyle,
    h_align: HorizontalAlignment,
    v_align: VerticalAlignment,
    wrap_text: bool,
}

impl CellStyle {
    fn resolve(
        model: &dyn CanvasModel,
        sheet: u32,
        row: i32,
        column: i32,
        theme: &CanvasTheme,
    ) -> Self {
        let style = model.get_cell_style(sheet, row, column).unwrap_or_default();
        let cell_type = model
            .get_cell_type(sheet, row, column)
            .unwrap_or(CellType::Text);

        let text_color = match style.font.color.as_deref() {
            None | Some("#000000") => CssColor::new(theme.default_text_color),
            Some(c) => CssColor::new(c),
        };

        let size_px = style.font.sz as f64;
        // Fallback to default as in IronCalc Font name default.
        let css = FontStyle::build(
            size_px,
            style.font.b,
            style.font.i,
            &style.font.name,
            "Calibri",
        );
        let font = FontStyle {
            size_px,
            underline: style.font.u,
            strikethrough: style.font.strike,
            css,
        };

        let alignment = style.alignment.as_ref();
        let h_align = match alignment.map(|a| &a.horizontal) {
            Some(HorizontalAlignment::Right) => HorizontalAlignment::Right,
            Some(HorizontalAlignment::Center) | Some(HorizontalAlignment::CenterContinuous) => {
                HorizontalAlignment::Center
            }
            Some(HorizontalAlignment::Left) | Some(HorizontalAlignment::Fill) => {
                HorizontalAlignment::Left
            }
            // Canvas 2D has no justify/distributed - fall back to left.
            Some(HorizontalAlignment::Justify) | Some(HorizontalAlignment::Distributed) => {
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

        Self {
            text_color: text_color.0,
            font,
            h_align,
            v_align,
            wrap_text,
        }
    }
}

/// Build a `TextPaint` for `addr` at `rect`, or `None` for empty/too-small
/// cells. Reads the formatted value from the model and resolves font /
/// alignment / colour via `CellStyle`.
pub(crate) fn resolve_text_paint(
    renderer: &CanvasRenderer,
    model: &dyn CanvasModel,
    addr: CellAddress,
    rect: PixelRect,
) -> Option<TextPaint> {
    let text = model
        .get_formatted_cell_value(addr.sheet, addr.row, addr.column)
        .ok()?;
    if text.is_empty() {
        return None;
    }
    if rect.width < MIN_TEXT_DIM_PX || rect.height < MIN_TEXT_DIM_PX {
        return None;
    }

    // Destructure to move fields directly - avoids cloning `css`.
    let CellStyle {
        font:
            FontStyle {
                css: font_css,
                size_px,
                underline,
                strikethrough: strike,
                ..
            },
        text_color,
        h_align,
        v_align,
        wrap_text,
        ..
    } = CellStyle::resolve(model, addr.sheet, addr.row, addr.column, renderer.theme());

    let approx_char_w = size_px * CHAR_WIDTH_FACTOR;
    let line_height = size_px * LINE_HEIGHT_FACTOR;
    let usable_w = rect.width - 2.0 * CELL_PADDING;
    let right = rect.right();
    let bottom = rect.bottom();
    let center = rect.center();

    // Set font on ctx so measure_text() returns accurate widths.
    let ctx = renderer.ctx_ref();
    ctx.set_font(&font_css);

    let text_lines = layout_lines(ctx, &text, wrap_text, usable_w, approx_char_w);

    let line_count = text_lines.len() as f64;
    let mut lines: Vec<TextLine> = Vec::new();

    for (i, line) in text_lines.into_iter().enumerate() {
        let tw = ctx
            .measure_text(&line)
            .map(|m| m.width())
            .unwrap_or(line.len() as f64 * approx_char_w);
        let i_f = i as f64;
        let center_x = match h_align {
            HorizontalAlignment::Right => right - CELL_PADDING - tw / 2.0,
            HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => center.x,
            _ => rect.top_left.x + CELL_PADDING + tw / 2.0,
        };
        let center_y = match v_align {
            VerticalAlignment::Bottom => {
                bottom - size_px / 2.0 - TEXT_V_INSET_PX + (i_f - line_count + 1.0) * line_height
            }
            VerticalAlignment::Center => center.y + (i_f + (1.0 - line_count) / 2.0) * line_height,
            _ => rect.top_left.y + size_px / 2.0 + TEXT_V_INSET_PX + i_f * line_height,
        };
        lines.push(TextLine {
            text: line,
            center_x,
            center_y,
            width: tw,
        });
    }

    Some(TextPaint {
        clip: rect,
        font_css,
        font_size_px: size_px,
        color: CssColor::new(&text_color).0,
        underline,
        strike,
        lines,
    })
}

/// Resolve one cell into renderer-ready `CellPaint`. Combines the
/// background fill, four border edges (with neighbour fallback for
/// left/top), and optional text layout. Infallible on the style-fetch
/// path: neighbour styles are passed in by the iterator.
pub(crate) fn resolve_cell_paint(
    renderer: &CanvasRenderer,
    show_grid: bool,
    slot: CellSlot,
    own_style: &Style,
    left_neighbour: Option<&Style>,
    top_neighbour: Option<&Style>,
) -> Option<CellPaint> {
    if slot.rect.width <= 0.0 || slot.rect.height <= 0.0 {
        return None;
    }

    let theme = renderer.theme();
    let bg = own_style
        .fill
        .fg_color
        .as_deref()
        .unwrap_or(theme.cell_bg)
        .to_owned();
    let grid_color = if show_grid {
        theme.grid_color
    } else {
        bg.as_str()
    };
    let own_bg_set = own_style.fill.fg_color.is_some();

    let left = resolve_inner_paint(
        own_style.border.left.as_ref(),
        left_neighbour.and_then(|n| n.border.right.as_ref()),
        left_neighbour.and_then(|n| n.fill.fg_color.as_deref()),
        own_bg_set,
        &bg,
        grid_color,
    );
    let top = resolve_inner_paint(
        own_style.border.top.as_ref(),
        top_neighbour.and_then(|n| n.border.bottom.as_ref()),
        top_neighbour.and_then(|n| n.fill.fg_color.as_deref()),
        own_bg_set,
        &bg,
        grid_color,
    );

    let right = if slot.outer_edges.contains(&OuterEdge::Right) || own_style.border.right.is_some()
    {
        Some(resolve_outer_paint(
            own_style.border.right.as_ref(),
            grid_color,
        ))
    } else {
        None
    };
    let bottom =
        if slot.outer_edges.contains(&OuterEdge::Bottom) || own_style.border.bottom.is_some() {
            Some(resolve_outer_paint(
                own_style.border.bottom.as_ref(),
                grid_color,
            ))
        } else {
            None
        };

    Some(CellPaint {
        addr: slot.addr,
        rect: slot.rect,
        bg,
        borders: BordersPaint {
            left,
            top,
            right,
            bottom,
        },
    })
}

/// Iterator decorator over `PaneCells` that yields fully resolved
/// `CellPaint` per visible cell. Threads previous-row/column styles for
/// inner-edge neighbour fallback so the renderer only paints, never
/// queries the model.
pub(crate) struct CellPaintsIter<'a> {
    slots: PaneCells<'a>,
    renderer: &'a CanvasRenderer,
    model: &'a dyn CanvasModel,
    show_grid: bool,
    /// Previous row's style for each column we've yielded - keyed by
    /// column index. Lookup feeds the **top** edge's fallback chain.
    prev_row_styles: HashMap<i32, Style>,
    /// Previous column's style in the **current** row. Resets to `None`
    /// when the row changes. Feeds the **left** edge's fallback chain.
    prev_col_style: Option<Style>,
    /// Last row we yielded; row change resets `prev_col_style`.
    last_row: Option<i32>,
}

impl<'a> CellPaintsIter<'a> {
    /// Build the resolved-paint stream for one pane. `show_grid` is read
    /// **once** here from the model and cached - replaces today's
    /// per-cell `get_show_grid_lines` call inside `render_cell_style`.
    pub(crate) fn new(
        renderer: &'a CanvasRenderer,
        model: &'a dyn CanvasModel,
        pane: &'a PaneRegion,
    ) -> Self {
        let sheet = model.get_selected_sheet();
        let show_grid = model.get_show_grid_lines(sheet).unwrap_or(true);
        Self {
            slots: pane.cells(model, renderer.canvas_size()),
            renderer,
            model,
            show_grid,
            prev_row_styles: HashMap::new(),
            prev_col_style: None,
            last_row: None,
        }
    }
}

impl<'a> Iterator for CellPaintsIter<'a> {
    type Item = CellPaint;

    fn next(&mut self) -> Option<CellPaint> {
        loop {
            let slot = self.slots.next()?;

            if Some(slot.addr.row) != self.last_row {
                self.prev_col_style = None;
                self.last_row = Some(slot.addr.row);
            }

            let Ok(own_style) =
                self.model
                    .get_cell_style(slot.addr.sheet, slot.addr.row, slot.addr.column)
            else {
                // Style fetch failed - skip this cell entirely (matches
                // today's `render_cell_style` early-return at cells.rs).
                continue;
            };

            let paint = resolve_cell_paint(
                self.renderer,
                self.show_grid,
                slot,
                &own_style,
                self.prev_col_style.as_ref(),
                self.prev_row_styles.get(&slot.addr.column),
            );

            // Cache for the next cell's neighbour lookups. Row cache
            // needs a clone so the column cache can move-take the value.
            self.prev_row_styles
                .insert(slot.addr.column, own_style.clone());
            self.prev_col_style = Some(own_style);

            if let Some(p) = paint {
                return Some(p);
            }
            // Degenerate rect - resolver returned None; advance to next slot.
        }
    }
}

/// Overlay ranges passed to `render()` for selection preview drawing.
#[derive(Clone, PartialEq, Default)]
pub struct RenderOverlays {
    /// Selection border in CSS pixels, pre-converted by the consumer.
    /// `None` means no selection is visible (e.g., during a sheet swap).
    pub selection: Option<super::geometry::PixelRect>,
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<RCRange>,
    /// All formula refs extracted from the current formula (multi-color overlays).
    pub formula_refs: Vec<FormulaRef>,
}

/// Hint to the canvas renderer about the minimum work needed for this repaint.
///
/// Currently `CanvasRenderer::render` treats all modes identically.
/// The enum is in place so future optimisations (skip layout recalc for
/// `FormatOnly`, skip cell-text for `ViewportUpdate`) can be added
/// without another architectural change.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CanvasRenderMode {
    /// Content or structure changed - repaint all cells (default).
    #[default]
    Full,
    /// Only formatting changed - repaint without model recalculation.
    FormatOnly,
    /// Navigation only - update selection box and scroll position.
    ViewportUpdate,
    /// Drag overlay changed (autofill preview, point-mode range) - no model change.
    Overlay,
}

/// Scroll origin for the visible sheet area.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Viewport {
    /// First visible row in the scrollable region (1-indexed).
    pub top_row: u32,
    /// First visible column in the scrollable region (1-indexed).
    pub left_column: u32,
}

/// Number of rows/columns pinned by the freeze-panes feature.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FreezeConfig {
    pub frozen_rows: u32,
    pub frozen_cols: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_header_rect_pins_x_to_left_strip() {
        let rect = Axis::Row.header_rect(100.0, 20.0);
        assert_eq!(rect.top_left.x, HEADER_OFFSET);
        assert_eq!(rect.top_left.y, 100.0);
        assert_eq!(rect.width, HEADER_COL_WIDTH);
        assert_eq!(rect.height, 20.0);
    }

    #[test]
    fn column_header_rect_pins_y_to_top_strip() {
        let rect = Axis::Column.header_rect(100.0, 20.0);
        assert_eq!(rect.top_left.x, 100.0);
        assert_eq!(rect.top_left.y, HEADER_OFFSET);
        assert_eq!(rect.width, 20.0);
        assert_eq!(rect.height, HEADER_ROW_HEIGHT);
    }

    #[test]
    fn row_header_rect_thickness_maps_to_height() {
        let rect = Axis::Row.header_rect(100.0, 50.0);
        assert_eq!(rect.height, 50.0);
    }

    #[test]
    fn column_header_rect_thickness_maps_to_width() {
        let rect = Axis::Column.header_rect(100.0, 50.0);
        assert_eq!(rect.width, 50.0);
    }

    #[test]
    fn row_strip_start_is_below_top_header() {
        assert_eq!(Axis::Row.strip_start(), HEADER_ROW_HEIGHT + HEADER_OFFSET);
    }

    #[test]
    fn column_strip_start_is_right_of_left_header() {
        assert_eq!(Axis::Column.strip_start(), HEADER_COL_WIDTH + HEADER_OFFSET);
    }

    fn vis(rows: (i32, i32), cols: (i32, i32)) -> crate::renderer::VisibleRegion {
        crate::renderer::VisibleRegion {
            first: crate::geometry::CellRC {
                row: rows.0,
                column: cols.0,
            },
            last: crate::geometry::CellRC {
                row: rows.1,
                column: cols.1,
            },
        }
    }

    #[test]
    fn row_visible_band_uses_first_last_row() {
        let v = vis((3, 17), (5, 12));
        let band = Axis::Row.visible_band(&v);
        assert_eq!(*band.start(), 3);
        assert_eq!(*band.end(), 17);
    }

    #[test]
    fn column_visible_band_uses_first_last_column() {
        let v = vis((3, 17), (5, 12));
        let band = Axis::Column.visible_band(&v);
        assert_eq!(*band.start(), 5);
        assert_eq!(*band.end(), 12);
    }

    fn frozen(rows: Option<(i32, i32)>, cols: Option<(i32, i32)>, origin: Point) -> FrozenRC {
        FrozenRC {
            row_band: rows.map(|(s, e)| s..=e),
            col_band: cols.map(|(s, e)| s..=e),
            offset: crate::geometry::FrozenOffset { origin },
        }
    }

    #[test]
    fn pane_top_left_origin_is_pinned_to_header_corner() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let p = PaneRegion::top_left(&frc);
        assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
        assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
        assert_eq!(p.last_row, 2);
        assert_eq!(p.last_col, 3);
    }

    #[test]
    fn pane_top_right_origin_uses_frozen_x_and_header_y() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let v = vis((3, 9), (4, 11));
        let p = PaneRegion::top_right(&frc, &v);
        assert_eq!(p.origin.x, 200.0);
        assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
        assert_eq!(*p.cols.start(), 4);
        assert_eq!(p.last_col, 11);
    }

    #[test]
    fn pane_bottom_left_origin_uses_header_x_and_frozen_y() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let v = vis((3, 9), (4, 11));
        let p = PaneRegion::bottom_left(&frc, &v);
        assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
        assert_eq!(p.origin.y, 100.0);
        assert_eq!(*p.rows.start(), 3);
        assert_eq!(p.last_row, 9);
    }

    #[test]
    fn pane_bottom_right_origin_matches_frozen_offset() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let v = vis((3, 9), (4, 11));
        let p = PaneRegion::bottom_right(&frc, &v);
        assert_eq!(p.origin.x, 200.0);
        assert_eq!(p.origin.y, 100.0);
        assert_eq!(p.last_row, 9);
        assert_eq!(p.last_col, 11);
    }

    #[test]
    fn viewport_same_value_is_equal() {
        let vp = Viewport {
            top_row: 5,
            left_column: 3,
        };
        assert_eq!(vp, vp);
    }

    #[test]
    fn viewport_different_top_row_is_not_equal() {
        let a = Viewport {
            top_row: 1,
            left_column: 1,
        };
        let b = Viewport {
            top_row: 2,
            left_column: 1,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn viewport_different_left_column_is_not_equal() {
        let a = Viewport {
            top_row: 1,
            left_column: 1,
        };
        let b = Viewport {
            top_row: 1,
            left_column: 2,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn freeze_config_same_value_is_equal() {
        let f = FreezeConfig {
            frozen_rows: 2,
            frozen_cols: 1,
        };
        assert_eq!(f, f);
    }

    #[test]
    fn freeze_config_different_rows_is_not_equal() {
        let a = FreezeConfig {
            frozen_rows: 2,
            frozen_cols: 1,
        };
        let b = FreezeConfig {
            frozen_rows: 3,
            frozen_cols: 1,
        };
        assert_ne!(a, b);
    }

    #[test]
    fn render_overlays_default_equals_itself() {
        let a = RenderOverlays::default();
        let b = RenderOverlays::default();
        assert!(a == b);
    }

    #[test]
    fn render_overlays_changed_point_range_is_not_equal() {
        use crate::model::RCRange;
        let a = RenderOverlays::default();
        let mut b = RenderOverlays::default();
        b.point_range = Some(RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        });
        assert!(a != b);
    }

    #[test]
    fn render_overlays_same_point_range_is_equal() {
        use crate::model::RCRange;
        let range = Some(RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        });
        let a = RenderOverlays {
            point_range: range,
            ..Default::default()
        };
        let b = RenderOverlays {
            point_range: range,
            ..Default::default()
        };
        assert!(a == b);
    }

    #[test]
    fn render_overlays_changed_selection_is_not_equal() {
        use crate::geometry::{PixelRect, Point};
        let a = RenderOverlays::default();
        let b = RenderOverlays {
            selection: Some(PixelRect {
                top_left: Point { x: 10.0, y: 10.0 },
                width: 80.0,
                height: 20.0,
            }),
            ..Default::default()
        };
        assert!(a != b);
    }

    #[test]
    fn render_overlays_cleared_selection_equals_default() {
        let a = RenderOverlays::default();
        let b = RenderOverlays {
            selection: None,
            ..Default::default()
        };
        assert!(a == b);
    }
}
