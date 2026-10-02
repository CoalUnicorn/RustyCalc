//! Drawing backend abstraction.
//!
//! `Painter` is the surface every renderer paint method calls into. The
//! trait surface lives here; concrete impls live in sibling adapter
//! crates: `CanvasPainter` in `iron-canvas-web`, `SvgPainter` in
//! `iron-canvas-export`, `RecorderPainter` in `iron-canvas-recorder`.
//!
//! `TextMetrics` is a separate supertrait because text measurement is
//! consumed outside the paint loop (e.g. for column-fit calculations)
//! and must stay callable without a paint-time context.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::Span;
use crate::geometry::path::Path;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Line;

/// Color/font argument for the `Painter` surface. The `Static` variant carries
/// a `&'static str` whose address is stable for the program lifetime, so the
/// Canvas-2D backend can ptr-eq it against its cache without ever allocating
/// a comparison `String`. `Borrowed` is the per-cell-owned path (cell fill
/// override, custom border color, interned font CSS) and falls back to
/// content-eq + `String` cache.
#[derive(Copy, Clone)]
pub enum PaintColor<'a> {
    Static(&'static str),
    Borrowed(&'a str),
}

impl<'a> PaintColor<'a> {
    pub fn as_str(&self) -> &str {
        match self {
            PaintColor::Static(s) => s,
            PaintColor::Borrowed(s) => s,
        }
    }

    /// Lift a theme color (`Cow<'static, str>`) into a `PaintColor`. Built-in
    /// themes carry `Cow::Borrowed(&'static str)` and route through `Static`,
    /// preserving the painter's ptr-eq fast path. Host-page themes carry
    /// `Cow::Owned(String)` and route through `Borrowed`, falling back to the
    /// content-eq cache.
    #[allow(clippy::ptr_arg)]
    pub fn from_theme_str(s: &'a Cow<'static, str>) -> PaintColor<'a> {
        match s {
            Cow::Borrowed(s) => PaintColor::Static(s),
            Cow::Owned(s) => PaintColor::Borrowed(s.as_str()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

/// Typed group label for `Painter::begin_group`. Enumerates the layers and
/// sub-sections the renderer brackets — SVG emits `<g class="...">` with the
/// kebab-case form, the recorder serializes it through serde, the Canvas-2D
/// backend no-ops on it. Closed set: a typed enum lets the recorder's
/// `skip_groups` filter compare by variant rather than string content, and
/// keeps the SVG class names disciplined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GroupClass {
    Grid,
    Overlay,
    Cells,
    FrozenSep,
    Headers,
    Corner,
    SelectionFill,
    SelectionStroke,
    Autofill,
    Clipboard,
    PointMode,
    FormulaRefs,
    ActiveCellRepaint,
    HeaderHighlights,
    /// Consumer band (`Orchestrator::add_decoration`) — every custom
    /// decoration shares this bracket.
    Custom,
}

impl GroupClass {
    pub fn as_str(self) -> &'static str {
        match self {
            GroupClass::Grid => "grid",
            GroupClass::Overlay => "overlay",
            GroupClass::Cells => "cells",
            GroupClass::FrozenSep => "frozen-sep",
            GroupClass::Headers => "headers",
            GroupClass::Corner => "corner",
            GroupClass::SelectionFill => "selection-fill",
            GroupClass::SelectionStroke => "selection-stroke",
            GroupClass::Autofill => "autofill",
            GroupClass::Clipboard => "clipboard",
            GroupClass::PointMode => "point-mode",
            GroupClass::FormulaRefs => "formula-refs",
            GroupClass::ActiveCellRepaint => "active-cell-repaint",
            GroupClass::HeaderHighlights => "header-highlights",
            GroupClass::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextBaseline {
    Top,
    Middle,
    Bottom,
    Alphabetic,
}

/// Per-char width as a fraction of font size — the deterministic glyph-width
/// estimate shared by every backend without a host text metric (SVG, PDF,
/// Recorder) and by `layout_into`'s measure-fallback. One value keeps wrap
/// math identical across all non-browser surfaces (measured == painted).
pub const CHAR_WIDTH_FACTOR: f64 = 1.0;

/// Deterministic text-width estimate: `chars × font_size_px × CHAR_WIDTH_FACTOR`.
/// The single fallback every measureless `TextMetrics` backend serializes; each
/// backend parses `font_size_px` via [`parse_font_size_px`] before calling.
pub fn approx_text_width(font_size_px: f64, text: &str) -> f64 {
    text.chars().count() as f64 * font_size_px * CHAR_WIDTH_FACTOR
}

/// Default size when a CSS `font` shorthand carries no `<n>px` token.
pub const DEFAULT_FONT_SIZE_PX: f64 = 12.0;

/// Extract the first `<n>px` token from a CSS `font` shorthand, falling back to
/// [`DEFAULT_FONT_SIZE_PX`] when none is present. The single size parser every
/// measureless backend (SVG, PDF, Recorder) shares so they agree on wrap math.
pub fn parse_font_size_px(font_css: &str) -> f64 {
    font_css
        .split_whitespace()
        .find_map(|tok| tok.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()))
        .unwrap_or(DEFAULT_FONT_SIZE_PX)
}

pub trait TextMetrics {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64;
}

/// How a stroke ends an open subpath or an independent segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// How a stroke joins two segments of a subpath.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

/// Stroke appearance for [`Painter::stroke_path`].
///
/// `dash` is empty for a solid stroke. A nonempty pattern is in the same units
/// as `width`, and its phase is zero. An odd-length pattern repeats twice to
/// form an even pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle<'a> {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f64,
    pub dash: &'a [f64],
}

impl StrokeStyle<'_> {
    /// `width` is finite and positive, `miter_limit` is finite and at least 1,
    /// every dash entry is finite and nonnegative, and a nonempty dash pattern
    /// has a finite positive sum (so an all-zero pattern is invalid).
    pub fn is_valid(&self) -> bool {
        self.width.is_finite()
            && self.width > 0.0
            && self.miter_limit.is_finite()
            && self.miter_limit >= 1.0
            && self.dash.iter().all(|d| d.is_finite() && *d >= 0.0)
            && (self.dash.is_empty() || {
                let sum: f64 = self.dash.iter().sum();
                sum.is_finite() && sum > 0.0
            })
    }
}

#[diagnostic::on_unimplemented(
    note = "implement the full `Painter` drawing surface (rect/circle/path fills and strokes, clears, borders, text). Reference impls: `CanvasPainter` (iron-canvas-canvas2d), `SvgPainter` and `PdfPainter` (iron-canvas-export), `RecorderPainter` (iron-canvas-recorder)"
)]
pub trait Painter: TextMetrics {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor);
    /// Fill `rect` with a horizontal linear gradient: `from` at the rect's
    /// left edge, `to` at its right edge. A zero-width or zero-height rect is
    /// a no-op. The renderer uses this for gradient data bars; every backend
    /// implements it with its native gradient (Canvas2D
    /// `createLinearGradient`, SVG `linearGradient`, PDF axial shading),
    /// never a browser-only shortcut or a banded approximation.
    fn rect_fill_hgradient(&self, rect: PixelRect, from: PaintColor, to: PaintColor);
    /// Fill the float path with `color`, in logical pixel space.
    ///
    /// Uses the nonzero winding rule; fill implicitly closes each open
    /// subpath. An empty path, a subpath with only a `Move`, and an invalid
    /// path (non-finite coordinate, segment without a current subpath, or a
    /// bad arc) are no-ops. The path can carry lines, quadratic and cubic
    /// curves, and circular arcs, so a shape reaches every backend as a
    /// curve. The existing [`Self::fill_circle`] primitive stays available.
    fn fill_path(&self, path: &Path<'_>, color: PaintColor);
    /// Stroke the float path with `color` and `style`.
    ///
    /// A path invalid under [`Path::validate`] or a style invalid under
    /// [`StrokeStyle::is_valid`] paints nothing. Stroke closes only subpaths
    /// that carry a `Close`; an open subpath stays open.
    ///
    /// The call must not change how any later painter operation draws. The
    /// implementation sets color, width, cap, join, miter limit, and dash for
    /// this call only; a solid path installs an empty dash pattern explicitly.
    /// Every call starts a fresh path.
    fn stroke_path(&self, path: &Path<'_>, color: PaintColor, style: &StrokeStyle<'_>);
    /// Fill the circle centred at `(cx, cy)` with the given `radius`, both in
    /// logical pixels. A non-positive radius is a no-op. Every backend draws
    /// it with its own curve primitive (Canvas2D `arc`, SVG `<circle>`, PDF
    /// four cubic Béziers), so the edge stays smooth at any zoom or device
    /// pixel ratio. The renderer uses this for the round conditional-
    /// formatting glyphs; [`Self::fill_path`] rounds to integer points and
    /// shows a 24-gon instead.
    fn fill_circle(&self, cx: f64, cy: f64, radius: f64, color: PaintColor);
    /// Clear the pixels under `rect` to fully transparent. Canvas-2D maps
    /// to `ctx.clearRect`; backends that don't compose alpha (SVG, Recorder)
    /// may no-op.
    fn clear_rect(&self, rect: PixelRect);
    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64);
    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64);
    fn stroke_line(&self, line: Line, color: PaintColor, width: f64);
    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64);
    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64);
    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64);
    fn push_clip(&self, rect: PixelRect);
    fn pop_clip(&self);
    #[allow(clippy::too_many_arguments)]
    fn fill_text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    );
    fn invalidate_cache(&self);

    /// Sync the backend's coordinate system to the device pixel ratio.
    /// Called by `LayerBase::resize` after a canvas resize. Canvas-2D resets
    /// the transform and applies a DPR scale; SVG/Recorder backends can
    /// no-op or stash the value internally.
    fn apply_dpr_transform(&self, dpr: f64);

    /// Restore sticky text-alignment defaults. Canvas-2D resets these on
    /// `set_width/set_height`; this hook is called after `invalidate_cache`
    /// so renderer code stays backend-agnostic. SVG/Recorder backends can
    /// no-op.
    fn reset_text_defaults(&self);

    /// Open a named group around subsequent draws. SVG emits `<g class="..">`,
    /// Recorder logs an op, Canvas-2D no-ops. The renderer brackets
    /// `render_grid` / `paint_overlay_layer` so SVG output is structured per layer.
    fn begin_group(&self, class: GroupClass);
    fn end_group(&self);
}

