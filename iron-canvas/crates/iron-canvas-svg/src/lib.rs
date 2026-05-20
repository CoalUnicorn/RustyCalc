//! SVG backend for `Painter`.
//!
//! Pure `std`. Emits a self-contained `<svg>` document for snapshot tests,
//! exports, and headless rendering. Design notes: `docs/svg-painter.md`.

#![allow(dead_code)]
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Span};
use iron_canvas_core::painter::{PaintColor, Painter, TextAlign, TextBaseline, TextMetrics};

// Matches RecorderPainter's fallback so wrap math is consistent across
// non-browser backends. SVG has no host-side text measurement API.
const CHAR_WIDTH_FACTOR: f64 = 1.0;

pub struct SvgPainter {
    body: RefCell<String>,
    defs: RefCell<String>,
    clip_depth: Cell<u32>,
    next_clip_id: Cell<u32>,
    group_depth: Cell<u32>,
    width: i32,
    height: i32,
    dpr: Cell<i32>,
}

impl SvgPainter {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            body: RefCell::new(String::new()),
            defs: RefCell::new(String::new()),
            clip_depth: Cell::new(0),
            next_clip_id: Cell::new(0),
            group_depth: Cell::new(0),
            width,
            height,
            dpr: Cell::new(1),
        }
    }

    /// Consume the painter and return the finished SVG document.
    /// Asserts clip and group balance.
    pub fn finish(self) -> String {
        debug_assert_eq!(
            self.clip_depth.get(),
            0,
            "SvgPainter finished with unbalanced push_clip/pop_clip",
        );
        debug_assert_eq!(
            self.group_depth.get(),
            0,
            "SvgPainter finished with unbalanced begin_group/end_group",
        );

        let dpr = self.dpr.get();
        let (attr_w, attr_h) = if dpr > 1 {
            (self.width * dpr, self.height * dpr)
        } else {
            (self.width, self.height)
        };

        let defs = self.defs.into_inner();
        let body = self.body.into_inner();
        let mut out = String::with_capacity(defs.len() + body.len() + 256);
        let _ = write!(
            out,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
            attr_w, attr_h, self.width, self.height
        );
        if !defs.is_empty() {
            out.push_str("<defs>");
            out.push_str(&defs);
            out.push_str("</defs>");
        }
        out.push_str(&body);
        out.push_str("</svg>");
        out
    }
}

fn xml_escape(s: &str, out: &mut String) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
}

// CSS font shorthand → (size_px, family). Skips weight/style tokens before
// the size; everything after the size joins as the family. Falls back to
// 12px sans-serif so a missing/garbled string never panics.
fn parse_font(font_css: &str) -> (f64, String) {
    let mut size = 12.0;
    let mut family_parts: Vec<&str> = Vec::new();
    let mut found_size = false;
    for tok in font_css.split_whitespace() {
        if !found_size {
            if let Some(n) = tok.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()) {
                size = n;
                found_size = true;
            }
        } else {
            family_parts.push(tok);
        }
    }
    let family = if family_parts.is_empty() {
        "sans-serif".to_string()
    } else {
        family_parts.join(" ")
    };
    (size, family)
}

impl TextMetrics for SvgPainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        let (size, _) = parse_font(font_css);
        text.chars().count() as f64 * size * CHAR_WIDTH_FACTOR
    }
}

impl Painter for SvgPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        let (x, y, w, h) = rect.as_f64_tuple();
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" fill=\"",
            x, y, w, h
        );
        xml_escape(color.as_str(), &mut body);
        body.push_str("\"/>");
    }

    fn clear_rect(&self, _rect: PixelRect) {
        // SVG has no concept of clearing alpha pixels; emitted elements
        // simply compose on top. The overlay-clear contract is a no-op here.
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let (x, y, w, h) = rect.as_f64_tuple();
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" fill=\"none\" stroke=\"",
            x, y, w, h
        );
        xml_escape(color.as_str(), &mut body);
        let _ = write!(body, "\" stroke-width=\"{:.3}\"/>", width);
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let (x, y, w, h) = rect.as_f64_tuple();
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" fill=\"none\" stroke=\"",
            x, y, w, h
        );
        xml_escape(color.as_str(), &mut body);
        let _ = write!(
            body,
            "\" stroke-width=\"{:.3}\" stroke-dasharray=\"4 3\"/>",
            width
        );
    }

    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, f64::from(y), color, width),
            Line::V { x, span } => self.stroke_vline(f64::from(x), span, color, width),
        }
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"",
            f64::from(span.from),
            y,
            f64::from(span.to),
            y
        );
        xml_escape(color.as_str(), &mut body);
        let _ = write!(body, "\" stroke-width=\"{:.3}\"/>", width);
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"",
            x,
            f64::from(span.from),
            x,
            f64::from(span.to),
        );
        xml_escape(color.as_str(), &mut body);
        let _ = write!(body, "\" stroke-width=\"{:.3}\"/>", width);
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<line x1=\"{:.3}\" y1=\"{:.3}\" x2=\"{:.3}\" y2=\"{:.3}\" stroke=\"",
            x1, y, x2, y
        );
        xml_escape(color.as_str(), &mut body);
        let _ = write!(body, "\" stroke-width=\"{:.3}\"/>", width);
    }

    fn push_clip(&self, rect: PixelRect) {
        let id = self.next_clip_id.get();
        self.next_clip_id.set(id + 1);
        let (x, y, w, h) = rect.as_f64_tuple();
        let _ = write!(
            self.defs.borrow_mut(),
            "<clipPath id=\"c{}\"><rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\"/></clipPath>",
            id, x, y, w, h
        );
        let _ = write!(self.body.borrow_mut(), "<g clip-path=\"url(#c{})\">", id);
        self.clip_depth.set(self.clip_depth.get() + 1);
    }

    fn pop_clip(&self) {
        debug_assert!(
            self.clip_depth.get() > 0,
            "SvgPainter pop_clip without matching push_clip",
        );
        self.clip_depth.set(self.clip_depth.get() - 1);
        self.body.borrow_mut().push_str("</g>");
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
        let (size, family) = parse_font(font_css.as_str());
        let anchor = match align {
            TextAlign::Start => "start",
            TextAlign::Center => "middle",
            TextAlign::End => "end",
        };
        let dom_baseline = match baseline {
            TextBaseline::Top => "hanging",
            TextBaseline::Middle => "central",
            TextBaseline::Bottom => "text-after-edge",
            TextBaseline::Alphabetic => "alphabetic",
        };
        let mut body = self.body.borrow_mut();
        let _ = write!(body, "<text x=\"{:.3}\" y=\"{:.3}\" font-family=\"", x, y);
        xml_escape(&family, &mut body);
        let _ = write!(body, "\" font-size=\"{:.3}\" fill=\"", size);
        xml_escape(color.as_str(), &mut body);
        let _ = write!(
            body,
            "\" text-anchor=\"{}\" dominant-baseline=\"{}\">",
            anchor, dom_baseline
        );
        xml_escape(text, &mut body);
        body.push_str("</text>");
    }

    fn invalidate_cache(&self) {}

    fn reset_text_defaults(&self) {}

    fn apply_dpr_transform(&self, dpr: i32) {
        self.dpr.set(dpr);
    }

    fn begin_group(&self, class: &'static str) {
        let mut body = self.body.borrow_mut();
        body.push_str("<g class=\"");
        xml_escape(class, &mut body);
        body.push_str("\">");
        self.group_depth.set(self.group_depth.get() + 1);
    }

    fn end_group(&self) {
        debug_assert!(
            self.group_depth.get() > 0,
            "SvgPainter end_group without matching begin_group",
        );
        self.group_depth.set(self.group_depth.get() - 1);
        self.body.borrow_mut().push_str("</g>");
    }
}

