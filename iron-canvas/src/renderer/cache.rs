use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ironcalc_base::types::{CellType, Style};

use crate::renderer::cells::CellPaint;
use crate::renderer::style::FontStyle;
use crate::renderer::text_paint::TextLine;
use crate::types::coord::CssColor;

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
    /// Dense, row-major styles for the current pane's rectangular range.
    /// Filled once per pane via `CanvasModel::get_cell_styles_in`, drained
    /// per-cell via `Option::take` in the bg pass. Capacity persists across
    /// frames; on the wasm path this is the single buffer the JS bridge
    /// drains a per-pane response into.
    pub(crate) pane_styles: Cell<Vec<Option<Style>>>,
    /// Dense, row-major formatted-cell-values for the current pane's range.
    /// Same shape and rhythm as `pane_styles`: filled once per pane via
    /// `CanvasModel::get_formatted_cell_values_in`, moved out per-cell via
    /// `Option::take` in the text pass.
    pub(crate) pane_values: Cell<Vec<Option<String>>>,
    /// Dense, row-major cell types for the current pane's range. Same shape
    /// and rhythm as `pane_styles` / `pane_values`: filled once per pane via
    /// `CanvasModel::get_cell_types_in`, copied per-cell into the text-pass
    /// `CellTextStyle::resolve` for alignment / error-color decisions.
    pub(crate) pane_cell_types: Cell<Vec<Option<CellType>>>,
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

/// Renderer-lifetime intern table for per-cell color strings (border + text
/// overrides). Keyed by the **raw** `&str` from the model so cache-hit lookups
/// stay zero-alloc (no normalization on the hit path); the value is the
/// `CssColor`-normalized output the painter actually consumes. Two raws that
/// normalize to the same color produce two entries — accepted, cardinality is
/// bounded by the small set of distinct colors a sheet uses.
pub(crate) struct ColorIntern {
    entries: RefCell<Vec<(Box<str>, Rc<str>)>>,
}

impl ColorIntern {
    pub(crate) fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
        }
    }

    /// Returns the interned normalized color for `raw`. Hit: `Rc::clone`.
    /// Miss: one `CssColor::new(raw).into_string()` + one `Box<str>` key +
    /// one `Rc<str>` value, then `Rc::clone` for the return.
    pub(crate) fn get(&self, raw: &str) -> Rc<str> {
        let mut entries = self.entries.borrow_mut();
        for (key, css) in entries.iter() {
            if &**key == raw {
                return Rc::clone(css);
            }
        }
        let css: Rc<str> = CssColor::new(raw).into_string().into();
        entries.push((raw.into(), Rc::clone(&css)));
        css
    }
}
