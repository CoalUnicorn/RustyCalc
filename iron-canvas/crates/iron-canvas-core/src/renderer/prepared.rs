//! Prepared work: the transaction boundary between fallible bulk-bridge
//! reads and infallible painting.
//!
//! # Why this module exists
//!
//! Before Stage 4, a pane's paint attempt fetched from the model and
//! mutated `PaneBuffers`/`PaneFingerprintState` interleaved, one field at a
//! time, hoping a mid-attempt bridge failure was caught before the next
//! `Cell::set`. `FetchedCells` bundles the four bulk-fetch channels
//! (styles, values, cell types, decorations) so a pane's fetch is one
//! value, not four parallel `Vec`s threaded through every call. `Prepared*`
//! types carry that bundle plus the paint decision it implies, produced by
//! a pure "prepare" step that reads the model and the pane's *committed*
//! `painted` fingerprint tree but never writes either `PaneBuffers`'
//! `styles`/`values`/`cell_types`/`decorations`/`range` fields or
//! `PaneFingerprintState`'s `painted` tree. After every required bridge read
//! is clean, Damage/Blit execution may take committed cell Vecs into an owned
//! result, but only [`RendererCore::commit_pane_cache`] installs the returned
//! content/range/fingerprint commit.
//!
//! # Fetch order
//!
//! [`FetchedCells::fetch_into`] fetches styles, then values, then cell
//! types, then decorations — unchanged from every bulk-fetch call site this
//! bundle replaces.
//!
//! # What "pure" means here
//!
//! A `prepare_*` method may freely mutate *renderer-lifetime scratch*
//! (`PaneBuffers::prepare_scratch`, `FrameCache::strip_scratch`) and the
//! per-frame `FrameTrace` counters — neither is committed, cross-frame
//! *content* state. It must never write `PaneBuffers`' four content fields,
//! `PaneBuffers::range`, or `PaneFingerprintState::painted`. A failed
//! preparation (a `None` return) is therefore always a safe no-op against
//! everything a later frame's paint-skip or blit-shift decision reads.
//!
//! # Blit preparation (Task 4)
//!
//! [`PaneCacheAction::Shift`] and [`PreparedPane::{Empty,Blit}`](PreparedPane)
//! extend the two enums Task 2 deliberately left narrow (see that task's
//! note, preserved in git history) to cover the blit fast-path:
//! [`RendererCore::prepare_blit`] classifies each `plan.shift_panes()` pane
//! exactly once (via [`super::blit_work::shifted_pane_work`], itself built on
//! [`super::cache::PaneBuffers::classify_shift`]) and either stages a strip
//! fetch (`Blit`), falls back to the SAME [`Self::prepare_full_pane`] every
//! other full-pane caller uses (`Full`), or records the pane has no live
//! range at all (`Empty`) — one fetch per pane, never a safety fetch followed
//! by a second `render_pane`-style refetch. [`RendererCore::execute_blit`]
//! then rotates, splices, and paints each `Blit` pane's buffers — the
//! rotation itself (`PaneBuffers::apply_shift`) only ever runs here, after
//! every pane's strip fetch is already confirmed clean, and the final range
//! metadata is not installed until the completion boundary.
//!
//! # Strip fingerprint truth
//!
//! A strip commit also carries a `PreparedFingerprintUpdate`: an eligible
//! row blit derives a complete post-shift tree during preparation (from
//! history it proved `Exact`, plus the strip it is about to paint) and commits
//! `Install`; Damage, column blits, ineligible row shifts, and emptied panes
//! commit `MarkStale`. Like every other prepared value, the update is only
//! *built* during preparation — an attempt that never reaches
//! [`RendererCore::install_pane_cache_commit`] recycles its candidate as plain
//! capacity and leaves the retained tree and its truth exactly as it found
//! them.

use crate::CellContentQuery;
use crate::chrome::{BlitPlan, Chrome, PaneRegion};
use crate::geometry::prim::Axis;
use crate::orchestrator::PaneVerdict;
use crate::painter::{PaintColor, Painter};
use crate::pending_work::RowSpan;
use crate::renderer::RendererCore;
use crate::renderer::blit_work::{self, BlitPaneWork};
use crate::renderer::cell::PaneCells;
use crate::renderer::cell::fingerprint::{PaneFingerprint, RepaintPlan, RowShiftFingerprint};
use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

// ==============================================================================
// FetchedCells — the four bulk-fetch channels as one value
// ==============================================================================

/// The four bulk-fetch channels for one address-space range, bundled so
/// every fetch/take/park call site threads one value instead of four
/// parallel `Vec`s. Owns its own allocation reuse: [`Self::fetch_into`]
/// takes a `reuse` bundle and fetches into its already-allocated `Vec`s
/// (the `*_in` accessors `clear()` before filling), so a caller that keeps
/// recycling the same bundle across frames never re-allocates once the
/// pane's dimensions stabilize.
#[derive(Default, Clone)]
pub(crate) struct FetchedCells {
    styles: Vec<Fetched<CellStyle>>,
    values: Vec<Fetched<String>>,
    cell_types: Vec<Fetched<CellKind>>,
    decorations: Vec<Fetched<CellDecoration>>,
}

impl FetchedCells {
    #[cfg(any(test, feature = "surface-introspection"))]
    pub(super) fn capacities(&self) -> (usize, usize, usize, usize) {
        (
            self.styles.capacity(),
            self.values.capacity(),
            self.cell_types.capacity(),
            self.decorations.capacity(),
        )
    }

