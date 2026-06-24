//! Resolved border data + paint passes.
//!
//! `ResolvedBorders` is the per-cell, per-edge `Option<BorderPaint>` carried
//! on `CellPaint`. The grid sub-pass paints left+top fallback strokes (gated
//! on show-grid + fill-suppression); the explicit sub-pass paints the four
//! `Option<BorderPaint>` edges. `paint_border` is the single pixel pusher —
//! `Double`-style borders draw as two parallel ±1 px strokes.
//!
//! Border resolution allocates at most once per edge with an explicit color
//! (`ColorIntern` makes it `Rc::clone` after the first sighting). Theme-grid
//! fallback edges use `Cow::Borrowed` clones — pointer-copy on built-in
//! themes, single `String::clone` on host-page themes.

use std::borrow::Cow;
use std::rc::Rc;

use crate::style::{Border, BorderItem, BorderStyle};

use super::paint::CellPaint;
use crate::geometry::constants::{MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::BorderEdge;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::cache::ColorIntern;
use crate::theme::CanvasTheme;

/// Per-edge `BorderPaint` resolved from a cell's `Borders` style. `None` on
/// an edge means the cell carries no explicit border there — the grid
/// sub-pass will fill the left/top edges with the theme grid line.
pub struct ResolvedBorders {
    pub left: Option<BorderPaint>,
    pub top: Option<BorderPaint>,
    pub right: Option<BorderPaint>,
    pub bottom: Option<BorderPaint>,
}

impl ResolvedBorders {
    /// Resolve every `Some` `BorderItem` on `border` into a `BorderPaint`.
    /// Zero allocations on edges that fall back to the theme grid color
    /// (`BorderColor::Static`); `Rc::clone` per edge with an explicit color
    /// (the renderer's `ColorIntern` absorbs the first-sighting alloc).
    pub(super) fn resolve(border: &Border, theme: &CanvasTheme, intern: &ColorIntern) -> Self {
        Self {
            left: border
                .left
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
            top: border
                .top
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
            right: border
                .right
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
            bottom: border
                .bottom
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
        }
    }
}

pub struct BorderStroke {
    pub width_px: i32,
    pub double: bool,
}

/// Color for a resolved border edge. `Static` carries the theme grid color as
/// `Cow<'static, str>` — built-in themes (`Cow::Borrowed`) ptr-eq through the
/// painter cache; host-page themes (`Cow::Owned`) clone the `String` once per
/// resolve and content-eq through the cache. `Owned` is the per-cell override
/// path, an interned `Rc<str>` from `ColorIntern` (`Rc::clone` after the first
/// sighting of each unique color).
pub enum BorderColor {
    Static(Cow<'static, str>),
    Owned(Rc<str>),
}

pub struct BorderPaint {
    pub color: BorderColor,
    pub stroke: BorderStroke,
}

impl BorderPaint {
    /// Thin grid-color stroke used as the left/top fallback when a cell has
    /// no explicit border on that edge. `Cow::clone` is a pointer copy for
    /// built-in themes and a `String::clone` for host-page themes.
    pub(super) fn grid_line(theme: &CanvasTheme) -> Self {
        Self {
            color: BorderColor::Static(theme.grid_color.clone()),
            stroke: BorderStroke {
                width_px: STANDARD_BORDER_WIDTH,
                double: false,
            },
        }
    }

    /// Resolve a `BorderItem` into a renderer-ready paint. Color falls back
    /// to `theme.grid_color` when the item carries no explicit color;
    /// `Cow::clone` keeps the built-in path zero-alloc. Explicit colors go
    /// through `ColorIntern` so they `Rc::clone` after the first sighting.
    fn resolve(item: &BorderItem, theme: &CanvasTheme, intern: &ColorIntern) -> Self {
        let color = match item.color.as_deref() {
            None => BorderColor::Static(theme.grid_color.clone()),
            Some(c) => BorderColor::Owned(intern.get(c)),
        };
        Self {
            color,
            stroke: BorderStroke::from_border_style(&item.style),
        }
    }
}

impl BorderStroke {
    /// Map `BorderStyle` -> pixel width + double-line flag.
    /// Dash/dot patterns render solid in v1 (no dash); width still follows the
    /// Medium / Thick / thin tier (e.g. `MediumDashed` keeps `MEDIUM_BORDER_WIDTH`).
    fn from_border_style(s: &BorderStyle) -> Self {
        match s {
            BorderStyle::Medium
            | BorderStyle::MediumDashed
            | BorderStyle::MediumDashDot
            | BorderStyle::MediumDashDotDot => Self {
                width_px: MEDIUM_BORDER_WIDTH,
                double: false,
            },
            BorderStyle::Thick => Self {
                width_px: THICK_BORDER_WIDTH,
                double: false,
            },
            BorderStyle::Double => Self {
                width_px: STANDARD_BORDER_WIDTH,
                double: true,
            },
            // foreign #[non_exhaustive]: BorderStyle is upstream (ironcalc).
            // Thin / Dotted / SlantDashDot / etc. fall through to standard solid.
            _ => Self {
                width_px: STANDARD_BORDER_WIDTH,
                double: false,
            },
        }
    }
}

impl<P: Painter> RendererCore<P> {
    /// Grid-fallback strokes on left+top, gated by show-grid + fill-suppression.
    /// Reads `p.borders` (pre-resolved at iteration time) so an explicit
    /// border on left/top still suppresses the grid stroke without re-walking
    /// `style.border`.
    /// `grid` is the theme's grid-line `BorderPaint`, resolved once per pass by
    /// the caller and shared across every cell — it's frame-invariant, so
    /// rebuilding it per slot only re-cloned `theme.grid_color` for nothing (B-3).
    pub(super) fn paint_borders_grid(&self, p: &CellPaint, grid: &BorderPaint) {
        if !self.frame_cache.show_grid.get() {
            return;
        }
        if p.style.fill_color.is_some() {
            return;
        }
        if p.borders.left.is_none() {
            self.paint_border(BorderEdge::Left, p.rect, grid);
        }
        if p.borders.top.is_none() {
            self.paint_border(BorderEdge::Top, p.rect, grid);
        }
    }

    /// Stroke pre-resolved explicit borders on the cell's four edges. Pure
    /// pixel pushing — no `BorderPaint::resolve` calls inside the loop.
    /// Run across every slot AFTER the grid sub-pass so an explicit right on
    /// cell A wins over cell B's grid left at the shared pixel column.
    pub(super) fn paint_borders_explicit(&self, p: &CellPaint) {
        if let Some(b) = &p.borders.left {
            self.paint_border(BorderEdge::Left, p.rect, b);
        }
        if let Some(b) = &p.borders.top {
            self.paint_border(BorderEdge::Top, p.rect, b);
        }
        if let Some(b) = &p.borders.right {
            self.paint_border(BorderEdge::Right, p.rect, b);
        }
        if let Some(b) = &p.borders.bottom {
            self.paint_border(BorderEdge::Bottom, p.rect, b);
        }
    }

    /// Single-cell border paint used by `repaint_active_cell` where there
    /// are no neighbour interactions to worry about. Composes the two
    /// sub-passes in their canonical order: grid fallback first, explicit
    /// over the top.
    pub(super) fn paint_borders(&self, p: &CellPaint, theme: &CanvasTheme) {
        self.paint_borders_grid(p, &BorderPaint::grid_line(theme));
        self.paint_borders_explicit(p);
    }

    /// Stroke one resolved border. `Double`-style borders render as two
    /// parallel strokes offset ±1 px on the cross-axis.
    fn paint_border(&self, edge: BorderEdge, rect: PixelRect, b: &BorderPaint) {
        // Extend each edge by half its width so perpendicular borders overlap
        // at the corner instead of leaving a butt-cap notch. With the painter's
        // parity-aware pixel snap, `width_px / 2` is the exact reach of the
        // crossing edge's band from its centerline (0 for 1-px borders, which
        // already meet cleanly).
        let line = edge.line(rect).extend(b.stroke.width_px / 2);
        let offsets: &[i32] = if b.stroke.double { &[-1, 1] } else { &[0] };
        let color = match &b.color {
            BorderColor::Static(s) => PaintColor::from_theme_str(s),
            BorderColor::Owned(s) => PaintColor::Borrowed(s),
        };
        for &d in offsets {
            self.painter
                .stroke_line(line.offset_cross(d), color, f64::from(b.stroke.width_px));
        }
    }
}
