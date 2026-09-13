//! Pixel- and cell-space primitives.
//!
//! Every visible artifact composes from [`pixel_rect::PixelRect`] and
//! [`prim::Line`]. The cell-address <-> pixel-rect mapping lives in
//! [`slot`]; layout values in [`constants`]; the Excel-style column
//! label helper in [`utils`].
//!
//! This module root defines the canvas extent itself: [`CanvasSize`], the
//! permissive logical (CSS) size, and [`CanvasMetrics`], the size and DPR pair
//! parsed once at the host boundary, with [`CanvasMetricError`] as its parse
//! failure. They stay at the root — not in a submodule of their own — because
//! every geometry walk, hit test, backend, and export path names them.

pub mod constants;
pub mod pixel_rect;
pub mod prim;
pub mod slot;
pub mod utils;

/// Size of the drawable canvas in logical (CSS) pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasSize {
    pub w: f64,
    pub h: f64,
}

impl CanvasSize {
    /// Integer logical-pixel bounds used by painters and geometry walks.
    ///
    /// Rounds outward so a fractional CSS extent is fully covered. Physical
    /// backing-store sizing lives on [`CanvasMetrics`], which validates the
    /// extent and DPR before any cast.
    pub fn to_logical_extent(self) -> (i32, i32) {
        (self.w.ceil() as i32, self.h.ceil() as i32)
    }
}

/// Why a canvas size and DPR pair cannot become [`CanvasMetrics`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasMetricError {
    /// The logical width or height is not a finite, non-negative value that
    /// fits an `i32` logical extent. A `NaN` or negative extent poisons every
    /// geometry walk derived from it; an extent past `i32::MAX` overflows the
    /// outward rounding.
    Size,
    /// The device pixel ratio is not finite and greater than zero. A zero,
    /// negative, or `NaN` DPR makes `size * dpr` meaningless, and the
    /// repaint path's backing-store alignment would have no fixed point.
    Dpr,
    /// The DPR-scaled backing store does not fit the backend's `u32`
    /// dimensions. Accepting it would leave the browser canvas at a size the
    /// geometry no longer describes.
    BackingOverflow,
}

impl std::fmt::Display for CanvasMetricError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Size => "canvas width and height must be finite, non-negative, and within i32",
            Self::Dpr => "device pixel ratio must be finite and greater than zero",
            Self::BackingOverflow => "canvas backing store must fit u32 after the DPR scale",
        };
        f.write_str(message)
    }
}

impl std::error::Error for CanvasMetricError {}

/// Validated canvas metrics: a logical size and DPR that may safely become
/// backing-store dimensions.
///
/// [`CanvasSize`] stays permissive on purpose — it is the general coordinate
/// value the geometry and hit-testing layers pass around, and the datagrid,
/// export, and diagnostics paths all hand-construct it. The host boundary is
/// different: a `NaN` extent, a zero/negative/`NaN` DPR, or a DPR-scaled
/// backing store past `u32` all produce a canvas whose pixels no length of
/// slot arithmetic can describe. This type is that parse. It is built once per
/// resize or recording load and then carried — `Orchestrator`, every `Surface`,
/// and `FrameInputs` all hold it — so no downstream cast needs to re-check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasMetrics {
    size: CanvasSize,
    dpr: f64,
}

impl CanvasMetrics {
    /// Parse a size/DPR pair. This is the only public constructor; there is no
    /// unchecked `CanvasMetrics { .. }` literal outside this module.
    pub fn new(size: CanvasSize, dpr: f64) -> Result<Self, CanvasMetricError> {
        let max_extent = f64::from(i32::MAX);
        if !size.w.is_finite()
            || !size.h.is_finite()
            || size.w < 0.0
            || size.h < 0.0
            || size.w > max_extent
            || size.h > max_extent
        {
            return Err(CanvasMetricError::Size);
        }
        if !dpr.is_finite() || dpr <= 0.0 {
            return Err(CanvasMetricError::Dpr);
        }
        let (backing_w, backing_h) = scale_to_backing(size, dpr);
        if backing_w > f64::from(u32::MAX) || backing_h > f64::from(u32::MAX) {
            return Err(CanvasMetricError::BackingOverflow);
        }
        Ok(Self { size, dpr })
    }

