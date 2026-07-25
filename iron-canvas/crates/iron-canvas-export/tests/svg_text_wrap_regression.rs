//! Regression for the SVG/PDF export text-clipping bug (2026-07-01):
//! `CHAR_WIDTH_FACTOR = 1.0` was ~2x wider than any real font, so a
//! wrapped cell that fits ~3 lines in the live canvas wrapped to 7 lines
//! in SVG/PDF export and overflowed its fixed row height, getting clipped.
//!
//! Reproduces the exact numbers from the reported bug's exported SVG: a
//! 253px-wide column (245px usable after `CELL_PADDING`), a 13px font, a
//! 55px-tall row, and the actual wrapped paragraph. Asserts not just that
//! the line count fell (a real fix could still overflow a *shorter* row),
//! but that the wrapped block's total height now fits inside the row —
//! the thing that was actually reported broken.

#![cfg(any(feature = "svg", feature = "pdf"))]

use iron_canvas_core::painter::CHAR_WIDTH_FACTOR;
use iron_canvas_core::renderer::cell::text::{TextLine, layout_into};

const REAL_ROW_HEIGHT_PX: f64 = 55.0;
const REAL_USABLE_WIDTH_PX: f64 = 245.0;
const REAL_FONT_SIZE_PX: f64 = 13.0;
// Mirrors `iron_canvas_core::renderer::cell::text::LINE_HEIGHT_FACTOR`,
// which is `pub(crate)` and not reachable from this crate — duplicated
// here, documented, rather than widening its visibility for one test.
const LINE_HEIGHT_FACTOR: f64 = 1.2;
const PARAGRAPH: &str = "Estimate your monthly mortgage payments \
and explore how different loan conditions affect your repayment";

fn wrapped_lines(
    metrics: &dyn iron_canvas_core::painter::TextMetrics,
    font_css: &str,
) -> Vec<TextLine> {
    let mut lines = Vec::new();
    let mut wrap_buf = String::new();
    layout_into(
        metrics,
        font_css,
        PARAGRAPH,
        true, // wrap_text: true, same as the failing cell's style
        REAL_USABLE_WIDTH_PX,
        REAL_FONT_SIZE_PX * CHAR_WIDTH_FACTOR,
        &mut lines,
        &mut wrap_buf,
    );
    lines
}

fn assert_block_fits_row(lines: &[TextLine], backend: &str) {
    assert!(
        lines.len() <= 4,
        "{backend}: expected far fewer than the old 7 wrapped lines, got {}",
        lines.len()
    );
    let block_height = lines.len() as f64 * (REAL_FONT_SIZE_PX * LINE_HEIGHT_FACTOR);
    assert!(
        block_height <= REAL_ROW_HEIGHT_PX,
        "{backend}: wrapped block ({block_height:.1}px) still overflows the real \
         {REAL_ROW_HEIGHT_PX}px row — this is the bug that was reported, not just a \
         reduced line count"
    );
}

#[cfg(feature = "svg")]
#[test]
fn svg_export_wraps_the_mortgage_paragraph_to_fit_its_row() {
    let painter = iron_canvas_export::SvgPainter::new(10, 10);
    let lines = wrapped_lines(&painter, "13px Inter");
    assert_block_fits_row(&lines, "svg");
}

#[cfg(feature = "pdf")]
#[test]
fn pdf_export_wraps_the_mortgage_paragraph_to_fit_its_row() {
    let painter = iron_canvas_export::PdfPainter::new(300, 300);
    let lines = wrapped_lines(&painter, "13px Aptos Narrow");
    assert_block_fits_row(&lines, "pdf");
}
