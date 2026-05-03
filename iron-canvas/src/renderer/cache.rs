use std::cell::Cell;

use crate::renderer::cells::CellPaint;

pub(super) struct FrameCache {
    /// Per-frame canvas state cache. Avoids redundant JS boundary crossings
    /// when adjacent cells share the same fill, stroke, font, or line width.
    /// `Cell<T>` allows mutation through `&self` so paint helpers keep their immutable signature.
    pub(crate) last_fill: Cell<CachedColor>,
    pub(crate) last_stroke: Cell<CachedColor>,
    pub(crate) last_font: Cell<CachedColor>,
    pub(super) last_line_width: Cell<f64>,
    /// Scratch buffer parking each pane's resolved `CellPaint`s during the
    /// streaming bg pass so the deferred border + text passes can iterate
    /// them without re-querying the model. Reused across pane calls to
    /// avoid 4 Vec allocations per frame.
    pub(super) text_slots: Cell<Vec<CellPaint>>,
    /// Per-frame cache of the active sheet's `get_show_grid_lines` flag.
    /// Set once at the top of `render_grid`; read per-cell by `paint_borders`
    /// to gate the right/bottom grid-line fallback. Avoids a model call per
    /// cell on the hot pane walk.
    pub(super) show_grid: Cell<bool>,
}

/// Per-frame ctx-state cache entry. The `Static` arm carries `&'static str`
/// for theme-driven calls so cache misses skip the `to_string()` allocation
/// the previous `Cell<String>` cache forced. `Owned` keeps the dynamic path
/// (per-cell colors built from `CssColor::new`) intact.
#[derive(Default)]
pub(super) enum CachedColor {
    #[default]
    Empty,
    Static(&'static str),
    Owned(String),
}

impl CachedColor {
    /// Compare against an arbitrary `&str` without forcing a new allocation
    /// on a cache hit. `Static` and `Owned` both fall back to value compare.
    pub(super) fn matches(&self, color: &str) -> bool {
        match self {
            CachedColor::Empty => false,
            CachedColor::Static(s) => *s == color,
            CachedColor::Owned(s) => s == color,
        }
    }

    /// Pointer-equality compare against a `&'static str`. The two `Static`
    /// arms are cheap; `Owned` still falls back to value compare so a
    /// dynamic→static transition is detected correctly.
    pub(super) fn matches_static(&self, color: &'static str) -> bool {
        match self {
            CachedColor::Empty => false,
            CachedColor::Static(s) => std::ptr::eq(*s, color),
            CachedColor::Owned(s) => s == color,
        }
    }
}
