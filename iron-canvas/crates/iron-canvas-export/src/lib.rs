//! Multi-format export backend for `iron-canvas`.
//!
//! Each format lives behind its own feature flag and contributes a
//! `Painter + BlitPainter + TextMetrics` adapter plus a `Surface` impl
//! that drives a throwaway `Orchestrator`.

pub mod common;

#[cfg(feature = "svg")]
pub mod svg;

#[cfg(feature = "svg")]
pub use svg::{SvgPainter, SvgSurface};
