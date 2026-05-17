//! Runtime tag for `Chrome` — which `FramePath` arm of `Chrome::next`
//! produced this frame. Diagnostics and per-pane fingerprint gating
//! read it; orchestrator `paint_*` arms dispatch on `PaintRegime`
//! upstream so they never need to match on this tag.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameKindTag {
    /// `FramePath::Fresh`: slot vecs freshly walked from the model
    /// through `Chrome::build`. First paint, or structural divergence.
    Fresh,
    /// `FramePath::SlotsReuse`: prev's slot vecs reused as-is; only
    /// per-frame state (theme, pane_fingerprints rotation) refreshed.
    SlotsReused,
    /// `FramePath::Blit(&plan)`: scroll-axis slot vec rebuilt around
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
