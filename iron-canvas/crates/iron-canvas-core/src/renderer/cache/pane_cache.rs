//! Cross-frame per-pane model data, plus the blit-aware shift that lets
//! the cache survive a single-axis scroll.
//!
//! Each [`PaneBuffers`] holds the styles/values/cell_types/decorations last fetched
//! for its [`crate::chrome::PaneRegion`], together with the `RCRange` the
//! fetch covered. `render_pane` skips the model refetch when the cached
//! `range` still matches the live pane range. Under a blit fast-path the
//! caller first classifies the shift via [`PaneBuffers::classify_shift`] (pure
//! — decides `Shifted`/`MissingCache`/`IncompatibleRange` without touching the
//! buffers), fetches the revealed strip, and only once that fetch is
//! confirmed clean calls [`PaneBuffers::apply_shift`] to rotate the buffers in
//! place so the kept band survives and the strip fetch splices into the
//! freshly revealed slots.

use std::cell::{Cell, RefCell};

use crate::chrome::{PaneRegion, PaneRegionMask};
use crate::geometry::prim::Axis;
use crate::renderer::cell::fingerprint::{
    PaneFingerprint, RepaintPlan, RowShiftFingerprint, RowShiftIneligible, plan_pane_repaint,
    rebuild_pane_fingerprint_in_place_from_cells, rotate_pane_fingerprint_in_place,
};
use crate::renderer::prepared::FetchedCells;
use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

/// One pane's painted-pixel fingerprint state, a sibling of
/// [`PaneBuffers`]'s model-buffer cache (`range`).
///
/// For the whole-pane comparison, a stale painted tree is
/// *self-disqualifying* — it needs no marker to be safe. The pane's
/// address-space `range` is folded into `PaneFingerprint.digest` itself
/// (`build_pane_fingerprint` hashes `range.r1/c1/r2/c2` before any row), and
/// `plan_pane_repaint` gates on `painted.range != scratch.range -> Full`
/// right after its digest compare. A splice-kind pane-cache commit that
/// cannot prove a complete post-strip tree (a Damage band, an emptied pane, a
/// column or ineligible row shift) therefore leaves `painted` holding the last
/// proven paint's range/digest and only marks it [`FingerprintTruth::Stale`];
/// the next frame's whole-pane compare against that tree naturally decides
/// Skip or repaint on its own merits. That conservative comparison is exactly
/// what heals a pane whose history no longer describes its pixels, and
/// [`Self::compare_to_painted`] therefore reads the tree regardless of
/// [`FingerprintTruth`].
///
/// [`Self::build_row_shift_candidate`] is the one operation that cannot rely
/// on that self-disqualification, which is why `truth` exists. It wants to
/// *carry history forward* rather than compare against it, and range equality
/// alone cannot prove history is carryable: a Damage strip changes pixels
/// while leaving the painted tree's range untouched. `truth` is the closed
/// answer to "does this tree still describe the pixels and committed buffers
/// it claims to" — `Stale` by default (an unpainted tree is never rotatable),
/// `Exact` only after an [`Self::install`] of a complete candidate, back to
/// `Stale` on any [`Self::mark_stale`] strip commit.
///
/// Ownership lives here (renderer-lifetime, on `RendererCore` via
/// `PaneCache`) rather than on `Chrome` (rebuilt every `Fresh`/`SlotsReuse`/
/// `Blitted` frame) — nothing has to be manually carried forward across a
/// frame rebuild, unlike the scalar arrays this replaces.
///
/// Two persistent slots, not one: comparing "what did we last paint" against
/// "what would we paint now" needs both trees alive at once, and the *next*
/// preparation needs a warm `Vec`-backed target to rebuild into without
/// reallocating. `painted` is the last-committed tree; `scratch` is a
/// non-semantic capacity pool only — never itself compared against
/// `painted` while both are live in persistent state (see
/// [`Self::build_candidate`]'s doc). On a successful [`Self::install`] the
/// now-stale `painted` tree becomes the next preparation's warm `scratch`
/// target; the tree that was just installed *is* the value a caller of
/// [`Self::build_candidate`] built and owned across the whole prepare step,
/// never parked here mid-attempt.
#[derive(Default)]
pub(crate) struct PaneFingerprintState {
    painted: RefCell<PaneFingerprint>,
    scratch: RefCell<PaneFingerprint>,
    truth: Cell<FingerprintTruth>,
}

