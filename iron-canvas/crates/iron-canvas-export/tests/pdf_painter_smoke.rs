//! Commit 3 smoke test: each `Painter`/`BlitPainter`/`TextMetrics`
//! method on `PdfPainter` emits the right PDF content-stream ops, and
//! `PdfSurface` satisfies the `Surface` trait bound so the orchestrator
//! can drive it.
//!
//! End-to-end orchestrator-driven paint isn't tested here for the same
//! reason `svg_surface_smoke` declined to: building a stub `CanvasModel`
//! is ~200 LOC for marginal extra coverage over the existing
//! `MemSurface`-driven integration tests in `iron-canvas-core`. The
//! contract that needs covering for PDF is the new bit — the
//! `Painter` → content-stream translation — and that's what each test
//! below pins.

#![cfg(feature = "pdf")]
// Test names intentionally embed PDF op letters (S/Q/BT/ET/RG/Tf/Tj/Tm)
// for readability — they document which content-stream operator the
// test pins. Suppress the snake_case lint for the whole file.
#![allow(non_snake_case)]

use std::rc::Rc;

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::layer::Surface;
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
};
use iron_canvas_core::{CanvasModel, Orchestrator};
use iron_canvas_export::pdf::{PdfPainter, PdfSurface};

const W: u32 = 100;
const H: u32 = 50;

fn rect(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
    PixelRect {
        top_left: Point { x, y },
        width: w,
        height: h,
    }
}

fn snapshot(p: &PdfPainter) -> String {
    let bytes = stream_bytes(p);
    match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => panic!("painter emitted non-UTF8 bytes"),
    }
}

fn stream_bytes(p: &PdfPainter) -> Vec<u8> {
    p.stream().borrow().bytes().to_vec()
}

// ---------------------------------------------------------------------------
// Painter trait — per-op output
// ---------------------------------------------------------------------------

#[test]
fn rect_fill_emits_rgb_then_re_then_f() {
    let p = PdfPainter::new(W, H);
    p.rect_fill(rect(10, 20, 30, 40), PaintColor::Static("#ff0000"));
    let s = snapshot(&p);
    assert!(s.contains("1.000 0.000 0.000 rg"), "missing red rg: {s:?}");
    assert!(s.contains("10.000 20.000 30.000 40.000 re"), "missing re: {s:?}");
    assert!(s.contains("\nf\n"), "missing fill op: {s:?}");
}

#[test]
fn rect_stroke_emits_rgb_width_re_S() {
    let p = PdfPainter::new(W, H);
    p.rect_stroke(rect(5, 5, 10, 10), PaintColor::Static("#00ff00"), 2.5);
    let s = snapshot(&p);
    assert!(s.contains("0.000 1.000 0.000 RG"), "missing green RG: {s:?}");
    assert!(s.contains("2.500 w"), "missing line width: {s:?}");
    assert!(s.contains("5.000 5.000 10.000 10.000 re"), "missing re: {s:?}");
    assert!(s.contains("\nS\n"), "missing stroke op: {s:?}");
}

#[test]
fn rect_dashed_emits_dash_pattern_and_resets() {
    let p = PdfPainter::new(W, H);
    p.rect_dashed(rect(0, 0, 10, 10), PaintColor::Static("#0000ff"), 1.0);
    let s = snapshot(&p);
    assert!(s.contains("[3 3] 0 d"), "missing dash pattern: {s:?}");
    assert!(s.contains("[] 0 d"), "missing dash reset: {s:?}");
    assert!(s.contains("\nS\n"));
}

#[test]
fn clear_rect_fills_white() {
    let p = PdfPainter::new(W, H);
    p.clear_rect(rect(0, 0, W as i32, H as i32));
    let s = snapshot(&p);
    assert!(s.contains("1.000 1.000 1.000 rg"), "clear_rect must use white: {s:?}");
    assert!(s.contains("0.000 0.000 100.000 50.000 re"));
    assert!(s.contains("\nf\n"));
}

#[test]
fn stroke_hline_emits_m_l_S() {
    let p = PdfPainter::new(W, H);
    p.stroke_hline(
        Span { from: 10, to: 90 },
        25.0,
        PaintColor::Static("#000000"),
        1.0,
    );
    let s = snapshot(&p);
    assert!(s.contains("10.000 25.000 m"), "missing moveto: {s:?}");
    assert!(s.contains("90.000 25.000 l"), "missing lineto: {s:?}");
    assert!(s.contains("\nS\n"));
}