    pub(crate) fn from_parts(
        styles: Vec<Fetched<CellStyle>>,
        values: Vec<Fetched<String>>,
        cell_types: Vec<Fetched<CellKind>>,
        decorations: Vec<Fetched<CellDecoration>>,
    ) -> Self {
        Self {
            styles,
            values,
            cell_types,
            decorations,
        }
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn into_parts(
        self,
    ) -> (
        Vec<Fetched<CellStyle>>,
        Vec<Fetched<String>>,
        Vec<Fetched<CellKind>>,
        Vec<Fetched<CellDecoration>>,
    ) {
        (self.styles, self.values, self.cell_types, self.decorations)
    }

    pub(crate) fn styles(&self) -> &[Fetched<CellStyle>] {
        &self.styles
    }

    pub(crate) fn values(&self) -> &[Fetched<String>] {
        &self.values
    }

    pub(crate) fn cell_types(&self) -> &[Fetched<CellKind>] {
        &self.cell_types
    }

    pub(crate) fn decorations(&self) -> &[Fetched<CellDecoration>] {
        &self.decorations
    }

    /// Bulk-fetch `range` on `sheet` from `model` into `reuse`'s
    /// already-allocated `Vec`s. Fetch order is fixed: styles, values,
    /// cell types, decorations — every downstream consumer (the
    /// fingerprint digest, the five-pass paint walk) assumes this order
    /// never changes.
    pub(crate) fn fetch_into(
        model: &dyn CellContentQuery,
        sheet: u32,
        range: RCRange,
        reuse: Self,
    ) -> Self {
        let Self {
            mut styles,
            mut values,
            mut cell_types,
            mut decorations,
        } = reuse;
        model.get_cell_styles_in(sheet, range, &mut styles);
        model.get_formatted_cell_values_in(sheet, range, &mut values);
        model.get_cell_types_in(sheet, range, &mut cell_types);
        model.get_cell_decorations_in(sheet, range, &mut decorations);
        Self {
            styles,
            values,
            cell_types,
            decorations,
        }
    }

    /// Borrow all four channels at once for
    /// [`RendererCore::paint_cells_pass`]. Their equal length is
    /// established by construction — [`Self::fetch_into`] fills all four
    /// from one range, [`Self::splice_strip_from`] splices all four — so it
    /// is asserted once here, at the borrow boundary, rather than per cell
    /// inside the paint walk.
    pub(super) fn as_mut(&mut self) -> FetchedCellsMut<'_> {
        debug_assert!(
            self.styles.len() == self.values.len()
                && self.styles.len() == self.cell_types.len()
                && self.styles.len() == self.decorations.len(),
            "the four channels address one row-major range and must stay equal-length"
        );
        FetchedCellsMut {
            styles: &mut self.styles,
            values: &mut self.values,
            cell_types: &mut self.cell_types,
            decorations: &mut self.decorations,
        }
    }

    /// Splice `strip` into this bundle's matching slots, one channel at a
    /// time in the fixed styles -> values -> cell types -> decorations
    /// order. The row/column index arithmetic is the single generic
    /// [`super::cell::splice_strip_into`], applied four times rather than
    /// restated. `strip` is drained in place (each of its slots receives
    /// the evicted pane value) and stays a valid, capacity-bearing bundle,
    /// so its owner can park it straight back into scratch.
    pub(super) fn splice_strip_from(
        &mut self,
        strip: &mut Self,
        pane_range: RCRange,
        strip_range: RCRange,
    ) {
        super::cell::splice_strip_into(
            &mut self.styles,
            &mut strip.styles,
            pane_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.values,
            &mut strip.values,
            pane_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.cell_types,
            &mut strip.cell_types,
            pane_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.decorations,
            &mut strip.decorations,
            pane_range,
            strip_range,
        );
    }

    /// True when any of the four channels reports a transient bridge
    /// failure. Computed unconditionally by every caller of
    /// [`Self::fetch_into`] — whether that fact goes on to HOLD a pane is a
    /// separate, per-caller policy decision (see `render_pane`'s own doc
    /// for why a `Fresh`-kind frame currently checks this but does not yet
    /// act on it).
    pub(crate) fn has_bridge_failure(&self) -> bool {
        super::cell::has_bridge_failure(&self.styles)
            || super::cell::has_bridge_failure(&self.values)
            || super::cell::has_bridge_failure(&self.cell_types)
            || super::cell::has_bridge_failure(&self.decorations)
    }
}

/// [`FetchedCells`] borrowed, not consumed: the one argument the five-pass
/// paint walk takes instead of four parallel mutable slices. Named as a
/// bundle because all four channels describe the SAME row-major address
/// space — the walk computes one index and reads every channel with it, so
/// splitting them into separate parameters only creates opportunities to
/// pair a pane's cells with the wrong sibling channel. Renderer-private and
/// borrow-only: it never allocates, clones, or moves the owned `Vec`s.
pub(super) struct FetchedCellsMut<'a> {
    pub(super) styles: &'a mut [Fetched<CellStyle>],
    pub(super) values: &'a mut [Fetched<String>],
    pub(super) cell_types: &'a mut [Fetched<CellKind>],
    pub(super) decorations: &'a mut [Fetched<CellDecoration>],
}

// ==============================================================================
// Prepared cache actions and commits
// ==============================================================================

/// What a successful preparation will need to do to the persistent pane
/// cache once its data is confirmed clean — decided at prepare time, as
/// data, rather than performed eagerly before the fetch that might fail.
/// Carried on [`PreparedPane`]; turned into a data-bearing [`PaneCacheCommit`]
/// by execution, and installed by [`RendererCore::commit_pane_cache`].
#[derive(Clone)]
pub(crate) enum PaneCacheAction {
    /// This pane has no live address-space range this frame — forget its
    /// cached range so a future re-grow refetches (the blit empty-pane path).
    Empty,
    /// Install a freshly fetched whole-pane range (the full-pane path).
    Replace { range: RCRange },
    /// Rotate the cached buffers from `prev_range` to `new_range` along
    /// `axis` and splice the already-fetched revealed strip into them (the
    /// blit shifted-pane path). The rotation itself is deferred to
    /// execution — see [`super::cache::PaneBuffers::apply_shift`] — never
    /// performed during preparation.
    Shift {
        prev_range: RCRange,
        new_range: RCRange,
        axis: Axis,
    },
    /// Splice one or more already-cached-range row-band strips into the
    /// existing buffers (the Damage path). `strips` names every spliced
    /// sub-range.
    Splice {
        range: RCRange,
        strips: Vec<RCRange>,
    },
}

