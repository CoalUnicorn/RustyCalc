//! Cell painting - pure pixel pusher.
//!
//! `render_pane` iterates the resolved-paint stream from `CellPaintsIter`
//! and hands each `CellPaint` to `paint_cell`. Nothing in this file talks
//! to the model: bg, borders, and text are pre-resolved upstream.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use crate::model::CssColor;
use crate::renderer::pane::{OuterEdge, PaneRegion};
use crate::style::FontStyle;
use crate::theme::CanvasTheme;
use crate::{col_width, row_height, CanvasModel, CanvasSize, Point, Span};

use super::super::geometry::{BorderEdge, FrameContext, Line, PixelRect};
use super::super::model::{CellAddress, RCRange};
// use super::super::types::{resolve_cell_paint, resolve_text_paint, BorderPaint, CellPaint};
use super::CanvasRenderer;
use crate::renderer::{MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

use ironcalc_base::types::{
    BorderItem, BorderStyle, CellType, HorizontalAlignment, Style, VerticalAlignment,
};
use web_sys::CanvasRenderingContext2d;
/// Pane boundary edges to force-stroke around the active cell when it is
/// repainted on top of the selection overlay.
const ACTIVE_CELL_OUTER_EDGES: &[OuterEdge] = &[OuterEdge::Right, OuterEdge::Bottom];

impl CanvasRenderer {
    /// Walk one frozen-pane quadrant. Pass 1 paints bg+borders for every
    /// cell, then pass 2 paints text on top so overflow is never clipped by
    /// a neighbour's background.
    pub(super) fn render_pane(&self, model: &dyn CanvasModel, pane: PaneRegion) {
        let paints: Vec<CellStyle> = self.paints_in(model, &pane).collect();
        for p in &paints {
            self.paint_bg(p);
        }
        self.paint_borders_batched(&paints);
        self.paint_pane_text(model, &paints);
    }

    /// Fill a cell's background rectangle. Border pass is separate (batched).
    pub(super) fn paint_bg(&self, p: &CellStyle) {
        self.set_fill_cached(&p.bg);
        self.ctx_ref().fill_rect(
            p.rect.top_left.x,
            p.rect.top_left.y,
            p.rect.width,
            p.rect.height,
        );
    }

    /// Paint bg + borders for one resolved `CellPaint`. Used by
    /// `repaint_active_cell` where a single-cell batch is not worth the
    /// overhead; the main pane pass uses `paint_bg` + `paint_borders_batched`.
    pub(super) fn paint_cell(&self, p: &CellStyle) {
        self.paint_bg(p);
        self.paint_border(BorderEdge::Left, p.rect, &p.borders.left);
        self.paint_border(BorderEdge::Top, p.rect, &p.borders.top);
        if let Some(b) = &p.borders.right {
            self.paint_border(BorderEdge::Right, p.rect, b);
        }
        if let Some(b) = &p.borders.bottom {
            self.paint_border(BorderEdge::Bottom, p.rect, b);
        }
    }

    /// Pass 2: resolve and paint text for every cell in a collected pane.
    fn paint_pane_text(&self, model: &dyn CanvasModel, paints: &[CellStyle]) {
        // TEST: paints remove later
        // Damage tracking to skip cells
        // let mut c = 0;
        for p in paints {
            if let Some(t) = resolve_text_paint(self, model, p.addr, p.rect) {
                self.paint_text(&t);
                // c += 1;
            }
        }
        // web_sys::console::log_1(&format!("Paint painted texts: {}", c).into());
    }

    /// Batch all border lines in `paints` into per-style paths so each
    /// distinct (color, width) combination emits exactly one `begin_path` →
    /// N×`move_to`/`line_to` → `stroke()` sequence instead of one per edge.
    ///
    /// Linear Vec search is fastest here: a pane typically has 2–5 distinct
    /// border styles (grid gray + maybe 1–2 user-set colors).
    fn paint_borders_batched(&self, paints: &[CellStyle]) {
        struct Bucket {
            color: String,
            width_px: f64,
            lines: Vec<Line>,
        }

        let mut buckets: Vec<Bucket> = Vec::new();

        for p in paints {
            let edges: [Option<(BorderEdge, &BorderPaint)>; 4] = [
                Some((BorderEdge::Left, &p.borders.left)),
                Some((BorderEdge::Top, &p.borders.top)),
                p.borders.right.as_ref().map(|b| (BorderEdge::Right, b)),
                p.borders.bottom.as_ref().map(|b| (BorderEdge::Bottom, b)),
            ];
            for (edge, border) in edges.into_iter().flatten() {
                let base = edge.line(p.rect);
                // Double borders emit two offset lines into the same bucket.
                let double_lines = [base.offset_cross(-1.0), base.offset_cross(1.0)];
                let single_line = [base];
                let lines: &[Line] = if border.stroke.double {
                    &double_lines
                } else {
                    &single_line
                };
                for &line in lines {
                    if let Some(b) = buckets
                        .iter_mut()
                        .find(|b| b.color == border.color && b.width_px == border.stroke.width_px)
                    {
                        b.lines.push(line);
                    } else {
                        buckets.push(Bucket {
                            color: border.color.clone(),
                            width_px: border.stroke.width_px,
                            lines: vec![line],
                        });
                    }
                }
            }
        }

        for bucket in &buckets {
            self.set_stroke_cached(&bucket.color);
            self.set_line_width_cached(bucket.width_px);
            self.ctx_ref().begin_path();
            for line in &bucket.lines {
                match line {
                    Line::H { span, y } => {
                        self.ctx_ref().move_to(span.from, *y);
                        self.ctx_ref().line_to(span.to, *y);
                    }
                    Line::V { x, span } => {
                        self.ctx_ref().move_to(*x, span.from);
                        self.ctx_ref().line_to(*x, span.to);
                    }
                }
            }
            self.ctx_ref().stroke();
        }
    }

    /// Stroke one resolved border. `Double`-style borders render as two
    /// parallel strokes offset ±1px on the cross-axis.
    fn paint_border(&self, edge: BorderEdge, rect: PixelRect, b: &BorderPaint) {
        let line = edge.line(rect);
        let offsets: &[f64] = if b.stroke.double {
            &[-1.0, 1.0]
        } else {
            &[0.0]
        };
        self.set_stroke_cached(&b.color);
        self.set_line_width_cached(b.stroke.width_px);
        for &d in offsets {
            self.stroke_line(line.offset_cross(d));
        }
    }

    /// Repaint one cell's full paint (bg + borders + text).
    ///
    /// Used by the selection overlay to restore the active cell on top of
    /// the semi-transparent selection fill. Resolves the paint inline -
    /// neighbour styles are skipped (passed as `None`); the visual
    /// difference is imperceptible at a single cell boundary.
    pub(super) fn repaint_active_cell(
        &self,
        model: &dyn CanvasModel,
        addr: CellAddress,
        frame: &FrameContext,
    ) {
        let range = RCRange::from_cell(addr.row, addr.column);
        let Some(rect) = self.range_pixel_bounds(model, frame, range) else {
            return;
        };
        let Ok(own_style) = model.get_cell_style(addr.sheet, addr.row, addr.column) else {
            return;
        };
        let show_grid = model.get_show_grid_lines(addr.sheet).unwrap_or(true);
        let Some(paint) = resolve_cell_paint(
            self,
            show_grid,
            CellSlot {
                addr,
                rect,
                outer_edges: ACTIVE_CELL_OUTER_EDGES,
            },
            &own_style,
            None,
            None,
        ) else {
            return;
        };
        self.paint_cell(&paint);
        if let Some(t) = resolve_text_paint(self, model, addr, rect) {
            self.paint_text(&t);
        }
    }
}

pub(crate) struct CellStyle {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub bg: String, // CSS colour, always set
    pub borders: BordersPaint,
}

impl CellStyle {
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
    ) -> Option<CellStyle> {
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

        let right =
            if slot.outer_edges.contains(&OuterEdge::Right) || own_style.border.right.is_some() {
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

        Some(Self {
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
pub struct CellSlot {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub outer_edges: &'static [OuterEdge],
}

#[derive(Clone, Default)]
pub struct RowStrip {
    row: i32,
    height: f64,
}

/// Stateful walk over the cells of a `PaneRegion`. Caches per-pane column
/// widths once, threads a row-top accumulator across rows, and skips
/// hidden rows/columns as well as cells that fall off the canvas. Replaces
/// the parameter cluster that used to feed `render_pane_row`.
pub struct PaneCells<'a> {
    pub pane: &'a PaneRegion,
    pub model: &'a dyn CanvasModel,
    pub sheet: u32,
    pub canvas: CanvasSize,
    pub current_row: Option<RowStrip>,
    pub row_iter: RangeInclusive<i32>,
    pub row_top: f64,
    pub col_iter: RangeInclusive<i32>,
    pub col_left: f64,
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
    type Item = CellStyle;

    fn next(&mut self) -> Option<CellStyle> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_edge_is_vertical_line_at_rect_x() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Left.line(rect),
            Line::V {
                x: 5.0,
                span: Span {
                    from: 10.0,
                    to: 25.0,
                }
            }
        );
    }

    #[test]
    fn right_edge_is_vertical_line_at_rect_right() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Right.line(rect),
            Line::V {
                x: 25.0,
                span: Span {
                    from: 10.0,
                    to: 25.0,
                },
            }
        )
    }

    #[test]
    fn top_edge_is_horizontal_line_at_rect_y() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Top.line(rect),
            Line::H {
                y: 10.0,
                span: Span {
                    from: 5.0,
                    to: 25.0,
                },
            }
        );
    }

    #[test]
    fn bottom_edge_is_horizontal_line_at_rect_bottom() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Bottom.line(rect),
            Line::H {
                y: 25.0,
                span: Span {
                    from: 5.0,
                    to: 25.0,
                },
            }
        );
    }
}
