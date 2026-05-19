use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ironcalc_base::types::{CellType, Style};

use super::cell::text::TextLine;
use super::cell::CellPaint;
use crate::chrome::{PaneRegion, PaneRegionMask};
use crate::geometry::prim::Axis;
use crate::painter::CssColor;
use crate::types::coord::RCRange;

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

/// Per-pane buffers that survive across frames. Holds the most recent
/// bulk-fetch output for one `PaneRegion`, plus the `RCRange` they were
/// fetched for. `render_pane` reads `range` to decide whether the cached
/// buffers are still valid for the live frame: if `frame.kind.reuses_slots()`
/// and the live pane range equals the cached range, no fetch is needed.
///
/// Each field stays `Cell`-wrapped so `render_pane` can `take` for
/// mutation and `set` back at the end of the call (same rhythm the
/// FrameCache scratch buffers used pre-Stage-3).
#[derive(Default)]
pub(crate) struct PaneBuffers {
    pub(crate) styles: Cell<Vec<Option<Style>>>,
    pub(crate) values: Cell<Vec<Option<String>>>,
    pub(crate) cell_types: Cell<Vec<Option<CellType>>>,
    /// The address-space range the buffers above were fetched for. `None`
    /// when this pane has never been painted, or was last seen empty
    /// (e.g. unfrozen-axis pane on a sheet without freezes).
    pub(crate) range: Cell<Option<RCRange>>,
}

impl PaneBuffers {
    /// Rotate `styles` / `values` / `cell_types` in place from the cached
    /// `prev_range` into `new_range` along `axis`. Returns `true` on
    /// success; on `false` the cache has been cleared (`range` set to
    /// `None`) so `render_pane` falls through to a full fetch instead of
    /// reading shifted-but-mismatched buffers.
    ///
    /// `range` is intentionally left at `prev_range` on success —
    /// `render_pane` reads both `range` and the live pane range, infers
    /// the single-axis shift, and runs the strip-fetch branch. Bumping
    /// to `new_range` here would trip the range-equality early-exit and
    /// skip the strip paint entirely.
    pub(crate) fn try_shift(&self, new_range: RCRange, axis: Axis) -> bool {
        let Some(prev_range) = self.range.get() else {
            return false;
        };
        if !shift_is_safe(prev_range, new_range, axis) {
            self.range.set(None);
            return false;
        }
        let mut styles = self.styles.take();
        let mut values = self.values.take();
        let mut cell_types = self.cell_types.take();
        apply_blit_shift(&mut styles, prev_range, new_range, axis);
        apply_blit_shift(&mut values, prev_range, new_range, axis);
        apply_blit_shift(&mut cell_types, prev_range, new_range, axis);
        self.styles.set(styles);
        self.values.set(values);
        self.cell_types.set(cell_types);
        true
    }
}

/// Four pane buffers, indexed by `PaneRegion as usize`. Renderer-lifetime
/// (sits alongside `FontIntern` / `ColNameIntern` / `ColorIntern`) — the
/// Stage 1 fingerprint-skip already proved we want cross-frame content
/// caching; Stage 3.1 graduates it from FrameCache scratch into a
/// first-class durable cache.
#[derive(Default)]
pub(crate) struct PaneCache {
    panes: [PaneBuffers; 4],
}

impl PaneCache {
    pub(crate) fn pane(&self, region: PaneRegion) -> &PaneBuffers {
        &self.panes[region as usize]
    }

    /// Drop the cached `range` for every pane named in `mask` so the next
    /// `render_pane` call refetches values/styles from the model instead
    /// of trusting the stale buffers. The buffer Vecs stay allocated —
    /// the refetch path overwrites them in place. Unmasked panes are
    /// untouched and keep fingerprint-skipping.
    pub(crate) fn invalidate(&self, mask: PaneRegionMask) {
        for region in mask.regions() {
            self.panes[region as usize].range.set(None);
        }
    }
}

/// True when `prev_range` can be `apply_blit_shift`-rotated into
/// `new_range` along `axis` without corrupting the buffer: the orthogonal
/// axis must be identical on both ranges and the scroll-axis extent must
/// be preserved. Stale caches (e.g. from a frame before a canvas resize)
/// fail this check; callers drop them rather than feeding `apply_blit_shift`
/// mismatched dimensions.
fn shift_is_safe(prev: RCRange, new: RCRange, axis: Axis) -> bool {
    match axis {
        Axis::Row => {
            prev.c1 == new.c1 && prev.c2 == new.c2 && (new.r2 - new.r1) == (prev.r2 - prev.r1)
        }
        Axis::Column => {
            prev.r1 == new.r1 && prev.r2 == new.r2 && (new.c2 - new.c1) == (prev.c2 - prev.c1)
        }
    }
}