/// Whether a pane's retained `painted` tree still describes the pixels and
/// committed buffers it claims to describe.
///
/// Closed and two-valued on purpose: the only question any caller may ask is
/// "may I treat this history as fact?", and `Stale` is the answer that costs
/// nothing but a repaint. A `bool` would invite a third reading ("unset");
/// an `Option<PaneFingerprint>` would conflate "no history" with "history I
/// can still usefully compare against", which is precisely the comparison
/// that heals a stale pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FingerprintTruth {
    /// The tree was built from, and installed alongside, the exact buffers
    /// that were painted.
    Exact,
    /// The tree may describe pre-strip pixels. Safe to compare against,
    /// never safe to carry forward.
    #[default]
    Stale,
}

impl PaneFingerprintState {
    /// Build a candidate tree from this frame's freshly bulk-fetched
    /// buffers as an OWNED value: rebuilds in place into `scratch` (reusing
    /// whatever warm `Vec` capacity is parked there via
    /// [`rebuild_pane_fingerprint_in_place_from_cells`]), then `mem::take`s it out,
    /// leaving `scratch` at `Default` again. The candidate belongs to the
    /// caller from this point on — a preparation that builds one and then
    /// abandons it (a held pane) never left `scratch` holding
    /// attempt-specific data for a later frame to misread; `scratch` was
    /// never anything but capacity to begin with, and it gets refilled with
    /// real capacity the moment this or a sibling pane's next
    /// [`Self::install`] runs.
    pub(crate) fn build_candidate(&self, cells: &FetchedCells, range: RCRange) -> PaneFingerprint {
        let mut scratch = self.scratch.borrow_mut();
        rebuild_pane_fingerprint_in_place_from_cells(&mut scratch, cells, range);
        std::mem::take(&mut *scratch)
    }

    /// Compare an already-built candidate against the last-committed
    /// `painted` tree — the whole-pane digest-equal Skip fast path lives in
    /// `plan_pane_repaint`'s first line. Read-only: does not touch
    /// `painted` or `scratch`.
    pub(crate) fn compare_to_painted(&self, candidate: &PaneFingerprint) -> RepaintPlan {
        plan_pane_repaint(&self.painted.borrow(), candidate)
    }

    /// Commit: install `candidate` as the pane's newly painted state,
    /// parking the now-stale evicted tree into `scratch` so its `Vec`
    /// capacity stays warm for the next [`Self::build_candidate`] call —
    /// zero allocation, zero clone. Must be called at most once per
    /// successful pane commit, and never on a held/failed preparation (see
    /// `RendererCore::install_pane_cache_commit`, the only caller).
    ///
    /// Installing a complete candidate alongside the cells it was built from
    /// is what makes history `Exact` — including a `Skip`/`Rows` verdict,
    /// whose candidate describes the pane just as completely as a `Full`
    /// one's does.
    pub(crate) fn install(&self, candidate: PaneFingerprint) {
        let old = std::mem::replace(&mut *self.painted.borrow_mut(), candidate);
        *self.scratch.borrow_mut() = old;
        self.truth.set(FingerprintTruth::Exact);
    }

    /// Commit: record that the painted tree may no longer describe the
    /// pane's pixels, without discarding it — the next whole-pane comparison
    /// still reads it, and that comparison is what schedules the healing
    /// repaint. For a strip commit that changed pixels the tree did not
    /// witness (a Damage band, an emptied pane), and for any shift this
    /// stage refuses to rotate.
    pub(crate) fn mark_stale(&self) {
        self.truth.set(FingerprintTruth::Stale);
    }

