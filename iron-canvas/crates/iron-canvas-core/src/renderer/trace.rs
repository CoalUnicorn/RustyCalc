use crate::address::RCRange;
use crate::frame::{BlitFallback, FrameOutcome, FrameTrace, GridVerdict};
use crate::painter::Painter;
use crate::renderer::prepared::FetchedCells;

use super::RendererCore;

impl<P: Painter> RendererCore<P> {
    /// Clear the trace for a new frame. Called once by `render_pending`
    /// before dispatch, never by a paint method — a paint method that reset
    /// it would erase attribution already recorded by the current attempt.
    pub fn reset_trace(&self) {
        self.trace.set(FrameTrace::default());
    }

    pub fn trace(&self) -> FrameTrace {
        self.trace.get()
    }

    #[cfg(feature = "surface-introspection")]
    pub fn strip_scratch_capacities(&self) -> Vec<(usize, usize, usize, usize)> {
        self.frame_cache
            .strip_scratch
            .borrow()
            .iter()
            .map(FetchedCells::capacities)
            .collect()
    }

    pub(super) fn trace_grid(&self, verdict: GridVerdict) {
        let mut t = self.trace.get();
        t.verdict = Some(verdict);
        self.trace.set(t);
    }

    /// Record why a `ScrollBlit` frame lost the grid-wide strip path.
    pub(super) fn trace_blit_fallback(&self, cold_cache: bool) {
        let mut t = self.trace.get();
        if t.blit_fallback.is_none() {
            t.blit_fallback = Some(BlitFallback { cold_cache });
        }
        self.trace.set(t);
    }

    pub(super) fn trace_frame_held(&self) {
        let mut t = self.trace.get();
        t.verdict = Some(GridVerdict::Held);
        t.outcome = FrameOutcome::HeldOnBridgeFailure;
        self.trace.set(t);
        // The structured snapshot records the SAME final grid verdict at
        // this decision site, so a held frame never publishes a null
        // `repaint.verdict` while the one-line trace says `grid:held`.
        // Capture-failure holds never reach here and keep `verdict: None`
        // — the grid genuinely never decided for them.
        #[cfg(feature = "dev-diagnostics")]
        self.diag_repaint(GridVerdict::Held, None, &[], &[]);
    }

    /// Charge one renderer bundle fetch over `range`. The legacy logical slot
    /// total remains a derived channel count for compatibility; the separate
    /// cell and batch counters make the trace useful without pretending to
    /// know how many host or adapter calls the model performed internally.
    pub(super) fn trace_fetch(&self, range: RCRange) {
        let mut t = self.trace.get();
        t.fetched_cell_slots += FetchedCells::logical_channel_slots(range);
        t.fetched_cells += FetchedCells::addressed_cells(range);
        t.fetch_batches += 1;
        self.trace.set(t);
    }
}