    /// Metrics for a paint attempt that ran before any `resize`: a zero-size
    /// logical canvas at DPR 1.0, exactly the pre-validation defaults. Valid by
    /// construction (0 is finite and non-negative; 1.0 is finite and positive),
    /// so it is a real constructor rather than an unchecked literal.
    pub(crate) fn unresized() -> Self {
        Self {
            size: CanvasSize { w: 0.0, h: 0.0 },
            dpr: 1.0,
        }
    }

    /// The validated logical (CSS) size.
    pub fn size(self) -> CanvasSize {
        self.size
    }

    /// The validated device pixel ratio.
    pub fn dpr(self) -> f64 {
        self.dpr
    }

    /// Integer logical-pixel bounds, as [`CanvasSize::to_logical_extent`].
    pub fn logical_extent(self) -> (i32, i32) {
        self.size.to_logical_extent()
    }

    /// Physical backing-store dimensions from the validated size and DPR.
    /// Truncates fractional pixels — matches browser canvas rounding — and
    /// cannot saturate, because [`Self::new`] rejected any overflowing pair.
    pub fn backing_size(self) -> (u32, u32) {
        let (w, h) = scale_to_backing(self.size, self.dpr);
        (w as u32, h as u32)
    }
}

/// DPR-scaled backing dimensions, still in `f64` so the caller can range-check
/// before the `u32` cast.
fn scale_to_backing(size: CanvasSize, dpr: f64) -> (f64, f64) {
    (size.w * dpr, size.h * dpr)
}

#[cfg(test)]
mod tests {
    use super::{CanvasMetricError, CanvasMetrics, CanvasSize};

    #[test]
    fn logical_extent_rounds_outward() {
        let size = CanvasSize { w: 100.1, h: 50.9 };

        assert_eq!(size.to_logical_extent(), (101, 51));
    }

    #[test]
    fn backing_size_truncates_after_dpr_scaling() {
        let metrics = CanvasMetrics::new(CanvasSize { w: 100.9, h: 50.9 }, 1.25)
            .expect("100.9 x 50.9 at DPR 1.25 is a valid metric pair");

        assert_eq!(metrics.backing_size(), (126, 63));
    }

    #[test]
    fn invalid_sizes_and_dprs_are_rejected() {
        let ok = CanvasSize { w: 100.0, h: 50.0 };
        for bad in [
            CanvasSize {
                w: f64::NAN,
                h: 50.0,
            },
            CanvasSize {
                w: 100.0,
                h: f64::INFINITY,
            },
            CanvasSize { w: -1.0, h: 50.0 },
            CanvasSize {
                w: 100.0,
                h: -0.0 - 1.0,
            },
            CanvasSize {
                w: f64::from(i32::MAX) + 1.0,
                h: 50.0,
            },
        ] {
            assert_eq!(
                CanvasMetrics::new(bad, 1.0).unwrap_err(),
                CanvasMetricError::Size,
                "{bad:?}"
            );
        }
        for bad_dpr in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
            assert_eq!(
                CanvasMetrics::new(ok, bad_dpr).unwrap_err(),
                CanvasMetricError::Dpr,
                "{bad_dpr}"
            );
        }
        // A finite pair whose DPR-scaled backing store leaves `u32`.
        assert_eq!(
            CanvasMetrics::new(ok, f64::from(u32::MAX)).unwrap_err(),
            CanvasMetricError::BackingOverflow
        );
    }

    /// Zero stays a legal metric: a hidden or not-yet-measured canvas reports
    /// a zero extent, and the geometry must still be able to describe it.
    #[test]
    fn zero_and_max_extents_are_accepted() {
        let zero = CanvasMetrics::new(CanvasSize { w: 0.0, h: 0.0 }, 1.0)
            .expect("a zero-size canvas is valid");
        assert_eq!(zero.backing_size(), (0, 0));

        let max = CanvasMetrics::new(
            CanvasSize {
                w: f64::from(i32::MAX),
                h: 1.0,
            },
            1.0,
        )
        .expect("the largest logical extent is valid at DPR 1");
        assert_eq!(max.logical_extent(), (i32::MAX, 1));
    }
}
