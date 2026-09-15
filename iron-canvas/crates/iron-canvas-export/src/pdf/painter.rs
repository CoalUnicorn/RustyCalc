//! PDF backend for `Painter`.
//!
//! Emits PDF content-stream ops into a shared `ContentStream` buffer.
//! No setter cache (PDF has no `ctx.*` state to dedupe against), no
//! per-paint clear (PDF is declarative — the page-open `cm` plus the
//! emitted ops are the entire output). The Y-flip CTM that maps the
//! painter's Y-down coords to PDF's Y-up user space is prepended by
//! [`PdfSurface::finish`](crate::pdf::PdfSurface::finish) rather than each painter op, so the painter
//! itself never needs to think about it: `re` is emitted with the same
//! `x y w h` we received from `PixelRect`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use iron_canvas_core::geometry::constants::DASHED_RECT_PATTERN;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
    parse_font_size_px,
};

use crate::common::color::parse_css_color;
use crate::common::escape::pdf_string_escape;
use crate::common::metrics;
use crate::pdf::doc::stream::ContentStream;

pub struct PdfPainter {
    body: Rc<RefCell<ContentStream>>,
    pub(super) width: u32,
    pub(super) height: u32,
    clip_depth: Cell<u32>,
    group_depth: Cell<u32>,
}

impl PdfPainter {
    /// Construct a painter that owns its own content stream. Use this
    /// for single-surface scenarios (unit tests, future
    /// orchestrator-driven export).
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_stream(Rc::new(RefCell::new(ContentStream::new())), width, height)
    }

    /// Construct a painter sharing an externally-owned stream. The
    /// `iron-canvas-web` facade uses this in Commit 4 to point both the
    /// grid and overlay `PdfSurface`s at the same buffer so paint calls
    /// from both layers concatenate in dispatch order.
    pub fn with_stream(body: Rc<RefCell<ContentStream>>, width: u32, height: u32) -> Self {
        Self {
            body,
            width,
            height,
            clip_depth: Cell::new(0),
            group_depth: Cell::new(0),
        }
    }

    /// Hand back the shared stream. `PdfSurface::finish` takes it, wraps
    /// it in the page-open CTM, and inlines the bytes as the `/Contents`
    /// object. Public so the `iron-canvas-web` facade (Commit 4) can
    /// point a second `PdfSurface` at the same buffer, and so smoke
    /// tests can inspect what each `Painter` method emits.
    pub fn stream(&self) -> Rc<RefCell<ContentStream>> {
        Rc::clone(&self.body)
    }

    /// Both `clip_depth` and `group_depth` must reach zero before
    /// `finish` runs — every `push_clip` / `begin_group` must have a
    /// matching pop. Mirrors `SvgPainter`'s balance assertion.
    pub(crate) fn assert_balanced(&self) {
        debug_assert_eq!(
            self.clip_depth.get(),
            0,
            "PdfPainter finished with unbalanced push_clip/pop_clip",
        );
        debug_assert_eq!(
            self.group_depth.get(),
            0,
            "PdfPainter finished with unbalanced begin_group/end_group",
        );
    }

    fn write(&self, bytes: &[u8]) {
        self.body.borrow_mut().write(bytes);
    }

    fn write_str(&self, s: &str) {
        self.write(s.as_bytes());
    }

    /// Emit `r g b rg` (nonstroking fill colour).
    fn emit_fill_color(&self, color: PaintColor) {
        let (r, g, b) = parse_css_color(color.as_str());
        self.write_str(&format!("{r:.3} {g:.3} {b:.3} rg\n"));
    }

    /// Emit `r g b RG` (stroking colour).
    fn emit_stroke_color(&self, color: PaintColor) {
        let (r, g, b) = parse_css_color(color.as_str());
        self.write_str(&format!("{r:.3} {g:.3} {b:.3} RG\n"));
    }

    fn emit_line_width(&self, width: f64) {
        self.write_str(&format!("{width:.3} w\n"));
    }

    fn emit_rect(&self, x: f64, y: f64, w: f64, h: f64) {
        self.write_str(&format!("{x:.3} {y:.3} {w:.3} {h:.3} re\n"));
    }

    fn emit_line(&self, x1: f64, y1: f64, x2: f64, y2: f64) {
        self.write_str(&format!("{x1:.3} {y1:.3} m\n{x2:.3} {y2:.3} l\nS\n"));
    }
}

impl TextMetrics for PdfPainter {
    // `fill_text` below always draws the base-14 standard Helvetica font
    // (`/F1`), regardless of the cell's declared family — PDF has no
    // embedded-font path, so `helvetica_advance_width` is the font that's
    // actually painted, not an approximation of it.
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        let size = parse_font_size_px(font_css);
        metrics::helvetica_advance_width(text, size)
    }
}