#[cfg(test)]
mod tests {
    use super::SvgPainter;
    use iron_canvas_core::geometry::pixel_rect::PixelRect;
    use iron_canvas_core::geometry::prim::{Line, Point, Span};
    use iron_canvas_core::painter::{PaintColor, Painter, TextAlign, TextBaseline, TextMetrics};

    fn rect(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
        PixelRect {
            top_left: Point { x, y },
            width: w,
            height: h,
        }
    }

    #[test]
    fn empty_finish_emits_bare_svg() {
        let p = SvgPainter::new(100, 50);
        let svg = p.finish();
        assert!(svg.starts_with("<svg "));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("width=\"100\""));
        assert!(svg.contains("height=\"50\""));
        assert!(svg.contains("viewBox=\"0 0 100 50\""));
        assert!(!svg.contains("<defs>"));
    }

    #[test]
    fn rect_fill_emits_rect_element() {
        let p = SvgPainter::new(100, 50);
        p.rect_fill(rect(1, 2, 3, 4), PaintColor::Static("#ff0000"));
        let svg = p.finish();
        assert!(svg.contains("<rect"));
        assert!(svg.contains("fill=\"#ff0000\""));
    }

    #[test]
    fn push_pop_clip_balances_and_emits_defs() {
        let p = SvgPainter::new(10, 10);
        p.push_clip(rect(0, 0, 5, 5));
        p.rect_fill(rect(0, 0, 5, 5), PaintColor::Static("#abc"));
        p.pop_clip();
        let svg = p.finish();
        assert!(svg.contains("<defs><clipPath id=\"c0\""));
        assert!(svg.contains("clip-path=\"url(#c0)\""));
        assert!(svg.contains("</g>"));
    }

    #[test]
    fn group_wrappers_emit_classed_g() {
        let p = SvgPainter::new(10, 10);
        p.begin_group("grid");
        p.rect_fill(rect(0, 0, 1, 1), PaintColor::Static("#000"));
        p.end_group();
        let svg = p.finish();
        assert!(svg.contains("<g class=\"grid\">"));
    }

    #[test]
    fn fill_text_xml_escapes_content() {
        let p = SvgPainter::new(10, 10);
        p.fill_text(
            "a<b&c\"d",
            1.0,
            2.0,
            PaintColor::Static("12px sans-serif"),
            PaintColor::Static("#000"),
            TextAlign::Start,
            TextBaseline::Alphabetic,
        );
        let svg = p.finish();
        assert!(svg.contains("a&lt;b&amp;c&quot;d"));
        assert!(svg.contains("text-anchor=\"start\""));
        assert!(svg.contains("dominant-baseline=\"alphabetic\""));
    }

    #[test]
    fn measure_text_width_uses_font_size() {
        let p = SvgPainter::new(10, 10);
        assert_eq!(p.measure_text_width("hello", "16px sans-serif"), 80.0);
        assert_eq!(p.measure_text_width("hi", "bold 12px serif"), 24.0);
        assert_eq!(p.measure_text_width("ab", "no-size"), 24.0);
    }

    #[test]
    fn dpr_scales_attr_dimensions_only() {
        let p = SvgPainter::new(100, 50);
        p.apply_dpr_transform(2);
        let svg = p.finish();
        assert!(svg.contains("width=\"200\""));
        assert!(svg.contains("height=\"100\""));
        assert!(svg.contains("viewBox=\"0 0 100 50\""));
    }
}
