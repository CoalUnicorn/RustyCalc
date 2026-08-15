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

pub(crate) mod font;
mod grid_cache;
mod intern;
mod scratch;

pub use grid_cache::{BufferTruth, GridCache, SegmentBuffers};
pub use intern::{ColorIntern, FontIntern};
pub use scratch::FrameCache;
