//! Overlay-layer paints: selection rectangle, autofill drag preview,
//! clipboard marching ants, point-mode range, formula-ref highlights.
//!
//! These run on the transparent overlay canvas, which paints after the
//! grid layer (cells + borders + text + headers + corner). Each helper
//! bails early via `Chrome::range_rect(...)?` when the range is entirely
//! outside the drawable fold, so overlays never leak onto the canvas for
//! off-screen refs like `=BB3`.

use std::borrow::Cow;

use crate::chrome::Chrome;
use crate::geometry::constants::DASHED_BORDER_WIDTH;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::types::coord::RCRange;

pub(crate) mod autofill;
pub(crate) mod clipboard;
pub(crate) mod formula_refs;
pub(crate) mod point_mode;
pub(crate) mod selection;

/// Controls whether `draw_dashed_range` fills the interior with a light tint.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum DashFill {
    /// Outline only (used for clipboard marching ants).
    Outline,
    /// Outline + semi-transparent fill tint (used for point-mode range and
    /// formula refs). Carries a precomputed `rgba(...)` tint as
    /// `Cow<'static, str>` — built-in themes use `Cow::Borrowed` for the
    /// zero-alloc ptr-eq path; host-page themes carry the owned tint.
    Tinted(Cow<'static, str>),
}

impl<P: Painter> RendererCore<P> {
    /// Dashed rectangle over `range`. Used for clipboard marching ants
    /// (`DashFill::Outline`) and point-mode / formula-ref highlights
    /// (`DashFill::Tinted`, which also draws the carried 8% fill).
    pub(crate) fn draw_dashed_range(
        &self,
        frame: &Chrome,
        range: RCRange,
        color: PaintColor,
        fill: DashFill,
    ) {
        let Some(b) = frame.range_rect(range) else {
            return;
        };

        // Tint first so the dashed outline lands cleanly on top — the 8%
        // alpha would otherwise wash over the dashes and dim them.
        // `DashFill::Tinted` carries `Cow<'static, str>`; the helper
        // preserves the ptr-eq fast path for built-in themes.
        if let DashFill::Tinted(tint) = &fill {
            self.painter.rect_fill(b, PaintColor::from_theme_str(tint));
        }

        self.painter
            .rect_dashed(b, color, f64::from(DASHED_BORDER_WIDTH));
    }
}
