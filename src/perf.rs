//! Commit->render performance measurement.
//!
//! Provides a `PerfTimings` context that records timestamps at each stage
//! of the cell commit pipeline. The `PerfPanel` component reads these
//! signals and displays the breakdown.
//!
//! Timestamps come from `performance.now()` (sub-millisecond resolution).

use leptos::prelude::*;

/// Shared timing signals, provided as Leptos context.
///
/// Written by `execute()` (commit phases) and the worksheet render `Effect`.
/// Read by `PerfPanel` to display the breakdown.
#[derive(Clone, Copy)]
pub struct PerfTimings {
    /// `performance.now()` just before `set_user_input()`.
    pub commit_start: RwSignal<Option<f64>>,
    /// `performance.now()` just after `set_user_input()`.
    pub input_done: RwSignal<Option<f64>>,
    /// `performance.now()` just after `evaluate()`.
    pub eval_done: RwSignal<Option<f64>>,
    /// Duration of the last `paintIfDirty()` call in milliseconds — measured
    /// inside the rAF loop only on frames that actually rendered (the loop
    /// is demand-driven and only runs when poked). Independent of the
    /// commit pipeline: scroll-only or overlay-only repaints update this too.
    pub render_ms: RwSignal<Option<f64>>,
    /// The formula/text that was committed (for display).
    pub last_formula: RwSignal<Option<String>>,
    /// One-line paint attribution for the last frame, straight from
    /// `IronCanvas.frameTrace()`: regime + per-pane verdict + cells fetched.
    /// Only sampled while the panel is open — reading it costs a wasm call
    /// per frame, and an instrument that runs when nobody is watching taxes
    /// the timings it exists to explain.
    pub frame_trace: RwSignal<Option<String>>,
    /// Authoritative canvas capture state, mirrored by the worksheet's
    /// diagnostics Effect. The rAF loop reads it (untracked) to decide
    /// whether to sample `frameDiagnostics()`. Dev-tools only — the design
    /// promise is that production builds retain no diagnostic state.
    #[cfg(feature = "dev-tools")]
    pub diag_enabled: RwSignal<bool>,
    /// JSON string of the last captured `IronCanvas.frameDiagnostics()`.
    /// `None` until capture is enabled and a painted frame completes.
    /// Dev-tools only.
    #[cfg(feature = "dev-tools")]
    pub frame_diagnostics: RwSignal<Option<String>>,
}

impl PerfTimings {
    pub fn new() -> Self {
        // Phase timestamps and render_ms are seeded to `Some(0.0)` so the
        // PerfPanel renders real-looking zeros from first paint instead of
        // a "commit a cell to measure" placeholder. Real values overwrite
        // these on the first commit / paint cycle.
        Self {
            commit_start: RwSignal::new(Some(0.0)),
            input_done: RwSignal::new(Some(0.0)),
            eval_done: RwSignal::new(Some(0.0)),
            render_ms: RwSignal::new(Some(0.0)),
            last_formula: RwSignal::new(None),
            frame_trace: RwSignal::new(None),
            #[cfg(feature = "dev-tools")]
            diag_enabled: RwSignal::new(false),
            #[cfg(feature = "dev-tools")]
            frame_diagnostics: RwSignal::new(None),
        }
    }
}

impl Default for PerfTimings {
    fn default() -> Self {
        Self::new()
    }
}

/// Read `performance.now()` from the browser.
pub fn now() -> f64 {
    leptos::prelude::window()
        .performance()
        .map(|p| p.now())
        .unwrap_or(0.0)
}
