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
use crate::RCRange;
use crate::painter::{CHAR_WIDTH_FACTOR, TextMetrics};
use crate::renderer::cache::font::escape_font_family;
use crate::renderer::cell::text::{CELL_PADDING, LINE_HEIGHT_FACTOR};
use crate::renderer::{TextLine, layout_into};

use crate::style::CellStyle;

/// Slack added to the measured content extent so glyphs don't touch the
/// cell border. Derived from the cell text pass's `CELL_PADDING` on both
/// sides so the two stay locked together — change `CELL_PADDING` and autofit
/// follows automatically.
pub const FIT_PADDING: f64 = 2.0 * CELL_PADDING;

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
pub fn font_css(style: &CellStyle) -> String {
    let size_px = style.font.size;
    let weight = if style.font.bold { "bold " } else { "" };
    let slant = if style.font.italic { "italic " } else { "" };
    let family = escape_font_family(&style.font.name, "Calibri");
    format!("{weight}{slant}{size_px}px {family}")
}

/// Widest formatted value across `col` over the `[first_row, last_row]`
/// used-row span, plus padding. `None` when every scanned cell in `col` is
/// empty (nothing to fit to), or when the selected-sheet read fails (an
/// out-of-frame query API, so it propagates the bridge failure with `?`
/// rather than holding a paint attempt).
pub fn fit_width(
    model: &dyn CanvasModel,
    metrics: &dyn TextMetrics,
    col: i32,
    first_row: i32,
    last_row: i32,
) -> Option<f64> {
    let sheet = model.get_selected_sheet()?;
    let (first, last) = capped_range(first_row, last_row);

    let span = RCRange {
        r1: first,
        c1: col,
        r2: last,
        c2: col,
    };

    let mut max = 0.0_f64;
    for (r, col) in span.cells() {
        let Some(text) = model.get_formatted_cell_value(sheet, r, col).value() else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let css = match model.get_cell_style(sheet, r, col).value() {
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

/// Tallest text block across `row` over the `[first_col, last_col]`
/// used-column span, plus padding. Multi-line aware: each cell's wrapped line
/// count comes from the renderer's own [`layout_into`], so the fitted height
/// matches what the painter stacks (the module's measured-==-painted invariant).
/// `None` when every scanned cell in `row` is empty, or when the
/// selected-sheet read fails (see [`fit_width`]'s doc on propagating rather
/// than holding).
pub fn fit_height(
    model: &dyn CanvasModel,
    metrics: &dyn TextMetrics,
    row: i32,
    first_col: i32,
    last_col: i32,
) -> Option<f64> {
    let sheet = model.get_selected_sheet()?;
    let (first, last) = capped_range(first_col, last_col);
    let span = RCRange {
        r1: row,
        c1: first,
        r2: row,
        c2: last,
    };

    // One-shot measurement (not the per-frame hot path), so plain local
    // scratch buffers are fine — no need for the renderer's slot-reuse dance.
    let mut lines: Vec<TextLine> = Vec::new();
    let mut wrap_buf = String::new();

    let mut max_height = 0.0_f64;
    for (r, col) in span.cells() {
        let Some(text) = model.get_formatted_cell_value(sheet, r, col).value() else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let style = model.get_cell_style(sheet, r, col).value();
        let size_px = style.as_ref().map_or(12.0, |s| s.font.size);
        let css = style
            .as_ref()
            .map_or_else(|| "12px sans-serif".to_owned(), font_css);
        let wrap = style
            .as_ref()
            .is_some_and(|s| s.alignment.as_ref().is_some_and(|a| a.wrap_text));
        // Soft-wrap needs the cell's usable inner width; `0.0` (no column
        // width) makes `layout_into` split on '\n' only, never soft-wrap.
        let usable_w = model
            .get_column_width(sheet, col)
            .map_or(0.0, |w| w - 2.0 * CELL_PADDING);

        // Reuse the painter's exact split + wrap so the line count we measure
        // is the line count that gets drawn.
        layout_into(
            metrics,
            &css,
            &text,
            wrap,
            usable_w,
            size_px * CHAR_WIDTH_FACTOR,
            &mut lines,
            &mut wrap_buf,
        );

        let line_height = size_px * LINE_HEIGHT_FACTOR;
        let cell_height = line_block_height(lines.len(), size_px, line_height);
        if cell_height > max_height {
            max_height = cell_height;
        }
    }

    if max_height <= 0.0 {
        return None;
    }
    Some(max_height.max(MIN_EXTENT))
}

/// Fitted pixel height of one cell's text block.
///
/// `line_count` is the number of wrapped lines from [`layout_into`]; `size_px`
/// is the font size; `line_height` is the painter's per-line advance
/// (`size_px * LINE_HEIGHT_FACTOR`). The painter stacks line `i` centered at
/// `top + size_px/2 + inset + i*line_height` (see `renderer/cell/text.rs`),
/// and [`FIT_PADDING`] is the top+bottom slack (== 2 * that vertical inset).
fn line_block_height(line_count: usize, size_px: f64, line_height: f64) -> f64 {
    if line_count == 0 {
        return 0.0;
    }
    // First line occupies one font height; each subsequent baseline sits one
    // `line_height` lower (so `n` lines have `n - 1` gaps). FIT_PADDING is the
    // top+bottom slack, added once. A single line reduces to the original
    // `size_px + FIT_PADDING`, leaving normal cells and double-click autofit
    // unchanged.
    let gaps = (line_count - 1) as f64;
    size_px + gaps * line_height + FIT_PADDING
}

#[cfg(test)]
mod tests {
    use super::*;

    // A single line must equal the pre-multi-line behavior so normal cells and
    // the double-click autofit don't shift.
    #[test]
    fn single_line_is_font_plus_padding() {
        assert_eq!(line_block_height(1, 14.0, 28.0), 14.0 + FIT_PADDING);
    }

    // Each extra line adds exactly one `line_height` gap.
    #[test]
    fn each_extra_line_adds_one_line_height() {
        let one = line_block_height(1, 14.0, 28.0);
        assert_eq!(line_block_height(2, 14.0, 28.0), one + 28.0);
        assert_eq!(line_block_height(3, 14.0, 28.0), one + 2.0 * 28.0);
    }

    // Empty (defensive) contributes no height — the caller skips empty cells.
    #[test]
    fn zero_lines_is_zero() {
        assert_eq!(line_block_height(0, 14.0, 28.0), 0.0);
    }
}
