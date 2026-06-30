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
    /// Physical backing-store dimensions from CSS size and DPR.
    /// Truncates fractional pixels — matches browser canvas rounding behaviour.
    pub fn to_backing_size(self, dpr: i32) -> (u32, u32) {
        (
            (self.w * f64::from(dpr)) as u32,
            (self.h * f64::from(dpr)) as u32,
        )
    }
}