#[test]
fn stroke_vline_emits_m_l_S() {
    let p = PdfPainter::new(W, H);
    p.stroke_vline(
        50.0,
        Span { from: 5, to: 45 },
        PaintColor::Static("#000000"),
        1.0,
    );
    let s = snapshot(&p);
    assert!(s.contains("50.000 5.000 m"));
    assert!(s.contains("50.000 45.000 l"));
}

#[test]
fn stroke_line_dispatches_h_and_v() {
    let p = PdfPainter::new(W, H);
    p.stroke_line(
        Line::H {
            span: Span { from: 0, to: 10 },
            y: 5,
        },
        PaintColor::Static("#000000"),
        1.0,
    );
    p.stroke_line(
        Line::V {
            x: 5,
            span: Span { from: 0, to: 10 },
        },
        PaintColor::Static("#000000"),
        1.0,
    );
    let s = snapshot(&p);
    assert!(s.contains("0.000 5.000 m"));
    assert!(s.contains("10.000 5.000 l"));
    assert!(s.contains("5.000 0.000 m"));
    assert!(s.contains("5.000 10.000 l"));
}

#[test]
fn push_pop_clip_emits_q_W_n_and_Q() {
    let p = PdfPainter::new(W, H);
    p.push_clip(rect(10, 10, 50, 30));
    p.pop_clip();
    let s = snapshot(&p);
    assert!(s.contains("q\n"), "missing graphics-state push: {s:?}");
    assert!(s.contains("10.000 10.000 50.000 30.000 re"));
    assert!(s.contains("W n\n"), "missing clip op: {s:?}");
    assert!(s.contains("Q\n"), "missing graphics-state pop: {s:?}");
}

#[test]
fn begin_end_group_emits_comment_and_q_Q() {
    let p = PdfPainter::new(W, H);
    p.begin_group(GroupClass::Cells);
    p.end_group();
    let s = snapshot(&p);
    assert!(s.contains("% group: cells"), "missing debug comment: {s:?}");
    assert!(s.contains("q\n"));
    assert!(s.contains("Q\n"));
}

#[test]
fn fill_text_emits_BT_Tf_color_Tm_Tj_ET() {
    let p = PdfPainter::new(W, H);
    p.fill_text(
        "Hi",
        20.0,
        15.0,
        PaintColor::Static("14px sans-serif"),
        PaintColor::Static("#000000"),
        TextAlign::Start,
        TextBaseline::Alphabetic,
    );
    let s = snapshot(&p);
    assert!(s.contains("BT\n"), "missing text-object start: {s:?}");
    assert!(s.contains("/F1 14.000 Tf"), "missing font select: {s:?}");
    assert!(s.contains("1 0 0 -1 20.000 15.000 Tm"), "missing text matrix: {s:?}");
    assert!(s.contains("(Hi) Tj"), "missing text payload: {s:?}");
    assert!(s.contains("ET\n"), "missing text-object end: {s:?}");
}

#[test]
fn fill_text_align_center_shifts_x_by_half_width() {
    let p = PdfPainter::new(W, H);
    // 4 chars at 10px each ⇒ width 40; centred at x=100 ⇒ tx = 80.
    p.fill_text(
        "abcd",
        100.0,
        20.0,
        PaintColor::Static("10px sans-serif"),
        PaintColor::Static("#000000"),
        TextAlign::Center,
        TextBaseline::Alphabetic,
    );
    let s = snapshot(&p);
    assert!(s.contains("1 0 0 -1 80.000 20.000 Tm"), "centre alignment off: {s:?}");
}

#[test]
fn fill_text_align_end_shifts_x_by_full_width() {
    let p = PdfPainter::new(W, H);
    p.fill_text(
        "abcd",
        100.0,
        20.0,
        PaintColor::Static("10px sans-serif"),
        PaintColor::Static("#000000"),
        TextAlign::End,
        TextBaseline::Alphabetic,
    );
    let s = snapshot(&p);
    assert!(s.contains("1 0 0 -1 60.000 20.000 Tm"), "end alignment off: {s:?}");
}

