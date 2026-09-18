//! Typed timing samples for the Perf panel and the capture store.
//!
//! Every sample answers one question about one call and carries its own
//! measured duration. A fact that did not happen is named, never encoded as
//! a zero.

use leptos::prelude::*;
use serde::Serialize;

/// Result of the model closure.
///
/// Independent of [`EvaluationOutcome`]: a mutation can fail and evaluation
/// can still be pending, deferred, or never run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MutationOutcome {
    Ok,
    Err,
}

/// What `evaluate()` did for one mutation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EvaluationOutcome {
    /// No evaluation belongs to this mutation. The closure failed, or the
    /// call was skipped for another reason.
    NotRun,
    /// The mutation ran in deferred mode and the caller evaluates later,
    /// off this sample. No duration is available yet.
    Deferred,
    /// `evaluate()` ran inside this mutation. `ms` is its duration.
    Measured { ms: f64 },
}

impl EvaluationOutcome {
    /// Map one wrapper call to its evaluation fact.
    ///
    /// `apply_ok` is false when the closure returned an error. Evaluation is
    /// skipped in both modes in that case, so the result is `NotRun` — never
    /// `Deferred`. `deferred` is true for `EvaluationMode::Deferred`.
    /// `eval_ms` is the measured `evaluate()` duration and is read only when
    /// evaluation ran.
    pub fn of_call(apply_ok: bool, deferred: bool, eval_ms: f64) -> Self {
        match (apply_ok, deferred) {
            (false, _) => Self::NotRun,
            (true, true) => Self::Deferred,
            (true, false) => Self::Measured { ms: eval_ms },
        }
    }
}

/// One completed model mutation: one `mutate` or `try_mutate` call.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationSample {
    /// Monotonic per-session identifier, assigned by [`PerfTimings`].
    /// The first mutation of a session is 1.
    pub seq: u64,
    /// `performance.now()` when the wrapper started.
    pub started_at_ms: f64,
    /// Duration of the model closure only. Excludes evaluation.
    pub apply_ms: f64,
    pub outcome: MutationOutcome,
    pub evaluation: EvaluationOutcome,
}

/// One `render_pending()` call that painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSample {
    /// `performance.now()` immediately before the call.
    pub started_at_ms: f64,
    /// Duration of the call, measured around it.
    pub ms: f64,
}

/// Callback the capture store installs to see every mutation sample.
///
/// The panel reads the `mutation` signal, which keeps only the last sample.
/// The store needs each one — two mutations inside a single task both belong
/// in the capture — so `publish_mutation` also hands the sample to this sink.
/// Dev-tools only, with the store that installs it.
#[cfg(feature = "dev-tools")]
pub type SampleSink = std::rc::Rc<dyn Fn(&MutationSample)>;

/// Shared timing signals, provided as Leptos context.
///
/// Written by the model wrappers and the worksheet render loop. Read by
/// `PerfPanel`. `Copy` so `AppState` stays `Copy`.
#[derive(Clone, Copy)]
pub struct PerfTimings {
    /// Last completed mutation, or `None` before the first one.
    pub mutation: RwSignal<Option<MutationSample>>,
    /// Sequence source for [`MutationSample::seq`]. Private: only
    /// `publish_mutation` assigns it.
    mutation_seq: RwSignal<u64>,
    /// Duration of the last `render_pending()` call that painted, measured
    /// inside the rAF loop. Independent of the model wrappers: a scroll-only
    /// or overlay-only repaint updates this and publishes no mutation.
    pub render_call: RwSignal<Option<RenderSample>>,
    /// One-line paint attribution for the last frame, straight from
    /// `IronCanvas.frameTrace()`: strategy + per-pane verdict + cells fetched.
    /// Only sampled while the panel is open — reading it costs a wasm call
    /// per frame, and an instrument that runs when nobody is watching taxes
    /// the timings it exists to explain.
    pub frame_trace: RwSignal<Option<String>>,
    /// Capture store's sample sink. Non-reactive on purpose: installing it
    /// must not re-run the panel, and it is not a rendering input.
    #[cfg(feature = "dev-tools")]
    sample_sink: StoredValue<Option<SampleSink>, LocalStorage>,
}

impl PerfTimings {
    pub fn new() -> Self {
        // Every sample starts empty. The panel shows a placeholder until a
        // real call produces a real duration, which keeps "not measured yet"
        // distinguishable from "measured zero".
        Self {
            mutation: RwSignal::new(None),
            mutation_seq: RwSignal::new(0),
            render_call: RwSignal::new(None),
            frame_trace: RwSignal::new(None),
            #[cfg(feature = "dev-tools")]
            sample_sink: StoredValue::new_local(None),
        }
    }

    /// Publish one completed mutation.
    ///
    /// Assigns the sequence, writes the signal in one step, and calls the
    /// capture sink when one is installed. One call per wrapper call, so two
    /// mutations inside one task both reach the store.
    pub fn publish_mutation(
        &self,
        started_at_ms: f64,
        apply_ms: f64,
        outcome: MutationOutcome,
        evaluation: EvaluationOutcome,
    ) {
        let seq = self.mutation_seq.get_untracked().wrapping_add(1);
        self.mutation_seq.set(seq);
        let sample = MutationSample {
            seq,
            started_at_ms,
            apply_ms,
            outcome,
            evaluation,
        };
        self.mutation.set(Some(sample));
        #[cfg(feature = "dev-tools")]
        if let Some(sink) = self.sample_sink.get_value() {
            // Release the StoredValue borrow before calling host code. The
            // sink can remove itself when the capture reaches a limit.
            sink(&sample);
        }
    }

    /// Install the capture store's sample sink. `None` removes it.
    #[cfg(feature = "dev-tools")]
    pub fn set_sample_sink(&self, sink: Option<SampleSink>) {
        self.sample_sink.set_value(sink);
    }
}

impl Default for PerfTimings {
    fn default() -> Self {
        Self::new()
    }
}
