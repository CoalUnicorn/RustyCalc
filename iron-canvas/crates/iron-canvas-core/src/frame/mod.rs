//! One paint attempt, start to finish: queued intent, captured scalar
//! inputs, the classification delta, the plan, and the report.
//!
//! The submodules follow the attempt in order. `work` holds the typed intent
//! the setters mark and `mem::take` drains; `inputs` snapshots the scalars;
//! `delta` classifies the snapshot against the committed `Chrome`; `plan`
//! turns intent plus delta into a `FramePlan`; `report` names what the attempt
//! did. See the stage-to-directory map in the module hierarchy design.

pub(crate) mod delta;
pub(crate) mod inputs;
pub(crate) mod plan;
pub(crate) mod report;
pub(crate) mod work;

#[cfg(test)]
mod plan_tests;

pub(crate) use delta::AxisRange;
pub use delta::{BlitPlan, FrameDelta, RebuildReason, Shift};
pub use inputs::{FrameInputFailure, FrameInputs};
pub use plan::RenderStrategy;
pub(crate) use plan::{GridWork, OverlayWork, plan_frame};
pub use report::{BlitFallback, FrameOutcome, FrameTrace, GridVerdict, PaintResult};
pub use work::{RowSpan, WorkFlags};