/// What a strip commit does to the pane's retained fingerprint tree.
///
/// A strip — a Damage band, or a blit's revealed band — repaints part of a
/// pane without rebuilding the whole-pane tree, so that tree's relationship to
/// the pixels afterwards is a *decision preparation makes*, never something a
/// later reader may infer: either preparation proved a complete tree for the
/// post-strip pane, or it did not. `Option<PaneFingerprint>` would spell those
/// two answers `Some`/`None`, and `None` reads equally well as "nothing
/// decided yet" — the one reading a commit must never carry.
pub(crate) enum PreparedFingerprintUpdate {
    /// Preparation derived a complete tree for the range this commit installs,
    /// from history it proved `FingerprintTruth::Exact` plus the strip the
    /// same commit is about to paint.
    Install(PaneFingerprint),
    /// The commit changes pixels the retained tree did not witness. The tree
    /// stays readable — the next whole-pane comparison against it is what
    /// schedules the healing repaint — but stops being carryable.
    MarkStale,
}

/// One pane's confirmed-safe installation, produced by execution and
/// consumed by [`RendererCore::commit_pane_cache`] — the only place a
/// `PaneBuffers`/`PaneFingerprintState`'s persistent, cross-frame content
/// changes. Each variant carries exactly the data its action needs; there
/// is no `Option` field standing in for "always `Some` by construction".
pub(crate) enum PaneCacheCommit {
    /// The pane has no live address-space range this frame — forget its
    /// cached range so a future re-grow refetches. An explicit, always-safe
    /// action, not a fetch failure (see this module's doc).
    Empty { pane: PaneRegion },
    Replace {
        pane: PaneRegion,
        range: RCRange,
        cells: FetchedCells,
        fingerprint: PaneFingerprint,
    },
    Splice {
        pane: PaneRegion,
        range: RCRange,
        cells: FetchedCells,
        fingerprint: PreparedFingerprintUpdate,
    },
}

/// Every pane's confirmed-safe installation for one paint attempt. A held
/// pane contributes no entry; a partial commit contains entries only for
/// the panes that actually executed. Execution aggregates these entries and
/// the orchestrator installs them once at its completion boundary.
#[derive(Default)]
pub(crate) struct PreparedCacheCommit {
    panes: Vec<PaneCacheCommit>,
}

impl PreparedCacheCommit {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            panes: Vec::with_capacity(capacity),
        }
    }

    pub(crate) fn push(&mut self, commit: PaneCacheCommit) {
        self.panes.push(commit);
    }
}

// ==============================================================================
// Prepared pane work
// ==============================================================================

/// The already-decided Skip/Rows/Full verdict for one full-pane
/// preparation, plus the candidate fingerprint tree to install at commit.
/// `candidate` is an owned value built fresh by preparation (reusing warm
/// `Vec` capacity internally — see [`super::cache::PaneFingerprintState::build_candidate`])
/// rather than left sitting in `PaneFingerprintState`'s persistent scratch
/// slot, so a preparation that never reaches commit never leaves a
/// half-finished tree in state a later frame could observe.
pub(crate) struct PreparedRepaint {
    pub(crate) plan: RepaintPlan,
    pub(crate) candidate: PaneFingerprint,
}

/// One already-fetched, already-validated Damage row-band strip, ready to
/// splice and paint once every sibling strip prepared for the same pane is
/// also confirmed clean.
pub(crate) struct PreparedStrip {
    pub(crate) range: RCRange,
    pub(crate) fetched: FetchedCells,
}

/// One pane's prepared paint work, built only by this module's own
/// `prepare_*`/`build_*` methods — never assembled ad hoc, so a caller can
/// never pair a pane's fetched data with the wrong dispatch.
pub(crate) enum PreparedPane {
    /// No live address-space range this frame — nothing to fetch or paint,
    /// just an explicit cache-forget action (see [`PaneCacheCommit::Empty`]).
    Empty {
        pane: PaneRegion,
        cache_action: PaneCacheAction,
    },
    Full {
        pane: PaneRegion,
        range: RCRange,
        fetched: FetchedCells,
        repaint: PreparedRepaint,
        cache_action: PaneCacheAction,
    },
    Damage {
        pane: PaneRegion,
        range: RCRange,
        strips: Vec<PreparedStrip>,
        cache_action: PaneCacheAction,
    },
    /// One shifted pane's blit work: the widened strip/clip geometry, the
    /// already-fetched and already-bridge-validated revealed strip, the
    /// `Shift` cache action naming the rotation execution still owes this
    /// pane's buffers, and the fingerprint policy this shift's shape and
    /// history earned. Built only by [`RendererCore::prepare_blit`].
    Blit {
        work: BlitPaneWork,
        fetched: FetchedCells,
        fingerprint: PreparedFingerprintUpdate,
        cache_action: PaneCacheAction,
    },
}

impl PreparedPane {
    /// The pane this prepared work belongs to, regardless of variant —
    /// exhaustive so a new variant fails to compile here rather than
    /// silently defaulting.
    pub(crate) fn region(&self) -> PaneRegion {
        match self {
            PreparedPane::Empty { pane, .. }
            | PreparedPane::Full { pane, .. }
            | PreparedPane::Damage { pane, .. } => *pane,
            PreparedPane::Blit { work, .. } => work.pane,
        }
    }
}

/// One blit attempt's complete, owned preparation: every `plan.shift_panes()`
/// pane classified and fetched exactly once, before any pixel shifts. `None`
/// from [`RendererCore::prepare_blit`] (rather than a partially-filled value)
/// is the whole-frame-hold signal — see that method's doc.
///
/// Deliberately does not also carry the reversible `Chrome` candidate
/// ([`crate::chrome::PreparedBlitFrame`], Task 3) or the pixel-only
/// [`BlitPlan`]: both are already owned by the one caller
/// (`Orchestrator::paint_viewport_regime`) that holds the commit-or-rollback
/// decision open, and `RendererCore::render_grid_blit` (this value's only
/// consumer) already receives `frame`/`plan` as its own parameters. Bundling
/// them again here would duplicate ownership of state the renderer never
/// mutates, not compose it.
pub(crate) struct PreparedBlit {
    panes: Vec<PreparedPane>,
}

