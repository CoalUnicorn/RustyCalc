//! Commit→render performance measurement.
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
    /// `performance.now()` just after canvas `render()`.
    pub render_done: RwSignal<Option<f64>>,
    /// The formula/text that was committed (for display).
    pub last_formula: RwSignal<Option<String>>,
}

impl PerfTimings {
    pub fn new() -> Self {
        Self {
            commit_start: RwSignal::new(None),
            input_done: RwSignal::new(None),
            eval_done: RwSignal::new(None),
            render_done: RwSignal::new(None),
            last_formula: RwSignal::new(None),
        }
    }
}

/// Read `performance.now()` from the browser.
pub fn now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}
