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
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Line, Point};

pub mod shapes;
pub use shapes::PainterShapes;

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

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Start,
    Center,
    End,
}

/// Typed group label for `Painter::begin_group`. Enumerates the layers and
/// sub-sections the renderer brackets — SVG emits `<g class="…">` with the
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

#[allow(dead_code)]
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
/// backend parses `font_size_px` from its own CSS shorthand before calling.
pub fn approx_text_width(font_size_px: f64, text: &str) -> f64 {
    text.chars().count() as f64 * font_size_px * CHAR_WIDTH_FACTOR
}

pub trait TextMetrics {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64;
}

#[diagnostic::on_unimplemented(
    note = "see the canvas-patterns skill for the full `Painter` method surface (paint_*, blit, text) — reference impls live in CanvasPainter (web), SvgPainter, PdfPainter, RecorderPainter"
)]
pub trait Painter: TextMetrics {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor);
    /// Fill the closed polygon defined by `points`, in pixel space. The path
    /// implicitly closes from `points.last()` to `points.first()`. Empty or
    /// single-point input is a no-op.
    fn fill_path(&self, points: &[Point], color: PaintColor);
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
    fn apply_dpr_transform(&self, dpr: i32);

    /// Restore sticky text-alignment defaults. Canvas-2D resets these on
    /// `set_width/set_height`; this hook is called after `invalidate_cache`
    /// so renderer code stays backend-agnostic. SVG/Recorder backends can
    /// no-op.
    fn reset_text_defaults(&self);

    /// Open a named group around subsequent draws. SVG emits `<g class="..">`,
    /// Recorder logs an op, Canvas-2D no-ops. The renderer brackets
    /// `render_grid` / `render_overlays` so SVG output is structured per layer.
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