// ==============================================================================
// RendererCore: prepare / execute / commit
// ==============================================================================

impl<P: Painter> RendererCore<P> {
    fn take_strip_scratch(&self) -> FetchedCells {
        self.frame_cache
            .strip_scratch
            .borrow_mut()
            .pop()
            .unwrap_or_default()
    }

    fn park_strip_scratch(&self, cells: FetchedCells) {
        self.frame_cache.strip_scratch.borrow_mut().push(cells);
    }

    pub(super) fn recycle_prepared_pane(&self, prepared: PreparedPane) {
        match prepared {
            PreparedPane::Empty { .. } => {}
            PreparedPane::Full {
                pane,
                fetched,
                repaint,
                ..
            } => {
                let pane_buf = self.pane_cache.pane(pane);
                pane_buf.park_prepare_scratch(fetched);
                pane_buf.fingerprint.recycle_candidate(repaint.candidate);
            }
            PreparedPane::Damage { strips, .. } => {
                for strip in strips {
                    self.park_strip_scratch(strip.fetched);
                }
            }
            PreparedPane::Blit {
                work,
                fetched,
                fingerprint,
                ..
            } => {
                let pane_buf = self.pane_cache.pane(work.pane);
                pane_buf.park_prepare_scratch(fetched);
                // A candidate this attempt never commits goes back to the warm
                // scratch slot as capacity — the same abort rhythm a held
                // full-pane preparation follows. `recycle_candidate` writes
                // only that non-semantic slot, so nothing here can install the
                // rotation the failed attempt was going to earn.
                if let PreparedFingerprintUpdate::Install(candidate) = fingerprint {
                    pane_buf.fingerprint.recycle_candidate(candidate);
                }
            }
        }
    }

    pub(super) fn recycle_prepared_panes(&self, prepared: impl IntoIterator<Item = PreparedPane>) {
        for pane in prepared {
            self.recycle_prepared_pane(pane);
        }
    }

    /// Pure: fetch one pane's full address-space range and decide
    /// Skip/Rows/Full against the pane's committed `painted` fingerprint
    /// tree, without installing anything. Fetches into the pane's parked
    /// `prepare_scratch` — never the committed `styles`/`values`/
    /// `cell_types`/`decorations`/`range` fields — so a `None` return (this
    /// frame kind reuses slots and the fetch's four channels report a
    /// bridge failure) leaves the committed cache untouched: the caller
    /// only has to avoid consuming a `None`, never undo a mutation.
    ///
    /// `fetched.has_bridge_failure()` is always computed. Slots-reuse holds
    /// here; Fresh's all-pane preparation performs the corresponding atomic
    /// check after this method returns the owned pane value.
    pub(super) fn prepare_full_pane(
        &self,
        model: &dyn CellContentQuery,
        pane: PaneRegion,
        range: RCRange,
        frame: &Chrome,
    ) -> Option<PreparedPane> {
        let pane_buf = self.pane_cache.pane(pane);
        let scratch = pane_buf.take_prepare_scratch();
        let fetched = FetchedCells::fetch_into(model, frame.sheet, range, scratch);
        self.trace_fetch(range);

        if frame.kind.reuses_slots() && fetched.has_bridge_failure() {
            pane_buf.park_prepare_scratch(fetched);
            return None;
        }

        Some(self.build_prepared_full_pane(pane, range, frame, fetched))
    }

    /// Tail of [`Self::prepare_full_pane`]: build the candidate fingerprint
    /// tree and the Skip/Rows/Full verdict. [`Self::prepare_blit`]'s
    /// `MissingCache`/`IncompatibleRange` fallback calls `prepare_full_pane`
    /// itself — the same one fetch `render_pane` uses, never this tail
    /// directly — so a blit-scope pane's fallback fetch is indistinguishable
    /// from an ordinary full-pane prepare. `!frame.kind.reuses_slots()` (a
    /// `Fresh` candidate) always forces
    /// `RepaintPlan::Full` without running the comparison at all — a Fresh
    /// frame has no prior valid pixels of its own to partially preserve, so
    /// a coincidental digest match against a stale `painted` tree must
    /// never be read as "skip".
    fn build_prepared_full_pane(
        &self,
        pane: PaneRegion,
        range: RCRange,
        frame: &Chrome,
        fetched: FetchedCells,
    ) -> PreparedPane {
        let pane_buf = self.pane_cache.pane(pane);
        let candidate = pane_buf.fingerprint.build_candidate(
            fetched.styles(),
            fetched.values(),
            fetched.cell_types(),
            fetched.decorations(),
            range,
        );
        let plan = if frame.kind.reuses_slots() {
            pane_buf.fingerprint.compare_to_painted(&candidate)
        } else {
            RepaintPlan::Full
        };
        PreparedPane::Full {
            pane,
            range,
            fetched,
            repaint: PreparedRepaint { plan, candidate },
            cache_action: PaneCacheAction::Replace { range },
        }
    }

