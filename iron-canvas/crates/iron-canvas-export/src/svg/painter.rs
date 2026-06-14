//! SVG backend for `Painter`.
//!
//! Pure `std`. Emits a self-contained `<svg>` document for snapshot tests,
//! exports, and headless rendering. Design notes: `docs/svg-painter.md`.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::mem;

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
    approx_text_width, parse_font_size_px,
};

use crate::common::escape::xml_escape;

pub struct SvgPainter {
    body: RefCell<String>,
    defs: RefCell<String>,
    clip_depth: Cell<u32>,
    next_clip_id: Cell<u32>,
    group_depth: Cell<u32>,
    pub(super) width: i32,
    pub(super) height: i32,
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

    /// Drain the buffered body + defs and return the finished SVG
    /// document. Takes `&self` so callers holding the painter behind
    /// `Rc` (e.g. `SvgSurface`) can finish without unwrapping.
    /// Asserts clip and group balance.
    pub fn finish(&self) -> String {
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

        let defs = mem::take(&mut *self.defs.borrow_mut());
        let body = mem::take(&mut *self.body.borrow_mut());
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

// CSS font shorthand → (size_px, family). Size comes from the shared
// `parse_font_size_px`; the family is everything after that token (only SVG
// needs it — PDF/recorder share just the size). Falls back to sans-serif so a
// missing/garbled string never panics.
fn parse_font(font_css: &str) -> (f64, String) {
    let toks: Vec<&str> = font_css.split_whitespace().collect();
    let family = toks
        .iter()
        .position(|t| t.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()).is_some())
        .map(|i| toks[i + 1..].join(" "))
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| "sans-serif".to_string());
    (parse_font_size_px(font_css), family)
}

impl TextMetrics for SvgPainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        let (size, _) = parse_font(font_css);
        approx_text_width(size, text)
    }
}

/// Write the shared `<rect x y width height` opening (no closing `>`) so the
/// four rect emitters (fill / stroke / dashed / clip) append only their own
/// attribute tail instead of repeating the geometry format (C-5).
fn open_rect(rect: PixelRect, out: &mut String) {
    let (x, y, w, h) = rect.as_f64_tuple();
    let _ = write!(
        out,
        "<rect x=\"{x:.3}\" y=\"{y:.3}\" width=\"{w:.3}\" height=\"{h:.3}\""
    );
}

