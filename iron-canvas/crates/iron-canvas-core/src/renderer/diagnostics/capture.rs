//! Capture writers for the per-attempt diagnostic snapshot.
//!
//! One method per renderer section. Each writer appends to the in-flight
//! capture owned by [`DiagState`](super::DiagState) and returns immediately
//! while capture is disabled, so a production build performs no work here.
//! The completion boundary in [`super`] seals the buffer.

use crate::chrome::{Chrome, GridLayout, PaneRegion};
use crate::frame::BlitPlan;
use crate::frame::GridVerdict;
use crate::frame::RebuildReason;
use crate::frame::work::RowSpan;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::renderer::prepared::{
    FetchedCells, PreparedFingerprintUpdate, PreparedRepaint, PreparedRepaintPlan, PreparedStrip,
};
use crate::renderer::repaint::plan::RepaintReason;
use crate::types::coord::RCRange;

use super::snapshot::{
    DIAG_SCHEMA_VERSION, DiagBlit, DiagBlitResultTag, DiagCache, DiagCacheActionTag,
    DiagChangedCell, DiagDeltaKind, DiagFetchPurpose, DiagFetchRequest, DiagFingerprintActionTag,
    DiagGeometry, DiagRepaintReason, DiagRevealedStrip, DiagSegment, DiagSourceRange,
    FrameDiagnostics,
};

impl<P: crate::painter::Painter> crate::renderer::RendererCore<P> {
    /// Classification facts plus the attempt-start committed cache truth,
    /// recorded once by `render_pending` after `plan_frame`. The
    /// capture-failure path never calls this — its snapshot keeps
    /// `delta`/`rebuild_reason` at `None` and `committed_before` is filled
    /// by `publish_diag`.
    pub(crate) fn diag_begin_attempt(
        &self,
        delta: DiagDeltaKind,
        rebuild_reason: Option<RebuildReason>,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.capture.borrow_mut();
        *slot = Some(FrameDiagnostics {
            schema_version: DIAG_SCHEMA_VERSION,
            delta: Some(delta),
            rebuild_reason,
            cache: DiagCache {
                committed_before: Some(self.cache_truth_now()),
                ..DiagCache::default()
            },
            ..FrameDiagnostics::default()
        });
    }