#[test]
fn fill_text_escapes_parens_and_backslash() {
    let p = PdfPainter::new(W, H);
    p.fill_text(
        "a(b\\c)",
        0.0,
        0.0,
        PaintColor::Static("12px sans-serif"),
        PaintColor::Static("#000000"),
        TextAlign::Start,
        TextBaseline::Alphabetic,
    );
    let s = snapshot(&p);
    assert!(s.contains("(a\\(b\\\\c\\)) Tj"), "escape failed: {s:?}");
}

#[test]
fn blit_is_no_op() {
    let p = PdfPainter::new(W, H);
    p.blit(rect(0, 0, 10, 10), rect(20, 20, 10, 10));
    assert_eq!(stream_bytes(&p), b"", "blit must emit nothing for PDF");
}

#[test]
fn invalidate_cache_and_reset_text_defaults_are_no_ops() {
    let p = PdfPainter::new(W, H);
    p.invalidate_cache();
    p.reset_text_defaults();
    p.apply_dpr_transform(2);
    assert_eq!(stream_bytes(&p), b"", "no-op ops must emit nothing");
}

#[test]
fn measure_text_width_uses_font_size_and_char_count() {
    let p = PdfPainter::new(W, H);
    // CHAR_WIDTH_FACTOR = 1.0 — mirrors svg_painter's contract.
    assert_eq!(p.measure_text_width("hello", "16px sans-serif"), 80.0);
    assert_eq!(p.measure_text_width("hi", "bold 12px serif"), 24.0);
    assert_eq!(p.measure_text_width("ab", "no-size"), 24.0); // default 12px
}

// ---------------------------------------------------------------------------
// PdfSurface — Surface trait + finish()
// ---------------------------------------------------------------------------

#[test]
fn orchestrator_accepts_pdf_surface() {
    // Mirrors svg_surface_smoke::orchestrator_accepts_svg_surface — the
    // point is that the trait bounds resolve, not that anything paints.
    let grid = PdfSurface::new(W, H);
    let overlay = PdfSurface::new(W, H);
    let _orch: Orchestrator<PdfSurface, Rc<dyn CanvasModel>> = Orchestrator::new(grid, overlay);
}

#[test]
fn finish_emits_y_flip_cm_at_page_origin() {
    let s = PdfSurface::new(W, H);
    let bytes = s.finish();
    // The CTM lives inside the /Contents stream object — search for the
    // exact `1 0 0 -1 0 <H> cm` payload independently of the wrapping
    // `<< /Length N >> stream\n…\nendstream` framing.
    let needle = format!("1 0 0 -1 0 {H} cm");
    assert!(
        find_substr(&bytes, needle.as_bytes()),
        "page-open CTM missing: expected `{needle}` somewhere in document"
    );
}

#[test]
fn finish_records_dimensions_in_mediabox() {
    let s = PdfSurface::new(W, H);
    let bytes = s.finish();
    let needle = format!("/MediaBox [0 0 {W} {H}]");
    assert!(
        find_substr(&bytes, needle.as_bytes()),
        "/MediaBox dimensions missing: expected `{needle}`"
    );
}

#[test]
fn finish_round_trips_through_pdf_header_and_eof() {
    let s = PdfSurface::new(W, H);
    let bytes = s.finish();
    assert!(bytes.starts_with(b"%PDF-1.7\n"));
    assert!(bytes.ends_with(b"%%EOF"));
}

#[test]
fn finish_includes_painter_output_after_ctm() {
    let s = PdfSurface::new(W, H);
    s.painter().rect_fill(
        rect(5, 5, 10, 10),
        PaintColor::Static("#abcdef"),
    );
    let bytes = s.finish();
    // The fill op must appear AFTER the CTM in the stream.
    let cm_needle = format!("1 0 0 -1 0 {H} cm");
    let re_needle = b"5.000 5.000 10.000 10.000 re";
    let Some(cm_pos) = position(&bytes, cm_needle.as_bytes()) else {
        panic!("cm missing");
    };
    let Some(re_pos) = position(&bytes, re_needle) else {
        panic!("re missing");
    };
    assert!(
        cm_pos < re_pos,
        "painter ops emitted before page-open CTM (cm at {cm_pos}, re at {re_pos})"
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_substr(haystack: &[u8], needle: &[u8]) -> bool {
    position(haystack, needle).is_some()
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}