impl Painter for SvgPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        let mut body = self.body.borrow_mut();
        open_rect(rect, &mut body);
        body.push_str(" fill=\"");
        xml_escape(color.as_str(), &mut body);
        body.push_str("\"/>");
    }

    fn fill_path(&self, points: &[Point], color: PaintColor) {
        if points.len() < 2 {
            return;
        }
        let mut body = self.body.borrow_mut();
        body.push_str("<path d=\"");
        let first = points[0];
        let _ = write!(body, "M{:.3} {:.3}", f64::from(first.x), f64::from(first.y));
        for p in &points[1..] {
            let _ = write!(body, " L{:.3} {:.3}", f64::from(p.x), f64::from(p.y));
        }
        body.push_str("Z\" fill=\"");
        xml_escape(color.as_str(), &mut body);
        body.push_str("\"/>");
    }

    fn clear_rect(&self, _rect: PixelRect) {
        // SVG has no concept of clearing alpha pixels; emitted elements
        // simply compose on top. The overlay-clear contract is a no-op here.
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let mut body = self.body.borrow_mut();
        open_rect(rect, &mut body);
        body.push_str(" fill=\"none\" stroke=\"");
        xml_escape(color.as_str(), &mut body);
        let _ = write!(body, "\" stroke-width=\"{width:.3}\"/>");
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let mut body = self.body.borrow_mut();
        open_rect(rect, &mut body);
        body.push_str(" fill=\"none\" stroke=\"");
        xml_escape(color.as_str(), &mut body);
        let _ = write!(body, "\" stroke-width=\"{width:.3}\" stroke-dasharray=\"4 3\"/>");
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
        {
            let mut defs = self.defs.borrow_mut();
            let _ = write!(defs, "<clipPath id=\"c{id}\">");
            open_rect(rect, &mut defs);
            defs.push_str("/></clipPath>");
        }
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

    fn begin_group(&self, class: GroupClass) {
        let mut body = self.body.borrow_mut();
        body.push_str("<g class=\"");
        body.push_str(class.as_str());
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

impl BlitPainter for SvgPainter {
    fn blit(&self, _src: PixelRect, _dst: PixelRect) {
        unreachable!(
            "SvgPainter::blit invoked — SVG export must always run via PaintRegime::Fresh"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::SvgPainter;
    use iron_canvas_core::geometry::pixel_rect::PixelRect;
    use iron_canvas_core::geometry::prim::Point;
    use iron_canvas_core::painter::{
        GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
    };

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
        p.begin_group(GroupClass::Grid);
        p.rect_fill(rect(0, 0, 1, 1), PaintColor::Static("#000"));
        p.end_group();
        let svg = p.finish();
        assert!(svg.contains("<g class=\"grid\">"));
    }

    #[test]
    fn grid_layer_has_section_groups() {
        // Drive the painter through the same begin_group/end_group
        // sequence `render_grid` produces: Grid wraps Cells / FrozenSep
        // / Headers / Corner as siblings. The rendered SVG should
        // surface that structure literally so a future SVG player can
        // toggle sections independently.
        let p = SvgPainter::new(10, 10);
        p.begin_group(GroupClass::Grid);
        p.begin_group(GroupClass::Cells);
        p.rect_fill(rect(0, 0, 1, 1), PaintColor::Static("#fff"));
        p.end_group();
        p.begin_group(GroupClass::FrozenSep);
        p.end_group();
        p.begin_group(GroupClass::Headers);
        p.end_group();
        p.begin_group(GroupClass::Corner);
        p.end_group();
        p.end_group();
        let svg = p.finish();

        let body_start = svg
            .find("<g class=\"grid\">")
            .expect("grid bracket present");
        let body_end = svg.rfind("</g>").expect("at least one closing g");
        let body = &svg[body_start..=body_end];
        assert!(body.contains("<g class=\"cells\">"));
        assert!(body.contains("<g class=\"frozen-sep\">"));
        assert!(body.contains("<g class=\"headers\">"));
        assert!(body.contains("<g class=\"corner\">"));
        // Sibling order: Cells first, Corner last — mirrors render_grid.
        let cells_at = body.find("<g class=\"cells\">").unwrap();
        let corner_at = body.find("<g class=\"corner\">").unwrap();
        assert!(cells_at < corner_at, "Cells must precede Corner in the SVG");
    }

    #[test]
    fn overlay_layer_has_structured_groups() {
        // Mirror of grid_layer_has_section_groups for the overlay
        // decoration brackets emitted by LayerBase::paint_overlay_layer.
        let p = SvgPainter::new(10, 10);
        p.begin_group(GroupClass::Overlay);
        for class in [
            GroupClass::SelectionFill,
            GroupClass::ActiveCellRepaint,
            GroupClass::SelectionStroke,
            GroupClass::HeaderHighlights,
            GroupClass::Autofill,
            GroupClass::Clipboard,
            GroupClass::PointMode,
            GroupClass::FormulaRefs,
        ] {
            p.begin_group(class);
            p.end_group();
        }
        p.end_group();
        let svg = p.finish();

        let body_start = svg
            .find("<g class=\"overlay\">")
            .expect("overlay bracket present");
        let body = &svg[body_start..];
        for required in [
            "<g class=\"selection-fill\">",
            "<g class=\"active-cell-repaint\">",
            "<g class=\"selection-stroke\">",
            "<g class=\"header-highlights\">",
            "<g class=\"autofill\">",
            "<g class=\"clipboard\">",
            "<g class=\"point-mode\">",
            "<g class=\"formula-refs\">",
        ] {
            assert!(body.contains(required), "overlay SVG missing {required}");
        }
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