    /// Derive a complete candidate for a row-scrolled pane from history
    /// instead of a full-pane fetch: rows that survived the shift are carried
    /// across from the painted tree, rows the widened `strip_range` names are
    /// fingerprinted from `strip` — the values the blit already fetched and
    /// is about to paint, read here *before* the painter drains them.
    ///
    /// Read-only with respect to everything semantic. `painted` and `truth`
    /// are only read; the candidate is assembled in the non-semantic
    /// `scratch` slot (for its warm `Vec` capacity, exactly like
    /// [`Self::build_candidate`]) and then taken out as an owned value, so
    /// an attempt that is later held leaves nothing of itself behind. A
    /// rejected rotation writes nothing at all.
    ///
    /// `prev_range` is the caller's own view of what the pane cached, taken
    /// from the same `PaneShiftPrep::Shifted` that authorized the blit; a
    /// painted tree describing anything else is not the history of this
    /// shift and is refused rather than reinterpreted.
    pub(crate) fn build_row_shift_candidate(
        &self,
        prev_range: RCRange,
        new_range: RCRange,
        axis: Axis,
        strip: &FetchedCells,
        strip_range: RCRange,
    ) -> RowShiftFingerprint {
        // Column-axis rotation is out of scope by design, not by omission: a
        // horizontal shift changes which columns each row spans, so no row
        // digest survives it.
        if let Axis::Column = axis {
            return RowShiftFingerprint::Ineligible(RowShiftIneligible::ColumnAxis);
        }
        if let FingerprintTruth::Stale = self.truth.get() {
            return RowShiftFingerprint::Ineligible(RowShiftIneligible::StaleHistory);
        }

        let painted = self.painted.borrow();
        if painted.range != prev_range {
            return RowShiftFingerprint::Ineligible(RowShiftIneligible::PriorRangeMismatch);
        }

        let mut scratch = self.scratch.borrow_mut();
        match rotate_pane_fingerprint_in_place(
            &mut scratch,
            &painted,
            new_range,
            strip,
            strip_range,
        ) {
            Ok(()) => RowShiftFingerprint::Rotated(std::mem::take(&mut scratch)),
            Err(reason) => RowShiftFingerprint::Ineligible(reason),
        }
    }

    /// Abort-only: return an uncommitted candidate to the warm scratch slot.
    /// The candidate was never installed, so this changes no painted state.
    pub(crate) fn recycle_candidate(&self, candidate: PaneFingerprint) {
        *self.scratch.borrow_mut() = candidate;
    }

    /// Test-only: the warm scratch tree's retained row capacity. Rows are the
    /// finest unit the tree stores, so one number describes all of it.
    #[cfg(feature = "surface-introspection")]
    fn scratch_row_capacity(&self) -> usize {
        self.scratch.borrow().rows.capacity()
    }

    /// Test-only, pre-Stage-6 shape preserved for API compatibility: `(row
    /// capacity, cell capacity)`. The cell leaf no longer exists (Stage 6
    /// collapsed `pane -> row -> cell` to `pane -> row`), so the second slot
    /// is always `0`; callers that want just the row number should use
    /// [`Self::scratch_row_capacity`] via [`PaneBuffers::fingerprint_row_scratch_capacity`].
    #[cfg(feature = "surface-introspection")]
    fn scratch_capacities(&self) -> (usize, usize) {
        (self.scratch_row_capacity(), 0)
    }
}

