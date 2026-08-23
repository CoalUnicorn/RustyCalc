//! Pixel- and cell-space primitives.
//!
//! Every visible artifact composes from [`pixel_rect::PixelRect`] and
//! [`prim::Line`]. The cell-address <-> pixel-rect mapping lives in
//! [`slot`]; layout values in [`constants`]; the Excel-style column
//! label helper in [`utils`].

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
    /// backing-store sizing remains a separate DPR-scaled truncation below.
    pub fn to_logical_extent(self) -> (i32, i32) {
        (self.w.ceil() as i32, self.h.ceil() as i32)
    }

    /// Physical backing-store dimensions from CSS size and DPR.
    /// Truncates fractional pixels — matches browser canvas rounding behaviour.
    pub fn to_backing_size(self, dpr: f64) -> (u32, u32) {
        ((self.w * dpr) as u32, (self.h * dpr) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::CanvasSize;

    #[test]
    fn logical_extent_rounds_outward() {
        let size = CanvasSize { w: 100.1, h: 50.9 };

        assert_eq!(size.to_logical_extent(), (101, 51));
    }

    #[test]
    fn backing_size_truncates_after_dpr_scaling() {
        let size = CanvasSize { w: 100.9, h: 50.9 };

        assert_eq!(size.to_backing_size(1.25), (126, 63));
    }
}