    /// Geometry of the frame a grid prepare is about to paint against.
    /// Called from every grid prepare entry point; idempotent overwrite.
    pub(crate) fn diag_geometry(&self, frame: &Chrome, layout: GridLayout) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.geometry = Some(DiagGeometry {
            canvas: frame.canvas_size(),
            backing_size: frame.metrics().backing_size(),
            dpr: frame.dpr(),
            sheet: frame.sheet,
            top_row: frame.pane_set.top_row(),
            left_column: frame.pane_set.left_column(),
            row_header_thickness: frame.row_header_thickness,
            col_header_thickness: frame.col_header_thickness,
            show_row_headers: frame.show_row_headers,
            show_col_headers: frame.show_col_headers,
            shape: layout.shape(),
            segments: layout
                .segments()
                .map(|segment| DiagSegment {
                    region: segment.region(),
                    range: segment.range(),
                    cells: FetchedCells::addressed_cells(segment.range()),
                })
                .collect(),
        });
    }

    /// One renderer-owned bundle fetch over `range`. Mirrors the existing
    /// `trace_fetch` counters with per-request attribution.
    pub(crate) fn diag_fetch(
        &self,
        purpose: DiagFetchPurpose,
        region: Option<PaneRegion>,
        range: RCRange,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.fetch.batches += 1;
        capture.fetch.addressed_cells += FetchedCells::addressed_cells(range);
        capture.fetch.logical_slots += FetchedCells::logical_channel_slots(range);
        capture.fetch.requests.push(DiagFetchRequest {
            purpose,
            region,
            range,
            cells: FetchedCells::addressed_cells(range),
            slots: FetchedCells::logical_channel_slots(range),
        });
    }

    /// Grid verdict plus the fingerprint branch reason (when a comparison
    /// ran) and the exact changed row spans. The single recorder for all
    /// three grid arms — Full (with comparison reason), Damage and Blit
    /// (both `Strip`, no reason) — so the structured snapshot never
    /// disagrees with the one-line trace.
    pub(crate) fn diag_repaint(
        &self,
        verdict: GridVerdict,
        reason: Option<RepaintReason>,
        changed_rows: &[RowSpan],
        changed_cells: &[RCRange],
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.repaint.verdict = Some(verdict);
        capture.repaint.reason = reason.map(|reason| match reason {
            RepaintReason::NoPaintedHistory => DiagRepaintReason::NoPaintedHistory,
            RepaintReason::LayoutMismatch => DiagRepaintReason::LayoutMismatch,
            RepaintReason::RowAddressMismatch => DiagRepaintReason::RowAddressMismatch,
            RepaintReason::FingerprintsEqual => DiagRepaintReason::FingerprintsEqual,
            RepaintReason::ChangedCell => DiagRepaintReason::ChangedCell,
            RepaintReason::ChangedCells => DiagRepaintReason::ChangedCells,
            RepaintReason::ChangedRows => DiagRepaintReason::ChangedRows,
            RepaintReason::ClipAlignment => DiagRepaintReason::ClipAlignment,
        });
        capture.repaint.changed_rows = changed_rows.to_vec();
        capture.repaint.changed_cells = changed_cells
            .iter()
            .map(|cell| {
                let cell = cell.normalized();
                DiagChangedCell {
                    row: cell.r1,
                    column: cell.c1,
                }
            })
            .collect();
    }

    pub(crate) fn diag_repaint_envelope(
        &self,
        clip: Option<PixelRect>,
        sources: &[Option<RCRange>; 4],
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.repaint.clip = clip;
        capture.repaint.source_ranges = [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ]
        .into_iter()
        .filter_map(|region| sources[region.index()].map(|range| DiagSourceRange { region, range }))
        .collect();
    }

    /// Prepared grid-cache transition tag, recorded once by each prepare entry
    /// point.
    pub(crate) fn diag_cache_planned(&self, action: DiagCacheActionTag) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.cache.planned_action = Some(action);
    }

    /// Fingerprint update carried by the prepared commit, recorded at each
    /// execute arm where the commit installs it.
    pub(crate) fn diag_fingerprint_action(&self, action: DiagFingerprintActionTag) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.cache.fingerprint_action = Some(action);
    }

    /// Blit detail for a `ScrollBlit` attempt. `delta` is derived from the
    /// committed vs candidate scroll-band origin — the renderer never
    /// re-reads the model for it. `clip` is recorded separately at the
    /// `push_clip` call site, not derived here.
    pub(crate) fn diag_blit(
        &self,
        plan: &BlitPlan,
        result: DiagBlitResultTag,
        cold_cache: Option<bool>,
        previous: Option<GridLayout>,
        candidate: GridLayout,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let delta = match (previous, plan.axis) {
            (Some(previous), Axis::Row) => scroll_delta(previous, candidate, true),
            (Some(previous), Axis::Column) => scroll_delta(previous, candidate, false),
            (None, _) => 0,
        };
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.blit = Some(DiagBlit {
            axis: plan.axis,
            delta,
            src: plan.shift.src,
            dst: plan.shift.dst,
            // `None` until the execute arm actually reaches `push_clip`;
            // fallback and held attempts never apply a clip.
            clip: None,
            strip: plan.pixel_strip,
            revealed: Vec::new(),
            result,
            cold_cache,
        });
    }

    pub(crate) fn diag_blit_revealed(&self, region: PaneRegion, range: RCRange) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        if let Some(blit) = &mut capture.blit {
            blit.revealed.push(DiagRevealedStrip { region, range });
        }
    }

    /// The exact pixel rectangle handed to `Painter::push_clip` for strip
    /// painting — the effective grid clip, recorded at the call site.
    pub(crate) fn diag_blit_clip(&self, clip: PixelRect) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        if let Some(blit) = &mut capture.blit {
            blit.clip = Some(clip);
        }
    }

    pub(crate) fn diag_paint_counts(&self, rows: usize, cells: usize) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.paint_counts.rows += rows;
        capture.paint_counts.cells += cells;
    }

    /// Everything the `PreparedGrid::Full` execute arm reports: verdict,
    /// fingerprint reason, changed addresses, the installed fingerprint, and
    /// the painted counts. The arm calls this once instead of carrying its
    /// capture inline, so the production path reads as paint-then-commit.
    ///
    /// Every writer reached here owns disjoint capture fields, so nothing in
    /// the arm depends on the order of these calls.
    pub(crate) fn diag_commit_replace(&self, repaint: &PreparedRepaint, layout: GridLayout) {
        if !self.diag.enabled.get() {
            return;
        }
        self.diag_repaint(
            GridVerdict::from(&repaint.plan),
            repaint.reason,
            &repaint.changed_rows,
            &repaint.changed_cells,
        );
        self.diag_fingerprint_action(DiagFingerprintActionTag::Install);
        match &repaint.plan {
            PreparedRepaintPlan::Skip => self.diag_paint_counts(0, 0),
            PreparedRepaintPlan::Full => {
                self.diag_paint_ranges(layout.segments().map(|segment| segment.range()));
            }
            PreparedRepaintPlan::Rows(spans) => {
                // Absolute row intervals per painted segment, merged so `rows`
                // counts distinct grid rows even when frozen columns visit the
                // same rows in the left and right segments. Cells stay disjoint
                // across segments.
                let mut row_intervals: Vec<(i32, i32)> = Vec::new();
                let mut cells = 0usize;
                for grid_segment in layout.segments() {
                    let range = grid_segment.range();
                    let cols = (range.c2 - range.c1 + 1).max(0) as usize;
                    for span in spans {
                        let r1 = span.start().max(range.r1);
                        let r2 = span.end().min(range.r2);
                        if r1 <= r2 {
                            row_intervals.push((r1, r2));
                            cells += (r2 - r1 + 1) as usize * cols;
                        }
                    }
                }
                self.diag_paint_counts(distinct_rows(&row_intervals), cells);
            }
            // Envelope plans record their own counts, inside
            // `paint_repaint_envelope`.
            PreparedRepaintPlan::Cell { .. } | PreparedRepaintPlan::Range { .. } => {}
        }
    }

    /// Everything the `PreparedGrid::Damage` execute arm reports: a strip
    /// verdict with no fingerprint comparison, a stale-marked fingerprint, and
    /// the painted strips' counts.
    pub(crate) fn diag_commit_splice(&self, strips: &[PreparedStrip]) {
        if !self.diag.enabled.get() {
            return;
        }
        self.diag_repaint(GridVerdict::Strip, None, &[], &[]);
        self.diag_fingerprint_action(DiagFingerprintActionTag::MarkStale);
        self.diag_paint_ranges(strips.iter().map(|strip| strip.range));
    }

    /// Everything the `PreparedGrid::Blit` execute arm reports after painting:
    /// the revealed strips' counts and the fingerprint update the commit
    /// carries. The blit clip is NOT here — it is recorded at its `push_clip`
    /// call site, which runs before the paint loop.
    pub(crate) fn diag_commit_shift(
        &self,
        address_strips: &[Option<PreparedStrip>; 2],
        fingerprint: &PreparedFingerprintUpdate,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        self.diag_repaint(GridVerdict::Strip, None, &[], &[]);
        self.diag_fingerprint_action(match fingerprint {
            PreparedFingerprintUpdate::Install(_) => DiagFingerprintActionTag::Install,
            PreparedFingerprintUpdate::MarkStale => DiagFingerprintActionTag::MarkStale,
        });
        self.diag_paint_ranges(address_strips.iter().flatten().map(|strip| strip.range));
    }

    /// Painted row/cell counts for one execute arm. `ranges` yields the
    /// absolute address ranges that arm painted: rows count distinct, because
    /// frozen columns revisit the same absolute rows in the left and right
    /// segments, while cells add up, because a cell belongs to one segment
    /// only.
    fn diag_paint_ranges(&self, ranges: impl Iterator<Item = RCRange>) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut row_intervals: Vec<(i32, i32)> = Vec::new();
        let mut cells = 0usize;
        for range in ranges {
            row_intervals.push((range.r1, range.r2));
            cells += FetchedCells::addressed_cells(range);
        }
        self.diag_paint_counts(distinct_rows(&row_intervals), cells);
    }
}

