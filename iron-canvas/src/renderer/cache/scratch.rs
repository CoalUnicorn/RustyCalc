//! Per-call scratch buffers parked on `RendererCore` so the streaming
//! cell-paint pipeline can `take` / `set` instead of allocating per cell.
//!
//! Lifetime is "across frames as `Vec` capacity, not across frames as
//! content": the buffer Vecs survive frame boundaries to keep allocations
//! warm, but their contents are clobbered on every `take`. Distinct from
//! [`super::pane_cache::PaneCache`], whose contents *do* survive frames
//! and feed paint-skipping.

use std::cell::{Cell, RefCell};

use super::super::cell::text::TextLine;
use super::super::cell::CellPaint;

pub(crate) struct FrameCache {
    /// Scratch buffer parking each pane's resolved `CellPaint`s during the
    /// streaming bg pass so the deferred border + text passes can iterate
    /// them without re-querying the model. Reused across pane calls to
    /// avoid 4 Vec allocations per frame.
    pub(crate) text_slots: Cell<Vec<CellPaint>>,
    /// Per-frame cache of the active sheet's `get_show_grid_lines` flag.
    /// Set once at the top of `render_grid`; read per-cell by `paint_borders`
    /// to gate the right/bottom grid-line fallback. Avoids a model call per
    /// cell on the hot pane walk.
    pub(crate) show_grid: Cell<bool>,
    /// Scratch String formatted into for row-header labels (`write!` instead
    /// of `i32::to_string()`). Cleared per-call; capacity persists across
    /// frames so steady-state row-label paints don't re-allocate.
    pub(crate) label_buf: RefCell<String>,
    /// Scratch line buffer parked here so `TextPaint::resolve_into` doesn't
    /// allocate a fresh `Vec<TextLine>` per cell with text. `layout_into`
    /// overwrites slots in place via a counter and `truncate`s the tail, so
    /// inner `String` capacities for slots `[0..line_count)` survive across
    /// cells. Slots beyond the count are dropped on shrink.
    pub(crate) text_lines: Cell<Vec<TextLine>>,
    /// Scratch line-builder for the wrap path. `layout_into` reuses this
    /// `String` across every wrapped raw-line of every cell, so the wrap
    /// branch is alloc-free in steady state. Renderer-lifetime, not per-cell.
    pub(crate) wrap_buf: RefCell<String>,
}
