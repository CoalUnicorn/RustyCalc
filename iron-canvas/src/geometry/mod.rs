pub mod constants;
pub mod frame;
pub mod pixel_rect;
pub mod prim;
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
    pub(crate) fn to_backing_size(self, dpr: f64) -> (u32, u32) {
        ((self.w * dpr) as u32, (self.h * dpr) as u32)
    }
}