/// Per-pane buffers that survive across frames. Holds the most recent
/// bulk-fetch output for one `PaneRegion`, plus the `RCRange` they were
/// fetched for. Full-pane preparation always refetches targeted panes; the
/// range tells Damage and Viewport whether those committed buffers can be
/// spliced or shifted safely.
///
/// Each field stays `Cell`-wrapped so `render_pane` can `take` for
/// mutation and `set` back at the end of the call (same rhythm the
/// FrameCache scratch buffers used pre-Stage-3).
#[derive(Default)]
pub struct PaneBuffers {
    pub styles: Cell<Vec<Fetched<CellStyle>>>,
    pub values: Cell<Vec<Fetched<String>>>,
    pub cell_types: Cell<Vec<Fetched<CellKind>>>,
    pub decorations: Cell<Vec<Fetched<CellDecoration>>>,
    /// The address-space range the buffers above were fetched for. `None`
    /// when this pane has never been painted, or was last seen empty
    /// (e.g. unfrozen-axis pane on a sheet without freezes).
    pub range: Cell<Option<RCRange>>,
    /// The pane's last-committed painted-pixel fingerprint tree, plus whether
    /// that tree may be carried forward. See [`PaneFingerprintState`]'s doc
    /// for why comparison needs no validity marker (a stale tree
    /// self-disqualifies via its baked-in range) while
    /// [`PaneFingerprintState::build_row_shift_candidate`] does, and gets one
    /// in [`FingerprintTruth`].
    pub(crate) fingerprint: PaneFingerprintState,
    /// Spare [`FetchedCells`] capacity for the next full-pane preparation
    /// attempt (`RendererCore::prepare_full_pane`). Preparation takes this
    /// — never the four committed fields above — as its fetch target, so a
    /// failed preparation has nothing of the committed cache to undo: it
    /// parks its failed fetch back here untouched; a successful commit
    /// parks the evicted old committed cells here instead (see
    /// [`Self::install_cells`]). Either way `styles`/`values`/`cell_types`/
    /// `decorations`/`range` above change only inside
    /// `RendererCore::commit_pane_cache`.
    prepare_scratch: Cell<FetchedCells>,
}

impl PaneBuffers {
    /// Pre-Stage-6 shape preserved for API compatibility. The second tuple's
    /// cell-leaf slot is always `0` now that `CellFingerprint` is gone; use
    /// [`Self::fingerprint_row_scratch_capacity`] for the row number alone.
    #[cfg(feature = "surface-introspection")]
    pub fn preparation_scratch_capacities(&self) -> ((usize, usize, usize, usize), (usize, usize)) {
        let cells = self.prepare_scratch.take();
        let cell_capacities = cells.capacities();
        self.prepare_scratch.set(cells);
        (cell_capacities, self.fingerprint.scratch_capacities())
    }

    /// Test-only: the warm fingerprint scratch tree's retained row capacity,
    /// added alongside [`Self::preparation_scratch_capacities`] (Stage 6)
    /// since that method's fixed shape has no room for a single clean number.
    #[cfg(feature = "surface-introspection")]
    pub fn fingerprint_row_scratch_capacity(&self) -> usize {
        self.fingerprint.scratch_row_capacity()
    }

    /// Pure: hand back whatever spare [`FetchedCells`] capacity is parked
    /// for reuse, WITHOUT touching the four committed content fields.
    pub(crate) fn take_prepare_scratch(&self) -> FetchedCells {
        self.prepare_scratch.take()
    }

    /// Park capacity for the next preparation attempt. Called both on a
    /// failed preparation (the just-fetched, now-discarded cells) and on a
    /// successful commit (the evicted old committed cells) — either way the
    /// four committed content fields are untouched by this call.
    pub(crate) fn park_prepare_scratch(&self, cells: FetchedCells) {
        self.prepare_scratch.set(cells);
    }

