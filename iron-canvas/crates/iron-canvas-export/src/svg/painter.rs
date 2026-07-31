//! SVG backend for `Painter`.
//!
//! Pure `std`. Emits a self-contained `<svg>` document for snapshot tests,
//! exports, and headless rendering. Draw calls buffer into `body`; clip paths
//! are emitted to a separate `defs` section and referenced by id.
//! [`SvgPainter::finish`] drains both into one document, asserting clip/group
//! balance.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::mem;

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
    parse_font_size_px,
};

use crate::common::escape::xml_escape;
use crate::common::metrics;

pub struct SvgPainter {
    body: RefCell<String>,
    defs: RefCell<String>,
    clip_depth: Cell<u32>,
    next_clip_id: Cell<u32>,
    group_depth: Cell<u32>,
    /// Set the first time `fill_text` runs. Gates the `@font-face` block
    /// `finish()` prepends to `defs` — cells with no text (or a painter
    /// that never draws any) keep emitting no `<defs>` at all, same as
    /// today, instead of always paying for the embedded font's data URI.
    has_text: Cell<bool>,
    pub(super) width: i32,
    pub(super) height: i32,
    dpr: Cell<f64>,
}

impl SvgPainter {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            body: RefCell::new(String::new()),
            defs: RefCell::new(String::new()),
            clip_depth: Cell::new(0),
            next_clip_id: Cell::new(0),
            group_depth: Cell::new(0),
            has_text: Cell::new(false),
            width,
            height,
            dpr: Cell::new(1.0),
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
        let (attr_w, attr_h) = if dpr > 1.0 {
            (f64::from(self.width) * dpr, f64::from(self.height) * dpr)
        } else {
            (f64::from(self.width), f64::from(self.height))
        };

        let mut defs = mem::take(&mut *self.defs.borrow_mut());
        let body = mem::take(&mut *self.body.borrow_mut());
        // Embed the bundled Inter font as a data URI so any viewer — not
        // just ones with Inter installed — renders the exact glyphs
        // `measure_text_width` measured. Gated on `has_text`: a painter
        // that never drew text keeps emitting no `<defs>` at all.
        if self.has_text.get() {
            let mut font_face = String::with_capacity(metrics::inter_base64().len() + 160);
            let _ = write!(
                font_face,
                "<style>@font-face{{font-family:'Inter';src:url(data:font/truetype;base64,{}) format('truetype');}}</style>",
                metrics::inter_base64()
            );
            font_face.push_str(&defs);
            defs = font_face;
        }
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

impl TextMetrics for SvgPainter {
    // Export always renders (and now always emits) the bundled Inter font —
    // see `fill_text` — so wrap math must measure that same font, not the
    // cell's declared family. `metrics::inter_advance_width` is the real
    // glyph-advance sum; unmapped glyphs fall back to the flat estimate.
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        let size = parse_font_size_px(font_css);
        metrics::inter_advance_width(text, size)
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
        let _ = write!(
            body,
            "\" stroke-width=\"{width:.3}\" stroke-dasharray=\"4 3\"/>"
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
        let size = parse_font_size_px(font_css.as_str());
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
        // The cell's declared family is intentionally not emitted here: SVG
        // export always renders the bundled Inter font (embedded via
        // `finish`'s `@font-face`), so `measure_text_width` and the glyphs a
        // viewer actually paints agree regardless of what font the workbook
        // itself names — see the metrics module doc comment.
        self.has_text.set(true);
        let mut body = self.body.borrow_mut();
        let _ = write!(
            body,
            "<text x=\"{:.3}\" y=\"{:.3}\" font-family=\"Inter\" font-size=\"{:.3}\" fill=\"",
            x, y, size
        );
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

    fn apply_dpr_transform(&self, dpr: f64) {
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
            "SvgPainter::blit invoked — SVG export must always run via a Fresh-regime frame"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::SvgPainter;
    use crate::common::metrics;
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
    fn measure_text_width_uses_real_inter_metrics_not_declared_family() {
        // Export always renders (and measures) the bundled Inter font
        // regardless of what family the cell declares — "sans-serif" /
        // "serif" here must not change the result, only size does.
        let p = SvgPainter::new(10, 10);
        assert_eq!(
            p.measure_text_width("hello", "16px sans-serif"),
            metrics::inter_advance_width("hello", 16.0),
        );
        assert_eq!(
            p.measure_text_width("hi", "bold 12px serif"),
            metrics::inter_advance_width("hi", 12.0),
        );
        // No `<n>px` token -> DEFAULT_FONT_SIZE_PX (12.0).
        assert_eq!(
            p.measure_text_width("ab", "no-size"),
            metrics::inter_advance_width("ab", 12.0),
        );
        // Real glyph advances, not the old flat 1.0-factor estimate
        // (5 chars * 16px = 80.0) — regression guard against reverting to
        // `approx_text_width`.
        assert!(p.measure_text_width("hello", "16px sans-serif") < 80.0);
    }

    #[test]
    fn finish_embeds_inter_font_face_only_when_text_was_drawn() {
        let empty = SvgPainter::new(10, 10);
        empty.rect_fill(rect(0, 0, 5, 5), PaintColor::Static("#fff"));
        assert!(!empty.finish().contains("@font-face"));

        let with_text = SvgPainter::new(10, 10);
        with_text.fill_text(
            "hi",
            0.0,
            0.0,
            PaintColor::Static("12px Aptos Narrow"),
            PaintColor::Static("#000"),
            TextAlign::Start,
            TextBaseline::Alphabetic,
        );
        let svg = with_text.finish();
        // One embedded font is used for every cell, regardless of the
        // cell's own declared family (`Aptos Narrow` above) — that's what
        // keeps measured and rendered glyphs in agreement on any viewer.
        assert!(svg.contains("@font-face"));
        assert!(svg.contains("font-family:'Inter'"));
        assert!(svg.contains("data:font/truetype;base64,"));
        assert!(svg.contains("font-family=\"Inter\""));
        assert_eq!(
            svg.matches("@font-face").count(),
            1,
            "exactly one embedded font, not one per text element"
        );
    }

    #[test]
    fn dpr_scales_attr_dimensions_only() {
        let p = SvgPainter::new(100, 50);
        p.apply_dpr_transform(2.0);
        let svg = p.finish();
        assert!(svg.contains("width=\"200\""));
        assert!(svg.contains("height=\"100\""));
        assert!(svg.contains("viewBox=\"0 0 100 50\""));
    }
}
