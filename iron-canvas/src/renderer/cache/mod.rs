//! Renderer-owned cache layer. Three distinct lifetimes share this
//! folder because they all live on `RendererCore` and they all dedupe
//! work the streaming cell pipeline would otherwise repeat:
//!
//! | Lifetime              | Type                              | Resets via                                  |
//! | --------------------- | --------------------------------- | ------------------------------------------- |
//! | Per-call scratch      | [`FrameCache`]                    | `Cell::take` / `Cell::set` rhythm per pass  |
//! | Cross-frame model     | [`PaneCache`] / [`PaneBuffers`]   | `invalidate(mask)` / `try_shift(..)` (blit) |
//! | Renderer-lifetime     | [`FontIntern`], [`ColNameIntern`], [`ColorIntern`] | insert-only                           |
//!
//! `font` is module-private — pure CSS-string construction consumed only
//! by [`FontIntern`]. Splitting it keeps the formatting concern separable
//! from the deduplication concern without exposing either upstream.

mod font;
mod intern;
mod pane_cache;
mod scratch;

pub(crate) use intern::{ColNameIntern, ColorIntern, FontIntern};
pub(crate) use pane_cache::PaneCache;
pub(crate) use scratch::FrameCache;
