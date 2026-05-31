//! Pure auto-fit measurement over a used-range span.
//!
//! `fit_width` scans a column across an explicit `[first_row, last_row]`
//! span (the sheet's used-row range, supplied by the consumer) and returns
//! the widest formatted value plus padding; `fit_height` scans a row across
//! a `[first_col, last_col]` span for the tallest font. This is Excel-style
//! "fit the whole column", not just the painted viewport — the caller owns
//! the range because the used-range query lives a layer above this one.
//!
//! The span is bounded by `capped_range` to `FIT_SCAN_CAP` entries so a
//! pathological used-range can't stall the canvas. No mutation: the returned
//! extent is what the consumer would apply via a resize.
//!
//! Font CSS is rebuilt here via `renderer::cache::font::escape_font_family`
//! — the same function the renderer's text pass calls through `FontIntern` —
//! so measured widths are guaranteed to match painted widths for any font
//! name, including multi-word families like "Times New Roman".

use crate::CanvasModel;
use crate::painter::TextMetrics;
use crate::renderer::cache::font::escape_font_family;

use ironcalc_base::types::Style;

/// Slack added to the measured content extent so glyphs don't touch the
/// cell border. Mirrors the cell text pass's `CELL_PADDING` on both sides.
pub const FIT_PADDING: f64 = 8.0;

/// Floor so an auto-fit never collapses a column/row to an unusable sliver.
const MIN_EXTENT: f64 = 5.0;

/// Upper bound on cells scanned per auto-fit. A column/row whose used range
/// exceeds this is measured over a bounded window so one double-click can't
/// freeze the canvas re-measuring hundreds of thousands of cells.
pub const FIT_SCAN_CAP: i32 = 20_000;

/// Bound a used-range span `[first, last]` (inclusive) to at most
/// `FIT_SCAN_CAP` entries before scanning. Anchored at `first` (top-down,
/// Excel-style): when the span is too long, the rows/cols nearest the start
/// are kept. An inverted span (`first > last`) is returned unchanged so the
/// caller's `first..=last` loop runs zero times.
fn capped_range(first: i32, last: i32) -> (i32, i32) {
    if first > last {
        return (first, last);
    }
    // Inclusive span: a cap of N spans `first ..= first + N - 1`. Saturating
    // add keeps a near-`i32::MAX` start from wrapping.
    let cap_last = first.saturating_add(FIT_SCAN_CAP - 1);
    (first, last.min(cap_last))
}

/// `ctx.font` string for a cell's style.
///
/// Delegates family quoting to `renderer::cache::font::escape_font_family`
/// — the same function the text pass calls through `FontIntern` — so the
/// string produced here is identical to the one the renderer paints with.
/// The fallback `"Calibri"` matches the literal passed at `text.rs:138`.
pub fn font_css(style: &Style) -> String {
    let size_px = f64::from(style.font.sz);
    let weight = if style.font.b { "bold " } else { "" };
    let slant = if style.font.i { "italic " } else { "" };
    let family = escape_font_family(&style.font.name, "Calibri");
    format!("{weight}{slant}{size_px}px {family}")
}

/// Widest formatted value across `col` over the `[first_row, last_row]`
/// used-row span, plus padding. `None` when every scanned cell in `col` is
/// empty (nothing to fit to).
pub fn fit_width(
    model: &dyn CanvasModel,
    metrics: &dyn TextMetrics,
    col: i32,
    first_row: i32,
    last_row: i32,
) -> Option<f64> {
    let sheet = model.get_selected_sheet();
    let (first, last) = capped_range(first_row, last_row);

    let mut max = 0.0_f64;
    for row in first..=last {
        let Some(text) = model.get_formatted_cell_value(sheet, row, col) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let css = match model.get_cell_style(sheet, row, col) {
            Some(style) => font_css(&style),
            None => "12px sans-serif".to_owned(),
        };
        let w = metrics.measure_text_width(&text, &css);
        if w > max {
            max = w;
        }
    }

    if max <= 0.0 {
        return None;
    }
    Some((max + FIT_PADDING).max(MIN_EXTENT))
}

/// Tallest font size across `row` over the `[first_col, last_col]`
/// used-column span, plus padding. Single line, so height derives from the
/// largest non-empty cell's font size. `None` when every scanned cell in
/// `row` is empty. `metrics` is kept for signature symmetry with `fit_width`.
pub fn fit_height(
    model: &dyn CanvasModel,
    metrics: &dyn TextMetrics,
    row: i32,
    first_col: i32,
    last_col: i32,
) -> Option<f64> {
    let _ = metrics;
    let sheet = model.get_selected_sheet();
    let (first, last) = capped_range(first_col, last_col);

    let mut max_size = 0.0_f64;
    for col in first..=last {
        let Some(text) = model.get_formatted_cell_value(sheet, row, col) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let size = match model.get_cell_style(sheet, row, col) {
            Some(style) => f64::from(style.font.sz),
            None => 12.0,
        };
        if size > max_size {
            max_size = size;
        }
    }

    if max_size <= 0.0 {
        return None;
    }
    Some((max_size + FIT_PADDING).max(MIN_EXTENT))
}