    /// Execution-only: hand the four committed content fields out as one
    /// [`FetchedCells`] bundle, leaving this pane's slots empty until the
    /// caller hands the bundle back via [`Self::set_cells`] at the
    /// completion boundary. Damage and Blit execution splice and paint
    /// through the returned bundle, so they must call this only *after*
    /// their preparation gate already proved every required fetch clean —
    /// preparation itself takes [`Self::take_prepare_scratch`] instead,
    /// which is exactly why a failed preparation has no committed state to
    /// undo.
    pub(crate) fn take_cells(&self) -> FetchedCells {
        FetchedCells::from_parts(
            self.styles.take(),
            self.values.take(),
            self.cell_types.take(),
            self.decorations.take(),
        )
    }

    /// Commit-only: swap `cells` in as the new committed content, returning
    /// the evicted old committed content so the caller can park it as the
    /// next attempt's scratch (see [`Self::park_prepare_scratch`]).
    pub(crate) fn install_cells(&self, cells: FetchedCells) -> FetchedCells {
        let (styles, values, cell_types, decorations) = cells.into_parts();
        FetchedCells::from_parts(
            self.styles.replace(styles),
            self.values.replace(values),
            self.cell_types.replace(cell_types),
            self.decorations.replace(decorations),
        )
    }

    /// Commit-only: set the committed content directly, no swap, no old
    /// value returned. Used by a Damage/Splice commit, whose buffers
    /// already ARE this pane's own committed buffers (taken once, spliced
    /// in place, and handed back) — there is no foreign "old" value to
    /// evict.
    pub(crate) fn set_cells(&self, cells: FetchedCells) {
        let (styles, values, cell_types, decorations) = cells.into_parts();
        self.styles.set(styles);
        self.values.set(values);
        self.cell_types.set(cell_types);
        self.decorations.set(decorations);
    }
}

/// Typed outcome of [`PaneBuffers::classify_shift`]. Replaces the old
/// `bool` so the dispatch site can decide strip-paint vs full-fetch once,
/// from a named reason, instead of dropping the bool and re-deriving the
/// decision downstream.
///
/// `Shifted` carries the ranges the dispatch site needs to build the pane's
/// `BlitPaneWork`. `MissingCache` is the never-cached case; `IncompatibleRange`
/// is the stale-cache case (e.g. a frame before a canvas resize) — both route
/// the pane through a full `render_pane` repaint.
#[derive(Debug, PartialEq)]
pub enum PaneShiftPrep {
    Shifted {
        prev_range: RCRange,
        new_range: RCRange,
    },
    MissingCache,
    IncompatibleRange {
        prev_range: RCRange,
        new_range: RCRange,
    },
}

impl PaneBuffers {
    /// Pure classification: reports which [`PaneShiftPrep`] variant this
    /// pane/range/axis is, WITHOUT rotating the buffers or clearing the
    /// cached `range` either way. Stage 4's blit preparation
    /// (`RendererCore::prepare_blit`) uses this to decide, per pane, whether
    /// to fetch a revealed strip or fall back to a full-pane fetch — *before*
    /// any pixel is shifted or any cache mutated, so a fetch that fails
    /// leaves the pane untouched. The actual rotation is deferred to
    /// [`Self::apply_shift`], called only once that fetch is confirmed clean.
    pub fn classify_shift(&self, new_range: RCRange, axis: Axis) -> PaneShiftPrep {
        let Some(prev_range) = self.range.get() else {
            return PaneShiftPrep::MissingCache;
        };
        if !shift_is_safe(prev_range, new_range, axis) {
            return PaneShiftPrep::IncompatibleRange {
                prev_range,
                new_range,
            };
        }
        PaneShiftPrep::Shifted {
            prev_range,
            new_range,
        }
    }