/// Number of distinct absolute grid rows covered by the given inclusive
/// `(r1, r2)` intervals. Intervals from different segments overlap when
/// frozen columns split one band into left and right halves; merging them
/// keeps `DiagPaintCounts.rows` a unique-row count. Intervals are assumed
/// valid (`r1 <= r2`), as constructed by the execute arms.
pub(crate) fn distinct_rows(intervals: &[(i32, i32)]) -> usize {
    let mut sorted: Vec<(i32, i32)> = intervals.to_vec();
    sorted.sort_unstable_by_key(|(r1, _)| *r1);
    let mut rows = 0usize;
    let mut current: Option<(i32, i32)> = None;
    for (r1, r2) in sorted {
        match current {
            Some((c1, c2)) if r1 <= c2 + 1 => {
                current = Some((c1, c2.max(r2)));
            }
            Some((c1, c2)) => {
                rows += (c2 - c1 + 1) as usize;
                current = Some((r1, r2));
            }
            None => current = Some((r1, r2)),
        }
    }
    if let Some((c1, c2)) = current {
        rows += (c2 - c1 + 1) as usize;
    }
    rows
}

/// Logical origin delta of the scroll-band BottomRight segment along the
/// given axis (`true` = rows). Zero when either side lacks the segment —
/// the blit result tag still carries the real fallback cause.
fn scroll_delta(previous: GridLayout, candidate: GridLayout, rows: bool) -> i32 {
    let origin = |layout: GridLayout| {
        layout.segment(PaneRegion::BottomRight).map(|segment| {
            let range = segment.range();
            if rows { range.r1 } else { range.c1 }
        })
    };
    match (origin(previous), origin(candidate)) {
        (Some(before), Some(after)) => after - before,
        _ => 0,
    }
}
