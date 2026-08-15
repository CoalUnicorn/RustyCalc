//! Per-call scratch buffers parked on `RendererCore` so the streaming
//! cell-paint pipeline can `take` / `set` instead of allocating per cell.
//!
//! Lifetime is "across frames as `Vec` capacity, not across frames as
//! content": the buffer Vecs survive frame boundaries to keep allocations
//! warm, but their contents are clobbered on every `take`. Distinct from
//! [`super::grid_cache::GridCache`], whose contents *do* survive frames
//! and feed paint-skipping.

use std::cell::{Cell, RefCell};

use super::super::cell::CellPaint;
use super::super::cell::text::TextLine;
use crate::renderer::prepared::FetchedCells;

pub struct FrameCache {
    /// Scratch buffer parking each pane's resolved `CellPaint`s during the
    /// streaming bg pass so the deferred border + text passes can iterate
    /// them without re-querying the model. Reused across pane calls to
    /// avoid 4 Vec allocations per frame.
    pub text_slots: Cell<Vec<CellPaint>>,
    /// Per-frame cache of the active sheet's `get_show_grid_lines` flag.
    /// Set once at the top of `render_grid`; read per-cell by `paint_borders_grid`
    /// to gate the left/top grid-line fallback. Avoids a model call per
    /// cell on the hot pane walk.
    pub show_grid: Cell<bool>,
    /// Scratch line buffer parked here so `TextPaint::resolve_into` doesn't
    /// allocate a fresh `Vec<TextLine>` per cell with text. `layout_into`
    /// overwrites slots in place via a counter and `truncate`s the tail, so
    /// inner `String` capacities for slots `[0..line_count)` survive across
    /// cells. Slots beyond the count are dropped on shrink.
    pub text_lines: Cell<Vec<TextLine>>,
    /// Scratch line-builder for the wrap path. `layout_into` reuses this
    /// `String` across every wrapped raw-line of every cell, so the wrap
    /// branch is alloc-free in steady state. Renderer-lifetime, not per-cell.
    pub wrap_buf: RefCell<String>,
    /// Strip-fetch scratch: the freshly-revealed subrange's bulk-fetch
    /// output for the Damage path's strip fetch+paint
    /// (`RendererCore::prepare_damage_pane` / `execute_damage_pane`) —
    /// reused by every strip of a multi-span Damage preparation — drained
    /// into the pane buffers by `splice_strip_into` and parked back here
    /// afterward, including when a later strip aborts the batch.
    /// Parked here (not in committed `SegmentBuffers`, whose contents survive
    /// frames) so the strip path reuses one warm bundle instead of
    /// `FetchedCells::default()`-ing (four fresh `Vec`s) per scroll.
    /// `pub(crate)`, not `pub` like this struct's other fields — `FetchedCells`
    /// itself is `pub(crate)` (an execution detail, not consumer API), so a
    /// field of that type can be no more visible than its own type.
    pub(crate) strip_scratch: RefCell<Vec<FetchedCells>>,
}