    /// Execution-only: rotate `styles` / `values` / `cell_types` /
    /// `decorations` in place from `prev_range` into `new_range` along
    /// `axis`, so the kept band survives and the revealed strip carries
    /// placeholders (`Fetched::Absent`) for the caller's already-fetched
    /// strip to splice into. Never call this during preparation — only after
    /// [`Self::classify_shift`] returned `Shifted` for these exact ranges AND
    /// the revealed strip's fetch is already confirmed clean (see
    /// `renderer::prepared`'s module doc for why preparation must never
    /// mutate committed buffers). Does not touch `range` — committing it to
    /// `new_range` is the caller's separate, later step, alongside installing
    /// the spliced buffers (see `RendererCore::commit_pane_cache`).
    pub fn apply_shift(&self, prev_range: RCRange, new_range: RCRange, axis: Axis) {
        let mut styles = self.styles.take();
        let mut values = self.values.take();
        let mut cell_types = self.cell_types.take();
        let mut decorations = self.decorations.take();
        apply_blit_shift(&mut styles, prev_range, new_range, axis, Fetched::Absent);
        apply_blit_shift(&mut values, prev_range, new_range, axis, Fetched::Absent);
        apply_blit_shift(
            &mut cell_types,
            prev_range,
            new_range,
            axis,
            Fetched::Absent,
        );
        apply_blit_shift(
            &mut decorations,
            prev_range,
            new_range,
            axis,
            Fetched::Absent,
        );
        self.styles.set(styles);
        self.values.set(values);
        self.cell_types.set(cell_types);
        self.decorations.set(decorations);
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

/// Address-space blit work for one shifted pane, computed before painting.
/// Carries the cached `prev_range`, the live `new_range`, the scroll `axis`
/// (taken from `BlitPlan`, never re-inferred), and the base revealed
/// `strip_range`. A renderer-local helper widens `strip_range` to the pixel
/// clip; this type is the cache's half of the split — no `Chrome` dependency.
#[derive(Clone, Copy)]
pub struct PaneBlitAddressWork {
    pub axis: Axis,
    pub prev_range: RCRange,
    pub new_range: RCRange,
    pub strip_range: RCRange,
}

impl PaneCache {
    pub fn pane(&self, region: PaneRegion) -> &PaneBuffers {
        &self.panes[region as usize]
    }

    /// Build address-space blit work from a `Shifted` [`PaneShiftPrep`]: the
    /// classification already proved compatibility (its `Shifted` vs
    /// `IncompatibleRange` split *is* the [`shift_is_safe`] predicate,
    /// single-sourced), so this only computes the base revealed strip. `axis`
    /// flows from `BlitPlan`, never re-inferred. Returns `None` only on the
    /// defensive zero-delta case `compute_strip` rejects.
    pub fn plan_blit_pane(
        &self,
        prev_range: RCRange,
        new_range: RCRange,
        axis: Axis,
    ) -> Option<PaneBlitAddressWork> {
        let strip_range = compute_strip(prev_range, new_range, axis)?;
        Some(PaneBlitAddressWork {
            axis,
            prev_range,
            new_range,
            strip_range,
        })
    }

    /// Drop the cached `range` for every pane named in `mask` so the next
    /// `render_pane` call refetches values/styles from the model instead
    /// of trusting the stale buffers. The buffer Vecs stay allocated —
    /// the refetch path overwrites them in place. Unmasked panes are
    /// untouched and keep fingerprint-skipping.
    ///
    /// Buffer-range invalidation only — never the painted-pixel tree. A
    /// masked pane whose refetch comes back byte-identical to what's already
    /// on screen (e.g. a content-dirty signal with no actual edit behind it)
    /// still finds its prior `painted` tree intact and skips the repaint for
    /// free; dropping the tree here would turn every SlotsReuse content signal
    /// into an unconditional whole-pane repaint, defeating the fingerprint
    /// skip this cache exists to provide.
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
///
/// Single source of the compatibility predicate: called only from
/// [`PaneBuffers::classify_shift`], whose `Shifted` vs `IncompatibleRange`
/// split *is* this invariant. `plan_blit_pane` reads `classify_shift`'s
/// `Shifted` result rather than re-testing.
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
/// freshly-revealed slots as `fill` (the caller's placeholder) for the
/// strip-fetch to overwrite.
///
/// Invariants (caller-enforced; `Chrome::classify` already guarantees these):
/// - `prev_range` and `new_range` differ on exactly the `axis` given.
/// - The orthogonal axis has identical first/last indices on both ranges.
/// - `|delta|` along `axis` is strictly less than the visible extent on
///   that axis (otherwise overlap is empty and the caller falls back to
///   a full rebuild — never calls this helper).
/// - At entry, `buf.len() == prev_rows * prev_cols`.
///
/// On exit, `buf.len() == new_rows * new_cols`. Strip slots (the newly-
/// revealed band along `axis`) carry `fill` (the caller's placeholder for an
/// un-fetched slot — `Fetched::Absent` for content, `None` for decorations);
/// kept-band slots carry the values that were at those `(row, col)` pairs in
/// `prev_range`.
///
/// Note: this operates on `Vec<E>` for arbitrary `E: Clone` — no `Copy`
/// bound. Use `slice::rotate_left` / `rotate_right` (which work for any
/// `E`), not `copy_within` (which is `E: Copy` only).
fn apply_blit_shift<E: Clone>(
    buf: &mut Vec<E>,
    prev_range: RCRange,
    new_range: RCRange,
    axis: Axis,
    fill: E,
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
            // which we then overwrite with `fill` for strip-fetch to replace.
            debug_assert_eq!(prev_cols, new_cols);
            debug_assert_eq!(prev_rows, new_rows);
            let cols = prev_cols;
            let delta = new_range.r1 - prev_range.r1;
            if delta > 0 {
                let shift = delta as usize * cols;
                buf.rotate_left(shift);
                let strip_start = buf.len() - shift;
                buf[strip_start..].fill(fill.clone());
            } else if delta < 0 {
                let shift = (-delta) as usize * cols;
                buf.rotate_right(shift);
                buf[..shift].fill(fill.clone());
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
                    row[cols - shift..].fill(fill.clone());
                }
            } else if delta < 0 {
                let shift = (-delta) as usize;
                for row in buf.chunks_exact_mut(cols) {
                    row.rotate_right(shift);
                    row[..shift].fill(fill.clone());
                }
            }
        }
    }