    /// Infallible: paint the prepared full-pane work into the backing
    /// target and return an owned commit — this never writes
    /// `PaneBuffers`/`PaneFingerprintState` itself (see
    /// `RendererCore::commit_pane_cache`). Clears the pane bg first only
    /// for a genuine `RepaintPlan::Full` on a slots-reused frame (prior
    /// pixels exist and may show cells whose data just disappeared); a
    /// `Fresh` candidate's forced Full never clears here because the
    /// caller owns clearing the whole canvas.
    pub(super) fn execute_full_pane(
        &self,
        frame: &Chrome,
        prepared: PreparedPane,
    ) -> PaneCacheCommit {
        let PreparedPane::Full {
            pane,
            range,
            mut fetched,
            repaint,
            cache_action,
        } = prepared
        else {
            unreachable!("execute_full_pane only ever receives PreparedPane::Full")
        };
        let PaneCacheAction::Replace {
            range: commit_range,
        } = cache_action
        else {
            unreachable!("PreparedPane::Full always carries a Replace cache action")
        };
        debug_assert_eq!(
            range, commit_range,
            "a full-pane prepared value's range and cache_action must always agree"
        );

        let theme = &frame.theme;
        let reuses_slots = frame.kind.reuses_slots();

        match &repaint.plan {
            RepaintPlan::Skip => {}
            RepaintPlan::Rows(spans) => {
                for span in spans {
                    let band = RCRange {
                        r1: span.r1,
                        c1: range.c1,
                        r2: span.r2,
                        c2: range.c2,
                    };
                    if let Some(band_rect) = frame.range_rect(band) {
                        self.painter
                            .rect_fill(band_rect, PaintColor::from_theme_str(&theme.cell_bg));
                    }
                    self.paint_cells_pass(
                        PaneCells::for_strip(&pane, frame, band),
                        range,
                        theme,
                        fetched.as_mut(),
                    );
                }
            }
            RepaintPlan::Full => {
                if reuses_slots && let Some(pane_rect) = frame.range_rect(range) {
                    self.painter
                        .rect_fill(pane_rect, PaintColor::from_theme_str(&theme.cell_bg));
                }
                self.paint_cells_pass(PaneCells::new(&pane, frame), range, theme, fetched.as_mut());
            }
        }

        PaneCacheCommit::Replace {
            pane,
            range,
            cells: fetched,
            fingerprint: repaint.candidate,
        }
    }

    /// Pure: clip every span in `spans` to `range`, then fetch each
    /// intersecting strip in order, stopping at the first bridge failure —
    /// an unfetched later span costs nothing, matching the existing
    /// short-circuit `render_pane_damage` already relied on. Critically,
    /// NO strip is spliced or painted here: a later span's failure must
    /// never leave an earlier, successfully fetched span partially
    /// committed. Returns `None` (held) on any failure; `Some(strips)`
    /// otherwise, `strips` empty when no span intersected this pane's
    /// range at all.
    pub(super) fn prepare_damage_pane(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        pane: PaneRegion,
        range: RCRange,
        spans: &[RowSpan],
    ) -> Option<PreparedPane> {
        let mut strips: Vec<PreparedStrip> = Vec::new();

        for span in spans {
            let r1 = span.r1.max(range.r1);
            let r2 = span.r2.min(range.r2);
            if r1 > r2 {
                continue;
            }
            let strip_range = RCRange {
                r1,
                c1: range.c1,
                r2,
                c2: range.c2,
            };

            let scratch = self.take_strip_scratch();
            let fetched = FetchedCells::fetch_into(model, frame.sheet, strip_range, scratch);
            self.trace_fetch(strip_range);

            if fetched.has_bridge_failure() {
                self.park_strip_scratch(fetched);
                for prepared in strips {
                    self.park_strip_scratch(prepared.fetched);
                }
                self.trace_pane(pane, PaneVerdict::Held);
                return None;
            }
            strips.push(PreparedStrip {
                range: strip_range,
                fetched,
            });
        }

        let strip_ranges = strips.iter().map(|s| s.range).collect();
        Some(PreparedPane::Damage {
            pane,
            range,
            strips,
            cache_action: PaneCacheAction::Splice {
                range,
                strips: strip_ranges,
            },
        })
    }

    /// Infallible: splice every prepared strip into the pane's committed
    /// buffers (taken once for the whole batch, not once per strip) and
    /// paint each strip's own band, in order. It takes the pane's committed
    /// buffers as an execution-owned value and returns them in the commit;
    /// range/fingerprint installation remains deferred. Only reachable once every
    /// strip in `prepared` already cleared `prepare_damage_pane`'s bridge
    /// check, so this function has no failure branch of its own.
    pub(super) fn execute_damage_pane(
        &self,
        frame: &Chrome,
        prepared: PreparedPane,
    ) -> PaneCacheCommit {
        let PreparedPane::Damage {
            pane,
            range,
            strips,
            cache_action,
        } = prepared
        else {
            unreachable!("execute_damage_pane only ever receives PreparedPane::Damage")
        };
        let PaneCacheAction::Splice {
            range: commit_range,
            strips: prepared_strip_ranges,
        } = cache_action
        else {
            unreachable!("PreparedPane::Damage always carries a Splice cache action")
        };
        debug_assert_eq!(
            range, commit_range,
            "a Damage prepared value's range and cache_action must always agree"
        );
        debug_assert_eq!(
            strips.len(),
            prepared_strip_ranges.len(),
            "prepare_damage_pane's strips and cache_action.strips must name the same set"
        );

        let theme = &frame.theme;
        let pane_buf = self.pane_cache.pane(pane);
        let mut cells = pane_buf.take_cells();

        for strip in strips {
            let PreparedStrip {
                range: strip_range,
                fetched: mut strip_cells,
            } = strip;

            cells.splice_strip_from(&mut strip_cells, range, strip_range);

            if let Some(strip_rect) = frame.range_rect(strip_range) {
                self.painter
                    .rect_fill(strip_rect, PaintColor::from_theme_str(&theme.cell_bg));
            }
            self.paint_cells_pass(
                PaneCells::for_strip(&pane, frame, strip_range),
                range,
                theme,
                cells.as_mut(),
            );

            self.park_strip_scratch(strip_cells);
        }

        self.trace_pane(pane, PaneVerdict::Strip);

        PaneCacheCommit::Splice {
            pane,
            range,
            cells,
            // A Damage band is never proof of border-safe pixels: clearing and
            // repainting a band cannot undo a medium/thick border that used to
            // bleed *outside* it, so the pixels above and below the band may
            // still show strokes no candidate would describe. Damage therefore
            // has no `Install` case at all — the next whole-pane comparison
            // against the retained tree is what heals it.
            fingerprint: PreparedFingerprintUpdate::MarkStale,
        }
    }