/// Shift a row-major pane buffer in place to match a new pane `RCRange`,
/// preserving entries whose `(row, col)` survived the scroll and leaving
/// freshly-revealed slots as `None` for the caller's strip-fetch to fill.
///
/// Invariants (caller-enforced; `screen_for_blit` already guarantees these):
/// - `prev_range` and `new_range` differ on exactly the `axis` given.
/// - The orthogonal axis has identical first/last indices on both ranges.
/// - `|delta|` along `axis` is strictly less than the visible extent on
///   that axis (otherwise overlap is empty and the caller falls back to
///   a full rebuild — never calls this helper).
/// - At entry, `buf.len() == prev_rows * prev_cols`.
///
/// On exit, `buf.len() == new_rows * new_cols`. Strip slots (the newly-
/// revealed band along `axis`) are `None`; kept-band slots carry the
/// values that were at those `(row, col)` pairs in `prev_range`.
///
/// Note: this operates on `Vec<Option<T>>` for arbitrary `T` — no `Copy`
/// bound. Use `slice::rotate_left` / `rotate_right` (which work for any
/// `T`), not `copy_within` (which is `T: Copy` only).
fn apply_blit_shift<T>(
    buf: &mut Vec<Option<T>>,
    prev_range: RCRange,
    new_range: RCRange,
    axis: Axis,
) {
    let prev_rows = (prev_range.r2 - prev_range.r1 + 1) as usize;
    let prev_cols = (prev_range.c2 - prev_range.c1 + 1) as usize;
    let new_rows = (new_range.r2 - new_range.r1 + 1) as usize;
    let new_cols = (new_range.c2 - new_range.c1 + 1) as usize;

    debug_assert_eq!(buf.len(), prev_rows * prev_cols);

    match axis {
        Axis::Row => {
            // Vertical scroll: row-major layout means the kept-band moves in
            // whole-row blocks of `cols` slots. Rotate the entire buffer by
            // `|delta_rows| * cols`; the displaced rows land in the strip,
            // which we then overwrite with None for strip-fetch to fill.
            debug_assert_eq!(prev_cols, new_cols);
            debug_assert_eq!(prev_rows, new_rows);
            let cols = prev_cols;
            let delta = new_range.r1 - prev_range.r1;
            if delta > 0 {
                let shift = delta as usize * cols;
                buf.rotate_left(shift);
                let strip_start = buf.len() - shift;
                buf[strip_start..].fill_with(|| None);
            } else if delta < 0 {
                let shift = (-delta) as usize * cols;
                buf.rotate_right(shift);
                buf[..shift].fill_with(|| None);
            }
        }
        Axis::Column => {
            // Horizontal scroll: row-major layout means each row's cells are
            // contiguous but adjacent-row cells are `cols` apart. Rotate one
            // row at a time so the kept-band lands at the correct column in
            // each row.
            debug_assert_eq!(prev_rows, new_rows);
            debug_assert_eq!(prev_cols, new_cols);
            let cols = prev_cols;
            let delta = new_range.c1 - prev_range.c1;
            if delta > 0 {
                let shift = delta as usize;
                for row in buf.chunks_exact_mut(cols) {
                    row.rotate_left(shift);
                    row[cols - shift..].fill_with(|| None);
                }
            } else if delta < 0 {
                let shift = (-delta) as usize;
                for row in buf.chunks_exact_mut(cols) {
                    row.rotate_right(shift);
                    row[..shift].fill_with(|| None);
                }
            }
        }
    }

    buf.resize_with(new_rows * new_cols, || None);
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
        let css: Rc<str> = build(size_px, bold, italic, family, fallback).into();
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

pub(crate) fn build(
    size_px: f64,
    bold: bool,
    italic: bool,
    family: &str,
    fallback: &str,
) -> String {
    let b = if bold { "bold " } else { "" };
    let i = if italic { "italic " } else { "" };
    let safe_family = escape_font_family(family, fallback);
    format!("{b}{i}{size_px}px {safe_family}")
}

pub(crate) fn escape_font_family(name: &str, fallback: &str) -> String {
    match name.trim() {
        "" => fallback.to_owned(),
        n if n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => n.to_owned(),
        n => format!("\"{}\"", n.replace('"', "")),
    }
}