    buf.resize(new_rows * new_cols, fill);
}

/// Slice of `new` lying outside `prev` along the scroll axis. Returns
/// `None` if the ranges are identical along `axis` (delta == 0), or if a
/// direct caller bypassed [`PaneBuffers::classify_shift`] and handed us
/// non-overlapping ranges. Valid blit callers prove overlap before this point.
fn compute_strip(prev: RCRange, new: RCRange, axis: Axis) -> Option<RCRange> {
    match axis {
        Axis::Row => {
            if new.r2 < prev.r1 || new.r1 > prev.r2 {
                debug_assert!(
                    ranges_overlap(prev.r1, prev.r2, new.r1, new.r2),
                    "compute_strip requires overlapping ranges from classify_shift"
                );
                return None;
            }
            if new.r1 < prev.r1 {
                Some(RCRange {
                    r1: new.r1,
                    r2: prev.r1 - 1,
                    c1: new.c1,
                    c2: new.c2,
                })
            } else if new.r2 > prev.r2 {
                // Includes `prev.r2` (not `prev.r2 + 1`) because that row
                // was the overflow row in prev — its pixels were off-canvas
                // and weren't shifted by the blit, so its on-canvas position
                // in new needs a fresh paint.
                Some(RCRange {
                    r1: prev.r2,
                    r2: new.r2,
                    c1: new.c1,
                    c2: new.c2,
                })
            } else {
                None
            }
        }
        Axis::Column => {
            if new.c2 < prev.c1 || new.c1 > prev.c2 {
                debug_assert!(
                    ranges_overlap(prev.c1, prev.c2, new.c1, new.c2),
                    "compute_strip requires overlapping ranges from classify_shift"
                );
                return None;
            }
            if new.c1 < prev.c1 {
                Some(RCRange {
                    r1: new.r1,
                    r2: new.r2,
                    c1: new.c1,
                    c2: prev.c1 - 1,
                })
            } else if new.c2 > prev.c2 {
                // Mirror of the Row down-scroll case: prev.c2 was the
                // overflow column whose pixels were off-canvas.
                Some(RCRange {
                    r1: new.r1,
                    r2: new.r2,
                    c1: prev.c2,
                    c2: new.c2,
                })
            } else {
                None
            }
        }
    }
}

