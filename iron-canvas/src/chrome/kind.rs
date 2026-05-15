//! Runtime tag for `Chrome` — which constructor produced this frame.
//! Regime arms in `orchestrator::paint_*` will match exhaustively on this
//! tag (Stage 5); adding a variant breaks every dispatch site at compile
//! time. Replaces a bool that could only distinguish two of the three
//! constructor regimes.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameKindTag {
    /// `Chrome::next_frame` with `prev = None` (or a structural-cause prev).
    /// Slot vecs are freshly walked from the model.
    Fresh,
    /// `Chrome::from_slots_reuse`: prev's slot vecs reused as-is, only
    /// per-frame state (theme, pane_fingerprints rotation) refreshed.
    SlotsReused,
    /// `Chrome::next_frame_with_blit`: scroll-axis slot vec rebuilt around
    /// a `BlitPlan`; cross-axis slot vec reused from prev.
    Blitted,
}

impl FrameKindTag {
    /// True when slot vecs along at least one axis were inherited from
    /// prev. `Fresh` is the only kind that does not reuse; both
    /// `SlotsReused` and `Blitted` carry forward prev's pane caches.
    pub(crate) fn reuses_slots(self) -> bool {
        matches!(self, Self::SlotsReused | Self::Blitted)
    }
}
