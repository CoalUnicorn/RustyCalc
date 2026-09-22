//! Pure digest of one capture under an explicit filter.
//!
//! The adapter reads the immutable [`CaptureRecord`] only: no signal, no
//! canvas, no clock. The caller supplies `observed_at_ms`, so two calls with
//! the same input return the same value.
//!
//! Two rules keep the numbers honest. First, every count stays separate —
//! grid paints, overlay-only paints, and skips overlap, so they are never
//! added into one total. Second, a missing duration is not zero: the render
//! statistics report their `measured` count beside the total, median, and
//! maximum, and `paint_counts.cells` is summed under the label
//! "addressed-cell visits" because it is not a unique cell count.

use super::capture::{AttemptOrigin, CaptureRecord, StopReason};
use super::{EvaluationOutcome, MutationOutcome};
use iron_canvas_core::{DiagCacheResolution, FrameOutcome, GridVerdict, RenderStrategy};

/// Which attempt records the digest measures.
///
/// The default removes forced-baseline attempts. `outcome` and `effective`
/// narrow the scope further; both are optional.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DigestFilter {
    /// Include forced-baseline attempts. The default removes them and counts
    /// the removals in [`CaptureDigest::forced_excluded`].
    pub include_forced_baseline: bool,
    /// Keep only attempts whose frame outcome is this value.
    pub outcome: Option<FrameOutcome>,
    /// Keep only attempts whose effective strategy is this value.
    pub effective: Option<RenderStrategy>,
}

/// Statistics over the attempts with a measured render call.
///
/// `median_ms` and `max_ms` are `None` when `measured` is zero. A duration
/// that was never sampled is not a zero duration.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderStats {
    pub measured: usize,
    pub total_ms: f64,
    pub median_ms: Option<f64>,
    pub max_ms: Option<f64>,
}

/// Renderer-owned fetch totals, summed over the scoped attempts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FetchTotals {
    pub addressed_cells: usize,
    pub logical_slots: usize,
    pub batches: usize,
}

/// The digest of one capture.
///
/// The categories overlap, so no field is a sum of another. `render_time_ms`
/// is the sum of the measured render-call samples, kept separate from
/// `wall_ms`.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureDigest {
    pub attempts: usize,
    pub committed: usize,
    pub held: usize,
    pub grid_paints: usize,
    pub overlay_only_paints: usize,
    pub skips: usize,
    pub fallbacks: usize,
    pub forced_excluded: usize,
    pub render: RenderStats,
    pub fetch: FetchTotals,
    /// Summed `paint_counts.cells`. This is addressed-cell visits, not a
    /// unique cell count.
    pub painted_cell_visits: usize,
    pub mutation_samples: usize,
    pub mutation_ok: usize,
    pub mutation_err: usize,
    pub evaluation_measured: usize,
    pub evaluation_deferred: usize,
    pub evaluation_not_run: usize,
    pub wall_ms: f64,
    pub active_ms: f64,
    pub render_time_ms: f64,
    pub truncated: bool,
    pub stop_reason: Option<StopReason>,
}

/// Compute the digest of `capture` under `filter`.
///
/// `observed_at_ms` closes `wall_ms` and `active_ms` for a capture that has
/// not finished. It is an argument, never a clock read, so the adapter stays
/// pure and a test can pin the value.
pub fn digest(
    capture: &CaptureRecord,
    filter: &DigestFilter,
    observed_at_ms: f64,
) -> CaptureDigest {
    let mut out = CaptureDigest {
        attempts: 0,
        committed: 0,
        held: 0,
        grid_paints: 0,
        overlay_only_paints: 0,
        skips: 0,
        fallbacks: 0,
        forced_excluded: 0,
        render: RenderStats::default(),
        fetch: FetchTotals::default(),
        painted_cell_visits: 0,
        mutation_samples: 0,
        mutation_ok: 0,
        mutation_err: 0,
        evaluation_measured: 0,
        evaluation_deferred: 0,
        evaluation_not_run: 0,
        wall_ms: 0.0,
        active_ms: 0.0,
        render_time_ms: 0.0,
        truncated: capture.truncated,
        stop_reason: capture.stop_reason,
    };

    let mut samples: Vec<f64> = Vec::new();
    for record in &capture.attempts {
        if !filter.include_forced_baseline && record.origin == AttemptOrigin::ForcedBaseline {
            out.forced_excluded += 1;
            continue;
        }
        if filter
            .effective
            .is_some_and(|effective| record.diagnostics.effective != Some(effective))
        {
            continue;
        }
        if filter
            .outcome
            .is_some_and(|outcome| record.diagnostics.outcome != outcome)
        {
            continue;
        }

        let diag = &record.diagnostics;
        out.attempts += 1;
        if diag.committed_seq.is_some() {
            out.committed += 1;
        }
        if diag.cache.resolution == DiagCacheResolution::HeldForRetry {
            out.held += 1;
        }
        if diag.painted_layers.grid {
            out.grid_paints += 1;
        }
        if diag.painted_layers.overlay && !diag.painted_layers.grid {
            out.overlay_only_paints += 1;
        }
        if diag.repaint.verdict == Some(GridVerdict::Skip) {
            out.skips += 1;
        }
        if diag.selected.is_some() && diag.effective.is_some() && diag.selected != diag.effective {
            out.fallbacks += 1;
        }
        out.painted_cell_visits += diag.paint_counts.cells;
        out.fetch.addressed_cells += diag.fetch.addressed_cells;
        out.fetch.logical_slots += diag.fetch.logical_slots;
        out.fetch.batches += diag.fetch.batches;

        if let Some(ms) = record.render_call_ms {
            out.render.measured += 1;
            out.render.total_ms += ms;
            out.render_time_ms += ms;
            samples.push(ms);
        }
    }

    if !samples.is_empty() {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = samples.len() / 2;
        let median = if samples.len() % 2 == 1 {
            samples[mid]
        } else {
            (samples[mid - 1] + samples[mid]) / 2.0
        };
        out.render.median_ms = Some(median);
        out.render.max_ms = samples.last().copied();
    }

    // A mutation sample is counted once, whatever number of attempts follow
    // it. Mutation success and evaluation state are independent: an errored
    // mutation is `mutation_err` with `evaluation_not_run`.
    for sample in &capture.mutations {
        out.mutation_samples += 1;
        match sample.outcome {
            MutationOutcome::Ok => out.mutation_ok += 1,
            MutationOutcome::Err => out.mutation_err += 1,
        }
        match sample.evaluation {
            EvaluationOutcome::Measured { .. } => out.evaluation_measured += 1,
            EvaluationOutcome::Deferred => out.evaluation_deferred += 1,
            EvaluationOutcome::NotRun => out.evaluation_not_run += 1,
        }
    }

    out.wall_ms = capture.wall_ms(observed_at_ms);
    let closed_pauses: f64 = capture.paused.iter().map(|(from, to)| to - from).sum();
    let open_pause = capture
        .pause_started_at_ms
        .map(|started| observed_at_ms - started)
        .unwrap_or(0.0);
    out.active_ms = out.wall_ms - closed_pauses - open_pause;

    out
}