fn ranges_overlap(prev_start: i32, prev_end: i32, new_start: i32, new_end: i32) -> bool {
    new_end >= prev_start && new_start <= prev_end
}

#[cfg(test)]
mod tests {
    use super::*;

    // `take_cells` is the execution-side mirror of `set_cells`: Damage and
    // Blit hand the committed channels out as one bundle, splice and paint
    // through it, and hand the same bundle back at the completion boundary.
    // Both halves have to move all four channels — a channel left behind in
    // the pane would be spliced at one range and read at another.
    #[test]
    fn take_cells_round_trips_every_channel_through_set_cells() {
        let buffers = PaneBuffers::default();
        let committed = FetchedCells::from_parts(
            vec![Fetched::Value(CellStyle::default()), Fetched::Absent],
            vec![
                Fetched::Value("a".to_string()),
                Fetched::Value("b".to_string()),
            ],
            vec![Fetched::Value(CellKind::Number), Fetched::Absent],
            vec![
                Fetched::Value(CellDecoration::Icon("star".to_string())),
                Fetched::Absent,
            ],
        );
        buffers.set_cells(committed.clone());

        let taken = buffers.take_cells();
        assert_eq!(taken.styles(), committed.styles());
        assert_eq!(taken.values(), committed.values());
        assert_eq!(taken.cell_types(), committed.cell_types());
        assert_eq!(taken.decorations(), committed.decorations());

        let drained = buffers.take_cells();
        assert!(
            drained.styles().is_empty()
                && drained.values().is_empty()
                && drained.cell_types().is_empty()
                && drained.decorations().is_empty(),
            "take_cells must leave every committed channel empty, not just some"
        );

        buffers.set_cells(taken);
        let reinstalled = buffers.take_cells();
        assert_eq!(reinstalled.styles(), committed.styles());
        assert_eq!(reinstalled.values(), committed.values());
        assert_eq!(reinstalled.cell_types(), committed.cell_types());
        assert_eq!(reinstalled.decorations(), committed.decorations());
    }

    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "compute_strip requires overlapping ranges")
    )]
    #[test]
    fn compute_strip_rejects_non_overlapping_row_ranges() {
        let prev = RCRange {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 4,
        };
        let new = RCRange {
            r1: 10,
            c1: 1,
            r2: 12,
            c2: 4,
        };

        #[cfg(debug_assertions)]
        let _ = compute_strip(prev, new, Axis::Row);
        #[cfg(not(debug_assertions))]
        assert!(
            compute_strip(prev, new, Axis::Row).is_none(),
            "non-overlapping row ranges are invalid blit work, not a strip"
        );
    }

    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "compute_strip requires overlapping ranges")
    )]
    #[test]
    fn compute_strip_rejects_non_overlapping_column_ranges() {
        let prev = RCRange {
            r1: 1,
            c1: 1,
            r2: 4,
            c2: 3,
        };
        let new = RCRange {
            r1: 1,
            c1: 10,
            r2: 4,
            c2: 12,
        };

        #[cfg(debug_assertions)]
        let _ = compute_strip(prev, new, Axis::Column);
        #[cfg(not(debug_assertions))]
        assert!(
            compute_strip(prev, new, Axis::Column).is_none(),
            "non-overlapping column ranges are invalid blit work, not a strip"
        );
    }
}