/// Backends that can copy a rectangle of already-painted pixels in place.
/// Split out of `Painter` so the scroll-blit dispatch becomes a compile-time
/// trait bound rather than a runtime capability check; SVG simply omits the
/// impl. `src` addresses the DPR-scaled backing store and the backend
/// multiplies on its side — `dst` flows through the active DPR transform.
pub trait BlitPainter: Painter {
    fn blit(&self, src: PixelRect, dst: PixelRect);
}

pub struct CssColor(String);

impl CssColor {
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() {
            Self("#000000".to_owned())
        } else {
            Self(s.to_lowercase())
        }
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod stroke_style_tests {
    use super::*;

    fn style(width: f64, miter_limit: f64, dash: &[f64]) -> StrokeStyle<'_> {
        StrokeStyle {
            width,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit,
            dash,
        }
    }

    #[test]
    fn solid_default_is_valid() {
        assert!(style(2.0, 10.0, &[]).is_valid());
    }

    #[test]
    fn nonpositive_or_nonfinite_width_is_invalid() {
        assert!(!style(0.0, 10.0, &[]).is_valid());
        assert!(!style(-1.0, 10.0, &[]).is_valid());
        assert!(!style(f64::NAN, 10.0, &[]).is_valid());
    }

    #[test]
    fn miter_limit_below_one_is_invalid() {
        assert!(!style(1.0, 0.9, &[]).is_valid());
        assert!(!style(1.0, f64::NAN, &[]).is_valid());
    }

    #[test]
    fn dash_entries_must_be_finite_nonnegative() {
        assert!(!style(1.0, 10.0, &[2.0, -1.0]).is_valid());
        assert!(!style(1.0, 10.0, &[2.0, f64::INFINITY]).is_valid());
    }

    #[test]
    fn all_zero_dash_is_invalid_but_positive_and_odd_patterns_are_valid() {
        assert!(!style(1.0, 10.0, &[0.0, 0.0]).is_valid());
        assert!(style(1.0, 10.0, &[3.0, 1.0]).is_valid());
        // An odd-length pattern is valid; it repeats twice when rendered.
        assert!(style(1.0, 10.0, &[4.0, 2.0, 1.0]).is_valid());
    }
}
