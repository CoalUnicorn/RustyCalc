//! Cross-frame per-pane model data, plus the blit-aware shift that lets
//! the cache survive a single-axis scroll.
//!
//! Each [`PaneBuffers`] holds the styles/values/cell_types last fetched
//! for its [`crate::chrome::PaneRegion`], together with the `RCRange` the
//! fetch covered. `render_pane` skips the model refetch when the cached
//! `range` still matches the live pane range. Under a blit fast-path the
//! caller calls [`PaneBuffers::try_shift`] first, rotating the buffers in
//! place so the kept band survives and only the revealed strip needs a
//! refetch.

use std::cell::Cell;

use crate::chrome::{PaneRegion, PaneRegionMask};
use crate::geometry::prim::Axis;
use crate::style::{CellKind, CellStyle};
use crate::types::coord::RCRange;

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
pub struct PaneBuffers {
    pub styles: Cell<Vec<Option<CellStyle>>>,
    pub values: Cell<Vec<Option<String>>>,
    pub cell_types: Cell<Vec<Option<CellKind>>>,
    /// The address-space range the buffers above were fetched for. `None`
    /// when this pane has never been painted, or was last seen empty
    /// (e.g. unfrozen-axis pane on a sheet without freezes).
    pub range: Cell<Option<RCRange>>,
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
    pub fn try_shift(&self, new_range: RCRange, axis: Axis) -> bool {
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
/// (sits alongside `FontIntern` / `ColorIntern`) — the
/// Stage 1 fingerprint-skip already proved we want cross-frame content
/// caching; Stage 3.1 graduates it from FrameCache scratch into a
/// first-class durable cache.
#[derive(Default)]
pub struct PaneCache {
    panes: [PaneBuffers; 4],
}

impl PaneCache {
    pub fn pane(&self, region: PaneRegion) -> &PaneBuffers {
        &self.panes[region as usize]
    }

    /// Drop the cached `range` for every pane named in `mask` so the next
    /// `render_pane` call refetches values/styles from the model instead
    /// of trusting the stale buffers. The buffer Vecs stay allocated —
    /// the refetch path overwrites them in place. Unmasked panes are
    /// untouched and keep fingerprint-skipping.
    pub fn invalidate(&self, mask: PaneRegionMask) {
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
