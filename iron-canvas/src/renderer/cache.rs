use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::renderer::cells::CellPaint;
use crate::renderer::text_paint::TextLine;
use crate::style::FontStyle;

pub(crate) struct FrameCache {
    /// Per-frame canvas state cache. Avoids redundant JS boundary crossings
    /// when adjacent cells share the same fill, stroke, font, or line width.
    /// `Cell<T>` allows mutation through `&self` so paint helpers keep their immutable signature.
    pub(crate) last_fill: Cell<CachedColor>,
    pub(crate) last_stroke: Cell<CachedColor>,
    pub(crate) last_font: Cell<CachedColor>,
    pub(crate) last_line_width: Cell<f64>,
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
    /// allocate a fresh `Vec<TextLine>` per cell with text. Each cell `clear()`s
    /// and refills; the inner per-line `String` capacities also survive.
    pub(crate) text_lines: Cell<Vec<TextLine>>,
}

/// Per-frame ctx-state cache entry. The `Static` arm carries `&'static str`
/// for theme-driven calls so cache misses skip the `to_string()` allocation
/// the previous `Cell<String>` cache forced. `Owned` keeps the dynamic path
/// (per-cell colors built from `CssColor::new`) intact.
#[derive(Default)]
pub(crate) enum CachedColor {
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
    /// dynamic -> static transition is detected correctly.
    pub(super) fn matches_static(&self, color: &'static str) -> bool {
        match self {
            CachedColor::Empty => false,
            CachedColor::Static(s) => std::ptr::eq(*s, color),
            CachedColor::Owned(s) => s == color,
        }
    }
}

/// Renderer-lifetime intern table for `ctx.font` strings.
///
/// `FontStyle::build` is the *only* allocation source on the per-cell text
/// path that doesn't depend on cell content. Realistic spreadsheets touch
/// fewer than ~10 unique (size, bold, italic, family) tuples, so a linear
/// scan beats a HashMap for the actual cardinality. Lives on `RendererCore`,
/// not `FrameCache`, because it is *cross-frame*: the same fonts repeat every
/// repaint.
pub(crate) struct FontIntern {
    entries: RefCell<Vec<(FontKey, Rc<str>)>>,
}

#[derive(PartialEq, Eq)]
struct FontKey {
    size_bits: u64,
    bold: bool,
    italic: bool,
    family: Box<str>,
}

impl FontIntern {
    pub(crate) fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Returns the interned `ctx.font` string for `(size_px, bold, italic, family)`.
    /// Cache hit: zero alloc, just an `Rc::clone`. Miss: one `FontStyle::build` +
    /// one `Box<str>` for the key family + one `Rc<str>` for the value.
    pub(crate) fn get_or_build(
        &self,
        size_px: f64,
        bold: bool,
        italic: bool,
        family: &str,
        fallback: &str,
    ) -> Rc<str> {
        let size_bits = size_px.to_bits();
        let mut entries = self.entries.borrow_mut();
        for (key, css) in entries.iter() {
            if key.size_bits == size_bits
                && key.bold == bold
                && key.italic == italic
                && &*key.family == family
            {
                return Rc::clone(css);
            }
        }
        let css: Rc<str> = FontStyle::build(size_px, bold, italic, family, fallback).into();
        entries.push((
            FontKey {
                size_bits,
                bold,
                italic,
                family: family.into(),
            },
            Rc::clone(&css),
        ));
        css
    }
}

/// Renderer-lifetime intern table for column-letter labels (`A`, `B`, ..., `XFD`).
///
/// Mirrors `FontIntern`: each unique column index pays one `col_name` allocation
/// the first time it scrolls into view, then header repaints `Rc::clone` instead.
/// Indexed by 1-based column number; entry 0 is the empty string returned by
/// `col_name(0)` so out-of-range queries don't blow up the lookup.
pub(crate) struct ColNameIntern {
    entries: RefCell<Vec<Rc<str>>>,
}

impl ColNameIntern {
    pub(crate) fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Interned label for column `col` (1-based). Grows the entry vec on demand
    /// up to `col`; subsequent calls are zero-alloc.
    pub(crate) fn get(&self, col: i32) -> Rc<str> {
        let idx = col.max(0) as usize;
        let mut entries = self.entries.borrow_mut();
        while entries.len() <= idx {
            let next = entries.len() as i32;
            entries.push(crate::geometry::utils::col_name(next).into());
        }
        Rc::clone(&entries[idx])
    }
}
