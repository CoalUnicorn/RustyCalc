//! Pixel- and cell-space primitives.
//!
//! Every visible artifact composes from [`pixel_rect::PixelRect`] and
//! [`prim::Line`]. The cell-address <-> pixel-rect mapping lives in
//! [`slot`]; layout values in [`constants`]; the canvas extent in
//! [`extent`]; the Excel-style column label helper in [`labels`].

pub mod constants;
pub mod extent;
pub mod labels;
pub mod pixel_rect;
pub mod prim;
pub mod slot;

pub use extent::{CanvasMetricError, CanvasMetrics, CanvasSize};
