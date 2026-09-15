//! Renderer-owned cache layer. Three distinct lifetimes share this
//! folder because they all live on `RendererCore` and they all dedupe
//! work the streaming cell pipeline would otherwise repeat:
//!
//! | Lifetime              | Type                              | Resets via                                  |
//! | --------------------- | --------------------------------- | ------------------------------------------- |
//! | Per-call scratch      | [`FrameCache`]                    | `Cell::take` / `Cell::set` rhythm per pass  |
//! | Cross-frame model     | [`GridCache`] / [`SegmentBuffers`]| grid-wide buffer invalidation / owned commits |
//! | Renderer-lifetime     | [`FontIntern`], [`ColorIntern`]   | insert-only                                 |
//!
//! `font` is `pub(crate)` — pure CSS-string construction consumed by
//! [`FontIntern`] and by `autofit` (which must produce identical font
//! strings to those the renderer paints).
//!
//! Three modules also belong to this tier: `fingerprint` owns the
//! retained-pixel truth in [`GridCache`],
//! `layout_transition` classifies a candidate layout against the committed
//! one, and `color` decodes the model's decoration color for both the cell
//! paint step and the fingerprint digest.

pub(crate) mod color;
pub(crate) mod fingerprint;
pub(crate) mod font;
mod grid_cache;
mod intern;
pub(crate) mod layout_transition;
mod scratch;

#[cfg(test)]
pub(crate) mod test_support;

pub use grid_cache::{BufferTruth, GridCache, SegmentBuffers};
pub use intern::{ColorIntern, FontIntern};
pub use scratch::FrameCache;