impl Painter for PdfPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.emit_fill_color(color);
        self.emit_rect(x, y, w, h);
        self.write_str("f\n");
    }

    fn fill_path(&self, points: &[Point], color: PaintColor) {
        if points.len() < 2 {
            return;
        }
        self.emit_fill_color(color);
        let first = points[0];
        self.write_str(&format!(
            "{:.3} {:.3} m\n",
            f64::from(first.x),
            f64::from(first.y)
        ));
        for p in &points[1..] {
            self.write_str(&format!("{:.3} {:.3} l\n", f64::from(p.x), f64::from(p.y)));
        }
        self.write_str("h\nf\n"); // h closes the subpath; f fills
    }

    fn clear_rect(&self, rect: PixelRect) {
        // PDF has no destination clear — fill with opaque white, which
        // matches what the canvas's default background paints over.
        let (x, y, w, h) = rect.as_f64_tuple();
        self.write_str("1.000 1.000 1.000 rg\n");
        self.emit_rect(x, y, w, h);
        self.write_str("f\n");
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.emit_stroke_color(color);
        self.emit_line_width(width);
        self.emit_rect(x, y, w, h);
        self.write_str("S\n");
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.emit_stroke_color(color);
        self.emit_line_width(width);
        self.write_str(&format!(
            "[{} {}] 0 d\n",
            DASHED_RECT_PATTERN[0], DASHED_RECT_PATTERN[1]
        ));
        self.emit_rect(x, y, w, h);
        self.write_str("S\n");
        // Reset to solid so subsequent strokes don't inherit the dash.
        self.write_str("[] 0 d\n");
    }

    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, f64::from(y), color, width),
            Line::V { x, span } => self.stroke_vline(f64::from(x), span, color, width),
        }
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        self.emit_stroke_color(color);
        self.emit_line_width(width);
        self.emit_line(f64::from(span.from), y, f64::from(span.to), y);
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        self.emit_stroke_color(color);
        self.emit_line_width(width);
        self.emit_line(x, f64::from(span.from), x, f64::from(span.to));
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.emit_stroke_color(color);
        self.emit_line_width(width);
        self.emit_line(x1, y, x2, y);
    }

    fn push_clip(&self, rect: PixelRect) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.clip_depth.set(self.clip_depth.get() + 1);
        // `q` pushes graphics state; `W n` installs the clip without
        // filling. `pop_clip` matches with a single `Q`.
        self.write_str("q\n");
        self.emit_rect(x, y, w, h);
        self.write_str("W n\n");
    }

    fn pop_clip(&self) {
        let depth = self.clip_depth.get();
        debug_assert!(depth > 0, "PdfPainter::pop_clip with no matching push");
        self.clip_depth.set(depth.saturating_sub(1));
        self.write_str("Q\n");
    }

    fn fill_text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        let font_str = font_css.as_str();
        let size = parse_font_size_px(font_str);

        // Real Helvetica width — same as measure_text_width, recomputed
        // here to avoid a borrow round-trip. Must stay in sync: `Tm`
        // below positions the run using this same estimate.
        let text_width = metrics::helvetica_advance_width(text, size);

        // Painter coords are Y-down. After the outer Y-flip CTM,
        // a counter-flipping text matrix (`1 0 0 -1 tx ty Tm`) restores
        // upright glyphs while letting us specify the baseline origin
        // directly in painter coordinates.
        let tx = match align {
            TextAlign::Start => x,
            TextAlign::Center => x - text_width / 2.0,
            TextAlign::End => x - text_width,
        };
        // Helvetica ascent aprox 0.8 x size, descent aprox 0.2 x size — Type1 base-14
        // metrics aren't queryable without parsing AFM tables we don't
        // ship, so these are the published nominal ratios.
        let ty = match baseline {
            TextBaseline::Top => y + size * 0.8,
            TextBaseline::Middle => y + size * 0.5,
            TextBaseline::Bottom => y - size * 0.2,
            TextBaseline::Alphabetic => y,
        };

        let (r, g, b) = parse_css_color(color.as_str());

        let mut buf = Vec::with_capacity(text.len() + 64);
        buf.extend_from_slice(b"BT\n");
        buf.extend_from_slice(format!("/F1 {size:.3} Tf\n").as_bytes());
        buf.extend_from_slice(format!("{r:.3} {g:.3} {b:.3} rg\n").as_bytes());
        buf.extend_from_slice(format!("1 0 0 -1 {tx:.3} {ty:.3} Tm\n").as_bytes());
        buf.push(b'(');
        pdf_string_escape(text, &mut buf);
        buf.extend_from_slice(b") Tj\n");
        buf.extend_from_slice(b"ET\n");
        self.write(&buf);
    }

    fn invalidate_cache(&self) {
        // No setter cache to dump — PDF ops carry their own color/width.
    }

    fn apply_dpr_transform(&self, _dpr: f64) {
        // DPR doesn't apply to PDF user space (1/72" regardless of
        // device pixel density). The page-open CTM in PdfSurface::finish
        // already maps painter coords to PDF coords; nothing else needs
        // to happen here.
    }

    fn reset_text_defaults(&self) {
        // PDF text state is bracketed by BT/ET per fill_text call, so
        // there's nothing sticky to reset between calls.
    }

    fn begin_group(&self, class: GroupClass) {
        self.group_depth.set(self.group_depth.get() + 1);
        // Comment for debug readability, then `q` to bracket the group
        // ops in their own graphics-state save/restore pair.
        self.write_str(&format!("% group: {}\nq\n", class.as_str()));
    }

    fn end_group(&self) {
        let depth = self.group_depth.get();
        debug_assert!(
            depth > 0,
            "PdfPainter::end_group with no matching begin_group"
        );
        self.group_depth.set(depth.saturating_sub(1));
        self.write_str("Q\n");
    }
}

impl BlitPainter for PdfPainter {
    fn blit(&self, _src: PixelRect, _dst: PixelRect) {
        // PDF has no source-copy primitive. The throwaway export
        // orchestrator can never reach the ScrollBlit strategy — see the
        // "`BlitPainter::blit` — short-circuit (proven safe)" section of
        // OUTPUT_REFACTOR_PLAN.md for the proof. No-op is sound.
    }
}