    /// Pure (with respect to committed state): classify and fetch every
    /// `plan.shift_panes()` pane exactly once, returning a fully-prepared
    /// [`PreparedBlit`] on success or `None` the instant any required pane's
    /// fetch reports a bridge failure — always *before* the caller shifts a
    /// single pixel. `render_grid_blit` is this method's only caller, and
    /// checks `None` before its first `Painter::blit`.
    ///
    /// Per pane: no live range ([`PaneRegion::range`] is `None`) prepares an
    /// [`PreparedPane::Empty`]; [`blit_work::shifted_pane_work`] classifying
    /// `Shifted` fetches the revealed strip once and prepares a
    /// [`PreparedPane::Blit`]; anything else (never cached, an incompatible
    /// cached range, or the defensive zero-delta guard) falls back to the
    /// SAME [`Self::prepare_full_pane`] every other full-pane caller uses —
    /// one fetch, never a safety fetch followed by a second refetch.
    pub(super) fn prepare_blit(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> Option<PreparedBlit> {
        let mask = plan.shift_panes();
        let mut panes = Vec::with_capacity(mask.regions().count());
        for pane in mask.regions() {
            let Some(new_range) = pane.range(frame) else {
                panes.push(PreparedPane::Empty {
                    pane,
                    cache_action: PaneCacheAction::Empty,
                });
                continue;
            };

            match blit_work::shifted_pane_work(&self.pane_cache, frame, plan, pane) {
                Some(work) => {
                    let pane_buf = self.pane_cache.pane(pane);
                    let scratch = pane_buf.take_prepare_scratch();
                    let fetched =
                        FetchedCells::fetch_into(model, frame.sheet, work.strip_range, scratch);
                    self.trace_fetch(work.strip_range);

                    if fetched.has_bridge_failure() {
                        pane_buf.park_prepare_scratch(fetched);
                        self.recycle_prepared_panes(panes);
                        self.trace_frame_held(pane);
                        return None;
                    }
                    // Built here, before `execute_blit` shifts a pixel and
                    // before `paint_cells_pass` drains the strip's fetched
                    // values — the rotation reads the same strip the painter
                    // is about to consume, and the same painted tree this
                    // attempt may still abandon.
                    let fingerprint = match pane_buf.fingerprint.build_row_shift_candidate(
                        work.prev_range,
                        work.new_range,
                        work.axis,
                        &fetched,
                        work.strip_range,
                    ) {
                        RowShiftFingerprint::Rotated(candidate) => {
                            PreparedFingerprintUpdate::Install(candidate)
                        }
                        // Every rejection — a column-axis shift, history that
                        // isn't `Exact`, a shape the rotation refuses — is the
                        // same conservative answer: keep the tree comparable,
                        // stop treating it as carryable.
                        RowShiftFingerprint::Ineligible(_) => PreparedFingerprintUpdate::MarkStale,
                    };
                    let cache_action = PaneCacheAction::Shift {
                        prev_range: work.prev_range,
                        new_range: work.new_range,
                        axis: work.axis,
                    };
                    panes.push(PreparedPane::Blit {
                        work,
                        fetched,
                        fingerprint,
                        cache_action,
                    });
                }
                None => {
                    // Every reason `shifted_pane_work` returns `None` here
                    // (never cached, incompatible range, zero-delta guard)
                    // routes uniformly to the full-pane fallback — this pane
                    // already proved it has a live range above, so it is
                    // exactly the "lost the strip path" case the trace names.
                    let cold_cache = self.pane_cache.pane(pane).range.get().is_none();
                    self.trace_blit_fallback(pane, cold_cache);

                    match self.prepare_full_pane(model, pane, new_range, frame) {
                        Some(prepared) => panes.push(prepared),
                        None => {
                            self.recycle_prepared_panes(panes);
                            self.trace_frame_held(pane);
                            return None;
                        }
                    }
                }
            }
        }
        Some(PreparedBlit { panes })
    }

    /// Infallible: execute every prepared pane in `prepared` — shift pixels
    /// are the caller's job (`render_grid_blit`, immediately before this
    /// call); this paints each pane's prepared strip/full-pane work and
    /// returns one aggregate commit. No model bulk fetch or shift
    /// reclassification happens here — every pane in `prepared` already
    /// cleared `Self::prepare_blit`'s bridge check.
    pub(super) fn execute_blit(
        &self,
        frame: &Chrome,
        prepared: PreparedBlit,
    ) -> PreparedCacheCommit {
        let mut cache_commit = PreparedCacheCommit::with_capacity(prepared.panes.len());
        for pane_work in prepared.panes {
            let region = pane_work.region();
            let (verdict, pixel_clip) = match &pane_work {
                PreparedPane::Empty { .. } => (None, None),
                PreparedPane::Full { repaint, .. } => {
                    (Some(PaneVerdict::from(&repaint.plan)), None)
                }
                PreparedPane::Blit { work, .. } => (Some(PaneVerdict::Strip), work.pixel_clip),
                PreparedPane::Damage { .. } => {
                    unreachable!("prepare_blit never constructs PreparedPane::Damage")
                }
            };
            if let Some(v) = verdict {
                self.trace_pane(region, v);
            }
            if let Some(clip) = pixel_clip {
                self.painter.push_clip(clip);
            }
            let commit = match pane_work {
                PreparedPane::Empty { pane, cache_action } => {
                    debug_assert!(
                        matches!(cache_action, PaneCacheAction::Empty),
                        "PreparedPane::Empty always carries an Empty cache action"
                    );
                    PaneCacheCommit::Empty { pane }
                }
                PreparedPane::Full { .. } => self.execute_full_pane(frame, pane_work),
                PreparedPane::Blit { .. } => self.execute_blit_pane(frame, pane_work),
                PreparedPane::Damage { .. } => {
                    unreachable!("prepare_blit never constructs PreparedPane::Damage")
                }
            };
            if pixel_clip.is_some() {
                self.painter.pop_clip();
            }
            cache_commit.push(commit);
        }
        cache_commit
    }

    /// Infallible: rotate this pane's cached buffers from `prev_range` to
    /// `new_range` ([`super::cache::PaneBuffers::apply_shift`] — never run
    /// during preparation, see that method's doc), splice the already-fetched
    /// revealed strip into the freshly-rotated slots, and paint the strip's
    /// cells. It takes the pane's committed buffers into the returned commit;
    /// range/fingerprint installation remains deferred to
    /// [`Self::commit_pane_cache`]. The pixel clip (main scroll pane only)
    /// is the caller's concern, wrapping this whole call — see
    /// [`Self::execute_blit`].
    fn execute_blit_pane(&self, frame: &Chrome, prepared: PreparedPane) -> PaneCacheCommit {
        let PreparedPane::Blit {
            work,
            fetched: mut strip_cells,
            fingerprint,
            cache_action,
        } = prepared
        else {
            unreachable!("execute_blit_pane only ever receives PreparedPane::Blit")
        };
        let PaneCacheAction::Shift {
            prev_range,
            new_range,
            axis,
        } = cache_action
        else {
            unreachable!("PreparedPane::Blit always carries a Shift cache action")
        };
        debug_assert_eq!(
            (work.prev_range, work.new_range),
            (prev_range, new_range),
            "a Blit prepared value's work and cache_action ranges must always agree"
        );

        let pane_buf = self.pane_cache.pane(work.pane);
        pane_buf.apply_shift(prev_range, new_range, axis);

        let theme = &frame.theme;
        let mut cells = pane_buf.take_cells();

        cells.splice_strip_from(&mut strip_cells, new_range, work.strip_range);

        if let Some(strip_rect) = frame.range_rect(work.strip_range) {
            self.painter
                .rect_fill(strip_rect, PaintColor::from_theme_str(&theme.cell_bg));
        }
        self.paint_cells_pass(
            PaneCells::for_strip(&work.pane, frame, work.strip_range),
            new_range,
            theme,
            cells.as_mut(),
        );

        // Park the drained (post-splice, placeholder-only) strip buffers as
        // this pane's next full-pane prepare_scratch — same capacity-reuse
        // rhythm as a Replace commit's evicted-old-cells park.
        pane_buf.park_prepare_scratch(strip_cells);

        PaneCacheCommit::Splice {
            pane: work.pane,
            range: new_range,
            cells,
            fingerprint,
        }
    }

    /// The orchestrated cache-completion entry point: installs every pane's
    /// confirmed commit into the persistent `PaneCache`. Damage/Blit
    /// execution may first transfer committed cell Vecs into the owned
    /// result, but range and fingerprint metadata are installed only here.
    /// Every entry already cleared its required bridge checks.
    pub(super) fn commit_pane_cache(&self, commit: PreparedCacheCommit) {
        for pane_commit in commit.panes {
            self.install_pane_cache_commit(pane_commit);
        }
    }

    pub(super) fn install_pane_cache_commit(&self, commit: PaneCacheCommit) {
        match commit {
            PaneCacheCommit::Empty { pane } => {
                let pane_buf = self.pane_cache.pane(pane);
                pane_buf.range.set(None);
                // Forgetting the buffer range without forgetting the tree: the
                // tree still describes a real past paint and stays comparable,
                // but a pane that just lost its range has no history any later
                // shift may carry forward.
                pane_buf.fingerprint.mark_stale();
            }
            PaneCacheCommit::Replace {
                pane,
                range,
                cells,
                fingerprint,
            } => {
                let pane_buf = self.pane_cache.pane(pane);
                let old = pane_buf.install_cells(cells);
                pane_buf.park_prepare_scratch(old);
                pane_buf.range.set(Some(range));
                pane_buf.fingerprint.install(fingerprint);
            }
            PaneCacheCommit::Splice {
                pane,
                range,
                cells,
                fingerprint,
            } => {
                let pane_buf = self.pane_cache.pane(pane);
                pane_buf.set_cells(cells);
                pane_buf.range.set(Some(range));
                // The one place a strip's fingerprint decision takes effect,
                // alongside the very cells and range it describes.
                match fingerprint {
                    PreparedFingerprintUpdate::Install(candidate) => {
                        pane_buf.fingerprint.install(candidate)
                    }
                    PreparedFingerprintUpdate::MarkStale => pane_buf.fingerprint.mark_stale(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range_2x2() -> RCRange {
        RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        }
    }

    /// Minimal `CellContentQuery` double: implements only the required
    /// single-cell accessors and lets the trait's own default bulk `*_in`
    /// loops drive `FetchedCells::fetch_into`, exactly like `UserModel`'s
    /// unbatched path in production.
    struct FetchModel {
        fail: bool,
    }

    impl CellContentQuery for FetchModel {
        fn get_cell_style(&self, _sheet: u32, _row: i32, _column: i32) -> Fetched<CellStyle> {
            if self.fail {
                Fetched::BridgeFailed
            } else {
                Fetched::Value(CellStyle::default())
            }
        }

        fn get_cell_type(&self, _sheet: u32, _row: i32, _column: i32) -> Fetched<CellKind> {
            if self.fail {
                Fetched::BridgeFailed
            } else {
                Fetched::Value(CellKind::Text)
            }
        }

        fn get_formatted_cell_value(
            &self,
            _sheet: u32,
            _row: i32,
            _column: i32,
        ) -> Fetched<String> {
            if self.fail {
                Fetched::BridgeFailed
            } else {
                Fetched::Value(String::new())
            }
        }
    }

    // Stage 4 (Task 2, final bullet): allocation capacity is parked for
    // reuse after a SUCCESSFUL fetch — a second `fetch_into` call handed
    // the first call's bundle back as `reuse` must not grow any of the
    // four `Vec`s further. Asserts capacity (a concrete, checkable proxy
    // for "no allocation happened" — see `rebuild_in_place_keeps_row_and_cell_vec_capacities_warm`
    // in `cell::fingerprint` for the same pattern), never pointer
    // identity.
    #[test]
    fn fetch_into_parks_allocation_capacity_for_reuse_after_success() {
        let model = FetchModel { fail: false };
        let range = range_2x2();

        let first = FetchedCells::fetch_into(&model, 0, range, FetchedCells::default());
        assert!(!first.has_bridge_failure());
        let warmed = first.capacities();
        assert!(
            warmed.0 > 0 && warmed.1 > 0 && warmed.2 > 0 && warmed.3 > 0,
            "a real fetch over a non-empty range must actually allocate"
        );

        let second = FetchedCells::fetch_into(&model, 0, range, first);
        assert_eq!(
            second.capacities(),
            warmed,
            "reusing a prior bundle as `reuse` must not grow any of the four Vecs"
        );
    }

    // Same property, but the bundle being recycled came back from a FAILED
    // fetch — this is exactly `render_pane`'s held path: park the failed
    // bundle, then a later successful retry must still reuse its capacity
    // rather than allocating fresh.
    #[test]
    fn fetch_into_parks_allocation_capacity_for_reuse_after_failure() {
        let failing = FetchModel { fail: true };
        let range = range_2x2();

        let failed = FetchedCells::fetch_into(&failing, 0, range, FetchedCells::default());
        assert!(failed.has_bridge_failure());
        let warmed = failed.capacities();
        assert!(
            warmed.0 > 0 && warmed.1 > 0 && warmed.2 > 0 && warmed.3 > 0,
            "a failed fetch over a non-empty range still allocates the four Vecs"
        );

        let healthy = FetchModel { fail: false };
        let retried = FetchedCells::fetch_into(&healthy, 0, range, failed);
        assert!(!retried.has_bridge_failure());
        assert_eq!(
            retried.capacities(),
            warmed,
            "a retry that reuses the failed bundle must not grow any of the four Vecs"
        );
    }

    fn cell_count(range: RCRange) -> usize {
        ((range.r2 - range.r1 + 1) * (range.c2 - range.c1 + 1)) as usize
    }

    fn absent_bundle(len: usize) -> FetchedCells {
        FetchedCells::from_parts(
            vec![Fetched::Absent; len],
            vec![Fetched::Absent; len],
            vec![Fetched::Absent; len],
            vec![Fetched::Absent; len],
        )
    }

    fn value_bundle(len: usize) -> FetchedCells {
        FetchedCells::from_parts(
            vec![Fetched::Value(CellStyle::default()); len],
            vec![Fetched::Value("x".to_string()); len],
            vec![Fetched::Value(CellKind::Number); len],
            vec![Fetched::Value(CellDecoration::Icon("star".to_string())); len],
        )
    }

    fn value_positions<T>(items: &[Fetched<T>]) -> Vec<usize> {
        items
            .iter()
            .enumerate()
            .filter(|(_, slot)| matches!(slot, Fetched::Value(_)))
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Splice an all-`Value` strip into an all-`Absent` pane bundle through
    /// the real `splice_strip_from`, and report which pane slots each channel
    /// received a value at. The markers are variant-level on purpose: the
    /// property under test is that ONE index mapping reaches all four
    /// channels, and re-deriving that mapping in the test would just
    /// duplicate `splice_strip_into`. Also asserts the strip came back fully
    /// drained, since its owner parks it straight into scratch.
    fn spliced_value_positions(pane_range: RCRange, strip_range: RCRange) -> [Vec<usize>; 4] {
        let mut pane = absent_bundle(cell_count(pane_range));
        let mut strip = value_bundle(cell_count(strip_range));

        pane.splice_strip_from(&mut strip, pane_range, strip_range);

        assert!(
            value_positions(strip.styles()).is_empty()
                && value_positions(strip.values()).is_empty()
                && value_positions(strip.cell_types()).is_empty()
                && value_positions(strip.decorations()).is_empty(),
            "every channel's strip slots must be drained into the pane, not copied"
        );
        [
            value_positions(pane.styles()),
            value_positions(pane.values()),
            value_positions(pane.cell_types()),
            value_positions(pane.decorations()),
        ]
    }

    // Damage splices row bands; a blit splices whichever axis scrolled. Both
    // go through the one bundle method, so a channel that drifted out of
    // lockstep would land its cells at different slots than its siblings.
    #[test]
    fn splice_strip_from_maps_row_and_column_strips_identically_across_channels() {
        let pane_range = RCRange {
            r1: 10,
            c1: 5,
            r2: 12,
            c2: 7,
        };

        let row_strip = RCRange {
            r1: 11,
            c1: 5,
            r2: 11,
            c2: 7,
        };
        assert_eq!(
            spliced_value_positions(pane_range, row_strip),
            [vec![3, 4, 5], vec![3, 4, 5], vec![3, 4, 5], vec![3, 4, 5]],
            "a middle row band must land on the pane's second row in every channel"
        );

        let column_strip = RCRange {
            r1: 10,
            c1: 6,
            r2: 12,
            c2: 6,
        };
        assert_eq!(
            spliced_value_positions(pane_range, column_strip),
            [vec![1, 4, 7], vec![1, 4, 7], vec![1, 4, 7], vec![1, 4, 7]],
            "a middle column band must land on the pane's second column in every channel"
        );
    }

    #[test]
    fn has_bridge_failure_is_false_for_a_clean_fetch() {
        let model = FetchModel { fail: false };
        let fetched = FetchedCells::fetch_into(&model, 0, range_2x2(), FetchedCells::default());
        assert!(!fetched.has_bridge_failure());
    }

    #[test]
    fn has_bridge_failure_is_true_when_any_channel_fails() {
        let model = FetchModel { fail: true };
        let fetched = FetchedCells::fetch_into(&model, 0, range_2x2(), FetchedCells::default());
        assert!(fetched.has_bridge_failure());
    }
}
