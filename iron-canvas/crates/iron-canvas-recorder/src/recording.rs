//! `.icr` (iron-canvas-recording) format — single-document JSON artifact
//! capturing a paint-level recording for bug-repro / dev tooling.
//!
//! This module is the single source of truth for the on-disk schema.
//! Any change to a field name, variant tag, or layout is a format change
//! — bump `ICR_SCHEMA_VERSION` and regenerate the golden fixture
//! (`tests/fixtures/fresh_paint.icr` via `ICR_REGEN=1 cargo test
//! -p iron-canvas-recorder --test golden_fixture`).
//!
//! # On-disk layout (v7)
//!
//! UTF-8 bytes. One JSON object — a `Recording` with `header` and
//! `frames` fields. Standard JSON, so `jq .` and any JSON validator
//! reads it without special-casing:
//!
//! ```text
//! {"header":{"schema_version":7,"iron_canvas_version":"0.1.0-alpha.1",...},
//!  "frames":[
//!    {"frame_idx":0,"t_ms":0,"origin":"forced_baseline",...},
//!    {"frame_idx":1,"t_ms":17,"origin":"live",...}
//!  ]}
//! ```
//!
//! There is no compression at this layer — deferred to a later phase.
//!
//! # Header (`IcrHeader`)
//!
//! | Field                 | Type            | Meaning                                                              |
//! | --------------------- | --------------- | -------------------------------------------------------------------- |
//! | `schema_version`      | `u32`           | Always `ICR_SCHEMA_VERSION` (currently `7`). Mismatch -> load fails.  |
//! | `iron_canvas_version` | `String`        | `env!("CARGO_PKG_VERSION")` at serialize time. Mismatch -> warn-only. |
//! | `canvas_w` / `canvas_h` | `f64`         | Canvas dimensions at recording start. The viewer auto-sizes to these.|
//! | `theme`               | `ThemeSnapshot` | Owned-string mirror of `CanvasTheme`'s 14 palette fields.            |
//! | `started_at_unix_ms`  | `u64`          | Wall-clock at `startRecording`. Host-supplied; tests pass `0`.       |
//! | `partial`             | `bool`          | `true` when the hard-cap watchdog (100 MB) auto-stopped capture.    |
//!
//! # Attempt (`Frame`)
//!
//! | Field           | Type                                  | Meaning                                                                                |
//! | --------------- | ------------------------------------- | -------------------------------------------------------------------------------------- |
//! | `frame_idx`     | `u32`                                 | Storage index; not a render-attempt identity.                                           |
//! | `t_ms`          | `u64`                                 | Milliseconds since `started_at_unix_ms`.                                               |
//! | `origin`        | `RecordOrigin`                        | Whether capture was requested by `startRecording` or a normal paint tick.              |
//! | `result`        | `RecordedPaintResult`                 | Scheduler result. Idle ticks are omitted; holds remain as zero-op retries.              |
//! | `trace`         | `TraceRecord`                         | Recorder-owned projection of the complete core trace.                                   |
//! | `grid_ops`      | `Vec<DrawOp>`                         | Ops captured from the grid surface for this frame. May be empty for `overlay`.        |
//! | `overlay_ops`   | `Vec<DrawOp>`                         | Ops captured from the overlay surface for this frame.                                  |
//!
//! Empty non-idle attempts are retained so bridge/input holds and retries are
//! visible in the diagnostic timeline.
//!
//! # Compatibility rules
//!
//! - `schema_version`: exact-match enforced by `deserialize()`. A
//!   reader for another schema version refuses the file.
//! - `iron_canvas_version`: divergence is a *warning* on load. The
//!   recording still plays. The viewer surfaces this as a banner since
//!   replay against drifted renderer output is the most common bug-repro
//!   gap.
//! - `DrawOp` variants: additive only within a schema version. Adding
//!   a new variant is a breaking change (older readers don't recognize
//!   it) — bump the schema.
//! - `trace`: the recorder projection is authoritative for strategy, work,
//!   outcome, fetch attribution, and attempt/commit identities. Painter ops
//!   remain layer-local and may be empty for a held or skipped attempt.
//!
//! # See also
//!
//! - Producer: `RecordingSurface` + `RecordingPainter` in `crate`.
//! - Viewer: `iron-canvas/web-test/recording-viewer.html` (standalone HTML).
//! - Regression sentinel: `tests/golden_fixture.rs` +
//!   `tests/fixtures/fresh_paint.icr`.

use serde::{Deserialize, Serialize};

use iron_canvas_core::geometry::{CanvasMetrics, CanvasSize};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{FrameOutcome, FrameTrace, GridVerdict, RenderStrategy};

use crate::DrawOp;

/// Bumped only on breaking changes to the on-disk shape (added fields
/// with defaults don't bump). The loader rejects mismatched versions.
pub const ICR_SCHEMA_VERSION: u32 = 7;

/// Why an attempt entered the recording timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordOrigin {
    ForcedBaseline,
    Live,
}

/// Scheduler result for a non-idle core attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedPaintResult {
    Painted,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TraceVerdict {
    Skip,
    Cell,
    Range,
    Rows { spans: u8, rows: u16 },
    Full,
    Strip,
    Held,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TraceOutcome {
    Painted,
    HeldOnBridgeFailure,
    HeldOnInputFailure { failure: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBlitFallback {
    pub cold_cache: bool,
}

/// Stable wire representation of the allocation-free core `FrameTrace`.
/// Serde and schema concerns stay in the recorder crate rather than in core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRecord {
    pub attempt_seq: u64,
    pub committed_seq: Option<u64>,
    pub strategy: Option<RenderStrategy>,
    pub effective: Option<RenderStrategy>,
    pub work: u8,
    pub verdict: Option<TraceVerdict>,
    pub outcome: TraceOutcome,
    pub blit_fallback: Option<TraceBlitFallback>,
    pub fetched_cell_slots: usize,
    pub fetched_cells: usize,
    pub fetch_batches: usize,
}

impl From<FrameTrace> for TraceRecord {
    fn from(trace: FrameTrace) -> Self {
        let verdict = trace.verdict.map(|verdict| match verdict {
            GridVerdict::Skip => TraceVerdict::Skip,
            GridVerdict::Cell => TraceVerdict::Cell,
            GridVerdict::Range => TraceVerdict::Range,
            GridVerdict::Rows { spans, rows } => TraceVerdict::Rows { spans, rows },
            GridVerdict::Full => TraceVerdict::Full,
            GridVerdict::Strip => TraceVerdict::Strip,
            GridVerdict::Held => TraceVerdict::Held,
        });
        let outcome = match trace.outcome {
            FrameOutcome::Painted => TraceOutcome::Painted,
            FrameOutcome::HeldOnBridgeFailure => TraceOutcome::HeldOnBridgeFailure,
            FrameOutcome::HeldOnInputFailure(failure) => TraceOutcome::HeldOnInputFailure {
                failure: match failure {
                    iron_canvas_core::FrameInputFailure::SelectedSheet => 0,
                    iron_canvas_core::FrameInputFailure::SelectedView => 1,
                    iron_canvas_core::FrameInputFailure::SheetMismatch => 2,
                    iron_canvas_core::FrameInputFailure::FrozenRows => 3,
                    iron_canvas_core::FrameInputFailure::FrozenColumns => 4,
                    iron_canvas_core::FrameInputFailure::RowHeaderVisibility => 5,
                    iron_canvas_core::FrameInputFailure::ColumnHeaderVisibility => 6,
                    // Codes are stable wire values: new variants append.
                    iron_canvas_core::FrameInputFailure::InvalidFrozenRowCount => 7,
                    iron_canvas_core::FrameInputFailure::InvalidFrozenColumnCount => 8,
                },
            },
        };
        let blit_fallback = trace.blit_fallback.map(|fallback| TraceBlitFallback {
            cold_cache: fallback.cold_cache,
        });
        Self {
            attempt_seq: trace.attempt_seq,
            committed_seq: trace.committed_seq,
            strategy: trace.strategy,
            effective: trace.effective,
            work: trace.work.bits(),
            verdict,
            outcome,
            blit_fallback,
            fetched_cell_slots: trace.fetched_cell_slots,
            fetched_cells: trace.fetched_cells,
            fetch_batches: trace.fetch_batches,
        }
    }
}

/// Per-attempt capture. `trace` is authoritative for strategy, work, outcome,
/// and identities; the storage index is only an array position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub frame_idx: u32,
    /// Milliseconds since `IcrHeader::started_at_unix_ms`.
    pub t_ms: u64,
    pub origin: RecordOrigin,
    pub result: RecordedPaintResult,
    pub trace: TraceRecord,
    pub grid_ops: Vec<DrawOp>,
    pub overlay_ops: Vec<DrawOp>,
}

impl Frame {
    /// A committed Fresh paint can replace all prior grid pixels.
    /// Use the effective strategy: a ScrollBlit can fall back to FullRebuild.
    pub fn is_replay_anchor(&self) -> bool {
        self.result == RecordedPaintResult::Painted
            && self.trace.outcome == TraceOutcome::Painted
            && self.trace.effective == Some(RenderStrategy::FullRebuild)
            && self.trace.committed_seq.is_some()
    }
}

/// A recording that passed the playback preconditions.
///
/// [`Recording`] is raw wire data: [`Recording::deserialize`] checks the schema
/// version and nothing else, because a file may legitimately be inspected or
/// shown by a viewer that paints nothing. Playback is different — it drives the
/// live painter and the live backing stores, so a recording whose canvas cannot
/// exist, whose timestamps run backwards, whose clip or group brackets are
/// unbalanced, or whose first grid ops have no committed anchor would push the
/// visible canvas into a state no later frame repairs.
///
/// This type is that parse. `ValidatedRecording::try_from(Recording)` is the only
/// constructor, so the web facade's playback session holds a checked value.
/// The low-level [`crate::replay`] API accepts raw operations separately.
/// This type is never serialized: the ICR
/// wire shape is unchanged, and this change only rejects inputs the loader
/// previously accepted.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedRecording {
    recording: Recording,
    metrics: CanvasMetrics,
}

impl ValidatedRecording {
    /// Validated canvas metrics from the header. Playback resizes the live
    /// canvas to these, so they are parsed once here rather than at each use.
    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics
    }

    /// The checked recording, for readers that need the header or frames.
    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    pub fn frames(&self) -> &[Frame] {
        &self.recording.frames
    }

    pub fn frame_count(&self) -> u32 {
        self.recording.frames.len() as u32
    }

    pub fn into_recording(self) -> Recording {
        self.recording
    }
}

impl TryFrom<Recording> for ValidatedRecording {
    type Error = IcrError;

    fn try_from(recording: Recording) -> Result<Self, Self::Error> {
        recording.validate_schema()?;
        let header = &recording.header;
        let metrics = CanvasMetrics::new(
            CanvasSize {
                w: header.canvas_w,
                h: header.canvas_h,
            },
            header.dpr,
        )
        .map_err(|error| IcrError::Format(format!("invalid recording canvas metrics: {error}")))?;
        if recording.frames.is_empty() {
            return Err(IcrError::Format("recording has no frames".to_string()));
        }
        validate_frames(&recording.frames)?;
        Ok(Self { recording, metrics })
    }
}

/// Rejections that protect live replay state. Each carries the offending frame
/// index so a viewer can point at it.
fn validate_frames(frames: &[Frame]) -> Result<(), IcrError> {
    let mut previous_t_ms: Option<u64> = None;
    let mut anchored = false;
    for (index, frame) in frames.iter().enumerate() {
        if let Some(previous) = previous_t_ms
            && frame.t_ms < previous
        {
            return Err(IcrError::Format(format!(
                "frame {index}: timestamp {} precedes the previous frame's {previous}",
                frame.t_ms,
            )));
        }
        previous_t_ms = Some(frame.t_ms);
        validate_ops(index, "grid", &frame.grid_ops)?;
        validate_ops(index, "overlay", &frame.overlay_ops)?;
        if frame.is_replay_anchor() {
            anchored = true;
        }
        if !frame.grid_ops.is_empty() && !anchored {
            return Err(IcrError::Format(format!(
                "frame {index}: grid ops precede any committed FullRebuild anchor, so a replay \
                 has no pixels to composite them onto",
            )));
        }
    }
    Ok(())
}

/// One frame's op list must be a balanced bracket sequence with finite numbers.
///
/// A frame is a whole paint attempt, so its clips and groups return to zero
/// inside it. Replay applies the ops directly to the live painter; an
/// unbalanced bracket would leave the visible canvas clipped or transformed for
/// every later frame, and a `NaN` coordinate would poison the painter's
/// transform for the same span.
fn validate_ops(frame_index: usize, channel: &str, ops: &[DrawOp]) -> Result<(), IcrError> {
    let mut clips: i32 = 0;
    let mut groups: i32 = 0;
    for op in ops {
        match op {
            DrawOp::PushClip { .. } => clips += 1,
            DrawOp::PopClip => {
                clips -= 1;
                if clips < 0 {
                    return Err(IcrError::Format(format!(
                        "frame {frame_index} {channel}: PopClip without a matching PushClip",
                    )));
                }
            }
            DrawOp::BeginGroup { .. } => groups += 1,
            DrawOp::EndGroup => {
                groups -= 1;
                if groups < 0 {
                    return Err(IcrError::Format(format!(
                        "frame {frame_index} {channel}: EndGroup without a matching BeginGroup",
                    )));
                }
            }
            _ => {}
        }
        if !draw_op_numbers_are_finite(op) {
            return Err(IcrError::Format(format!(
                "frame {frame_index} {channel}: draw op carries a non-finite coordinate or width",
            )));
        }
    }
    if clips != 0 {
        return Err(IcrError::Format(format!(
            "frame {frame_index} {channel}: {clips} clip(s) left open at the end of the frame",
        )));
    }
    if groups != 0 {
        return Err(IcrError::Format(format!(
            "frame {frame_index} {channel}: {groups} group(s) left open at the end of the frame",
        )));
    }
    Ok(())
}

/// Every `f64` a draw op can carry. Integer fields (`PixelRect`, `Point`,
/// `Span`, `Line`) are finite by their types.
fn draw_op_numbers_are_finite(op: &DrawOp) -> bool {
    match op {
        DrawOp::RectStroke { width, .. } | DrawOp::RectDashed { width, .. } => width.is_finite(),
        DrawOp::StrokeLine { width, .. } => width.is_finite(),
        DrawOp::StrokeHLine { y, width, .. } => y.is_finite() && width.is_finite(),
        DrawOp::StrokeVLine { x, width, .. } => x.is_finite() && width.is_finite(),
        DrawOp::StrokeTextHLine {
            x1, x2, y, width, ..
        } => x1.is_finite() && x2.is_finite() && y.is_finite() && width.is_finite(),
        DrawOp::FillText { x, y, .. } => x.is_finite() && y.is_finite(),
        DrawOp::ApplyDprTransform { dpr } => dpr.is_finite() && *dpr > 0.0,
        DrawOp::RectFill { .. }
        | DrawOp::FillPath { .. }
        | DrawOp::ClearRect { .. }
        | DrawOp::PushClip { .. }
        | DrawOp::PopClip
        | DrawOp::InvalidateCache
        | DrawOp::ResetTextDefaults
        | DrawOp::BeginGroup { .. }
        | DrawOp::EndGroup
        | DrawOp::Blit { .. } => true,
    }
}

/// Flattened, owned-string mirror of `CanvasTheme`. Built `From<&CanvasTheme>`
/// so the engine type stays serde-free — its `Cow<'static, str>` fields
/// don't round-trip through `serde_json` (they'd deserialize as `Cow::Owned`,
/// breaking the ptr-eq fast path the renderer relies on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeSnapshot {
    pub grid_color: String,
    pub grid_separator_color: String,
    pub header_bg: String,
    pub header_border_color: String,
    pub header_text_color: String,
    pub header_selected_bg: String,
    pub header_selected_color: String,
    pub default_text_color: String,
    pub error_text_color: String,
    pub selection_color: String,
    pub cell_bg: String,
    pub pointing: String,
    pub selection_fill: String,
    pub pointing_tint: String,
}

impl From<&CanvasTheme> for ThemeSnapshot {
    fn from(t: &CanvasTheme) -> Self {
        Self {
            grid_color: t.grid_color.to_string(),
            grid_separator_color: t.grid_separator_color.to_string(),
            header_bg: t.header_bg.to_string(),
            header_border_color: t.header_border_color.to_string(),
            header_text_color: t.header_text_color.to_string(),
            header_selected_bg: t.header_selected_bg.to_string(),
            header_selected_color: t.header_selected_color.to_string(),
            default_text_color: t.default_text_color.to_string(),
            error_text_color: t.error_text_color.to_string(),
            selection_color: t.selection_color.to_string(),
            cell_bg: t.cell_bg.to_string(),
            pointing: t.pointing.to_string(),
            selection_fill: t.selection_fill.to_string(),
            pointing_tint: t.pointing_tint.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IcrHeader {
    pub schema_version: u32,
    /// `env!("CARGO_PKG_VERSION")` at serialization time. Used by the
    /// viewer to warn on cross-version playback.
    pub iron_canvas_version: String,
    /// Canvas dimensions at recording start. Inlined as bare `f64`s
    /// rather than `CanvasSize` to keep the engine `CanvasSize` serde-free.
    pub canvas_w: f64,
    pub canvas_h: f64,
    /// Device pixel ratio at recording start. Playback uses this to size
    /// the backing store and forward the right transform to the live
    /// painter without scanning frame 0 for the first
    /// `ApplyDprTransform`. Added in schema v2.
    pub dpr: f64,
    pub theme: ThemeSnapshot,
    /// Unix epoch milliseconds when `startRecording` fired. Host-supplied.
    pub started_at_unix_ms: u64,
    /// `true` when `stopRecording` was triggered by the hard-cap watchdog
    /// (Stage 3) rather than an explicit user call.
    pub partial: bool,
}

impl IcrHeader {
    pub fn new(
        canvas_w: f64,
        canvas_h: f64,
        dpr: f64,
        theme: ThemeSnapshot,
        started_at_unix_ms: u64,
    ) -> Self {
        Self {
            schema_version: ICR_SCHEMA_VERSION,
            iron_canvas_version: env!("CARGO_PKG_VERSION").to_string(),
            canvas_w,
            canvas_h,
            dpr,
            theme,
            started_at_unix_ms,
            partial: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub header: IcrHeader,
    pub frames: Vec<Frame>,
}

impl Recording {
    pub fn new(header: IcrHeader) -> Self {
        Self {
            header,
            frames: Vec::new(),
        }
    }

    pub fn push_frame(&mut self, frame: Frame) {
        self.frames.push(frame);
    }

    /// Encode the recording as one JSON document. `Recording` derives
    /// `Serialize`, so this is a single `to_writer` over `{header, frames}`.
    pub fn serialize(&self) -> Result<Vec<u8>, IcrError> {
        let mut out = Vec::new();
        serde_json::to_writer(&mut out, self)?;
        Ok(out)
    }

    /// Decode a single JSON document. Rejects on schema-version mismatch
    /// — the caller decides whether `iron_canvas_version` divergence
    /// is fatal.
    ///
    /// This is the *wire* check only. Playback must convert the result with
    /// [`ValidatedRecording::try_from`], which additionally rejects a
    /// recording that cannot drive the live painter (see that type).
    pub fn deserialize(bytes: &[u8]) -> Result<Self, IcrError> {
        let rec: Recording = serde_json::from_slice(bytes)?;
        rec.validate_schema()?;
        Ok(rec)
    }

    fn validate_schema(&self) -> Result<(), IcrError> {
        if self.header.schema_version != ICR_SCHEMA_VERSION {
            return Err(IcrError::Format(format!(
                "schema_version mismatch: file={}, reader={}",
                self.header.schema_version, ICR_SCHEMA_VERSION,
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum IcrError {
    Json(serde_json::Error),
    Format(String),
}

impl std::fmt::Display for IcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IcrError::Json(e) => write!(f, "json error: {}", e),
            IcrError::Format(s) => write!(f, "format error: {}", s),
        }
    }
}

impl std::error::Error for IcrError {}

impl From<serde_json::Error> for IcrError {
    fn from(e: serde_json::Error) -> Self {
        IcrError::Json(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::geometry::pixel_rect::PixelRect;
    use iron_canvas_core::geometry::prim::Point;
    use iron_canvas_core::painter::{TextAlign, TextBaseline};
    use iron_canvas_core::theme::CanvasTheme;

    use crate::GroupClass;

    fn header() -> IcrHeader {
        IcrHeader::new(
            800.0,
            400.0,
            1.0,
            ThemeSnapshot::from(&CanvasTheme::light()),
            0,
        )
    }

    fn pix(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
        PixelRect {
            top_left: Point { x, y },
            width: w,
            height: h,
        }
    }

    fn trace(strategy: RenderStrategy, work: u8) -> TraceRecord {
        let core_trace = FrameTrace {
            attempt_seq: 1,
            committed_seq: Some(1),
            strategy: Some(strategy),
            effective: Some(strategy),
            work: iron_canvas_core::WorkFlags::from_bits_retain(work),
            ..FrameTrace::default()
        };
        TraceRecord::from(core_trace)
    }

    #[test]
    fn trace_projection_uses_one_grid_verdict_and_pane_free_hold() {
        let painted = TraceRecord::from(FrameTrace {
            verdict: Some(GridVerdict::Rows { spans: 2, rows: 3 }),
            ..FrameTrace::default()
        });
        assert_eq!(
            painted.verdict,
            Some(TraceVerdict::Rows { spans: 2, rows: 3 })
        );

        let held = TraceRecord::from(FrameTrace {
            outcome: FrameOutcome::HeldOnBridgeFailure,
            ..FrameTrace::default()
        });
        assert_eq!(held.outcome, TraceOutcome::HeldOnBridgeFailure);
    }

    #[test]
    fn serialize_deserialize_round_trip() {
        let mut rec = Recording::new(header());
        rec.push_frame(Frame {
            frame_idx: 0,
            t_ms: 0,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Painted,
            trace: trace(RenderStrategy::FullRebuild, 0b0100), // GEOMETRY
            grid_ops: vec![DrawOp::RectFill {
                rect: pix(0, 0, 10, 10),
                color: "#fff".into(),
            }],
            overlay_ops: vec![],
        });
        rec.push_frame(Frame {
            frame_idx: 1,
            t_ms: 17,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Painted,
            trace: trace(RenderStrategy::OverlayOnly, 0b1000), // OVERLAY
            grid_ops: vec![],
            overlay_ops: vec![DrawOp::RectStroke {
                rect: pix(0, 0, 20, 20),
                color: "#17a2d3".into(),
                width: 1.5,
            }],
        });

        let bytes = rec.serialize().expect("serialize");
        let back = Recording::deserialize(&bytes).expect("deserialize");
        assert_eq!(rec, back);
    }

    #[test]
    fn input_failure_codes_survive_recording_round_trip() {
        use iron_canvas_core::FrameInputFailure;

        let cases = [
            (FrameInputFailure::SelectedSheet, 0),
            (FrameInputFailure::SelectedView, 1),
            (FrameInputFailure::SheetMismatch, 2),
            (FrameInputFailure::FrozenRows, 3),
            (FrameInputFailure::FrozenColumns, 4),
            (FrameInputFailure::RowHeaderVisibility, 5),
            (FrameInputFailure::ColumnHeaderVisibility, 6),
            (FrameInputFailure::InvalidFrozenRowCount, 7),
            (FrameInputFailure::InvalidFrozenColumnCount, 8),
        ];
        for (failure, code) in cases {
            let mut rec = Recording::new(header());
            rec.push_frame(Frame {
                frame_idx: 0,
                t_ms: 0,
                origin: RecordOrigin::Live,
                result: RecordedPaintResult::Retry,
                trace: TraceRecord::from(FrameTrace {
                    outcome: FrameOutcome::HeldOnInputFailure(failure),
                    ..FrameTrace::default()
                }),
                grid_ops: Vec::new(),
                overlay_ops: Vec::new(),
            });
            let bytes = rec.serialize().expect("held recording serializes");
            let back = Recording::deserialize(&bytes).expect("held recording decodes");
            assert_eq!(back.frames.len(), 1);
            assert_eq!(back.frames[0].result, RecordedPaintResult::Retry);
            assert_eq!(
                back.frames[0].trace.outcome,
                TraceOutcome::HeldOnInputFailure { failure: code }
            );
        }
    }

    #[test]
    fn zero_op_retry_keeps_trace_in_timeline() {
        let core_trace = FrameTrace {
            attempt_seq: 7,
            outcome: iron_canvas_core::FrameOutcome::HeldOnInputFailure(
                iron_canvas_core::FrameInputFailure::SelectedSheet,
            ),
            ..FrameTrace::default()
        };
        let mut rec = Recording::new(header());
        rec.push_frame(Frame {
            frame_idx: 0,
            t_ms: 12,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Retry,
            trace: TraceRecord::from(core_trace),
            grid_ops: Vec::new(),
            overlay_ops: Vec::new(),
        });

        let back = Recording::deserialize(&rec.serialize().expect("serialize")).expect("decode");
        assert_eq!(back.frames.len(), 1);
        assert_eq!(back.frames[0].trace.attempt_seq, 7);
        assert_eq!(back.frames[0].result, RecordedPaintResult::Retry);
        assert!(back.frames[0].grid_ops.is_empty());
        assert!(back.frames[0].overlay_ops.is_empty());
    }

    #[test]
    fn header_version_pin() {
        // The header captures the iron-canvas version at serialize time.
        // The crate's CARGO_PKG_VERSION is the recorder crate's version
        // (intentional: the recorder is the producer of the format).
        let h = header();
        assert_eq!(h.iron_canvas_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(h.schema_version, ICR_SCHEMA_VERSION);
    }

    #[test]
    fn theme_snapshot_from_canvas_theme() {
        // Round-trip via serde to prove all 14 fields survive.
        let original = ThemeSnapshot::from(&CanvasTheme::light());
        let json = serde_json::to_string(&original).expect("serialize theme");
        let back: ThemeSnapshot = serde_json::from_str(&json).expect("deserialize theme");
        assert_eq!(original, back);

        // Spot-check one known value to confirm we're mirroring the right
        // field, not just any 14 strings.
        assert_eq!(original.cell_bg, "#FFFFFF");
        assert_eq!(original.pointing, "#1E6FD9");
    }

    #[test]
    fn schema_version_mismatch_is_rejected() {
        let mut rec = Recording::new(header());
        rec.header.schema_version = 99;
        let bytes = rec.serialize().expect("serialize");
        let err = Recording::deserialize(&bytes).expect_err("should reject");
        assert!(matches!(err, IcrError::Format(_)));
    }

    #[test]
    fn empty_input_rejected() {
        let err = Recording::deserialize(b"").expect_err("should reject");
        assert!(matches!(err, IcrError::Json(_)));
    }

    #[test]
    fn empty_frames_serializes_to_single_object() {
        let rec = Recording::new(header());
        let bytes = rec.serialize().expect("serialize");
        let s = std::str::from_utf8(&bytes).expect("utf-8");
        assert!(s.starts_with('{') && s.ends_with('}'), "want JSON object");
        assert!(s.contains("\"frames\":[]"), "empty frames array");
        let back = Recording::deserialize(&bytes).expect("deserialize");
        assert_eq!(back.frames.len(), 0);
    }

    /// A frame carrying the given grid ops, with a committed FullRebuild
    /// trace — the shape a capture's baseline writes.
    fn anchored_frame(index: u32, t_ms: u64, grid_ops: Vec<DrawOp>) -> Frame {
        Frame {
            frame_idx: index,
            t_ms,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Painted,
            trace: trace(RenderStrategy::FullRebuild, 0b0100),
            grid_ops,
            overlay_ops: Vec::new(),
        }
    }

    #[test]
    fn validated_recording_accepts_anchored_frames_and_metrics() {
        let mut rec = Recording::new(header());
        rec.push_frame(anchored_frame(
            0,
            0,
            vec![
                DrawOp::PushClip {
                    rect: pix(0, 0, 10, 10),
                },
                DrawOp::RectFill {
                    rect: pix(0, 0, 10, 10),
                    color: "#fff".into(),
                },
                DrawOp::PopClip,
            ],
        ));
        let validated = ValidatedRecording::try_from(rec).expect("well-formed recording");

        assert_eq!(validated.metrics().backing_size(), (800, 400));
        assert_eq!(validated.frame_count(), 1);
        assert_eq!(validated.frames().len(), 1);
    }

    /// A diagnostics-only prefix (no grid ops, no anchor) is legitimate: the
    /// recording may begin with held attempts that painted nothing.
    #[test]
    fn validated_recording_accepts_anchorless_diagnostics_prefix() {
        let mut rec = Recording::new(header());
        let mut held = anchored_frame(0, 0, Vec::new());
        held.trace.outcome = TraceOutcome::HeldOnBridgeFailure;
        held.trace.committed_seq = None;
        held.trace.effective = None;
        held.result = RecordedPaintResult::Retry;
        rec.push_frame(held);
        rec.push_frame(anchored_frame(1, 17, Vec::new()));

        assert!(ValidatedRecording::try_from(rec).is_ok());
    }

    #[test]
    fn validation_and_anchor_use_effective_committed_paint() {
        let mut fallback = anchored_frame(0, 0, vec![DrawOp::InvalidateCache]);
        fallback.trace.strategy = Some(RenderStrategy::ScrollBlit);
        assert!(fallback.is_replay_anchor());
        let mut rec = Recording::new(header());
        rec.push_frame(fallback.clone());
        assert!(ValidatedRecording::try_from(rec).is_ok());

        for failure in 0..3 {
            let mut invalid = fallback.clone();
            match failure {
                0 => invalid.trace.outcome = TraceOutcome::HeldOnBridgeFailure,
                1 => invalid.result = RecordedPaintResult::Retry,
                2 => invalid.trace.effective = Some(RenderStrategy::ScrollBlit),
                _ => unreachable!(),
            }
            invalid.trace.strategy = Some(RenderStrategy::FullRebuild);
            assert!(!invalid.is_replay_anchor());
            let mut rec = Recording::new(header());
            rec.push_frame(invalid);
            assert!(ValidatedRecording::try_from(rec).is_err());
        }
    }

    #[test]
    fn validated_recording_checks_directly_constructed_schema() {
        let mut rec = Recording::new(header());
        rec.push_frame(anchored_frame(0, 0, Vec::new()));
        rec.header.schema_version = ICR_SCHEMA_VERSION + 1;
        assert!(ValidatedRecording::try_from(rec).is_err());
    }

    #[test]
    fn validated_recording_rejects_invalid_canvas_metrics() {
        for (w, h, dpr) in [
            (f64::NAN, 400.0, 1.0),
            (800.0, f64::INFINITY, 1.0),
            (800.0, 400.0, 0.0),
            (800.0, 400.0, -1.0),
            (f64::from(i32::MAX) * 4.0, 400.0, 1.0),
        ] {
            let mut rec = Recording::new(IcrHeader::new(
                w,
                h,
                dpr,
                ThemeSnapshot::from(&CanvasTheme::light()),
                0,
            ));
            rec.push_frame(anchored_frame(0, 0, Vec::new()));
            assert!(
                ValidatedRecording::try_from(rec).is_err(),
                "{w} x {h} @ {dpr} must be rejected"
            );
        }
    }

    #[test]
    fn validated_recording_rejects_backwards_timestamps() {
        let mut rec = Recording::new(header());
        rec.push_frame(anchored_frame(0, 10, Vec::new()));
        rec.push_frame(anchored_frame(1, 9, Vec::new()));

        assert!(ValidatedRecording::try_from(rec).is_err());
    }

    #[test]
    fn validated_recording_rejects_unbalanced_brackets_and_non_finite_numbers() {
        let cases = [
            // Clip left open at the end of the frame.
            vec![DrawOp::PushClip {
                rect: pix(0, 0, 1, 1),
            }],
            // Pop with nothing pushed.
            vec![DrawOp::PopClip],
            // Group left open.
            vec![DrawOp::BeginGroup {
                class: GroupClass::Grid,
            }],
            // Non-finite stroke width.
            vec![DrawOp::RectStroke {
                rect: pix(0, 0, 1, 1),
                color: "#000".into(),
                width: f64::NAN,
            }],
            // Non-finite text origin.
            vec![DrawOp::FillText {
                text: "x".into(),
                x: f64::INFINITY,
                y: 0.0,
                font_css: "12px sans-serif".into(),
                color: "#000".into(),
                align: TextAlign::Start,
                baseline: TextBaseline::Top,
            }],
            // DPR transform that cannot name a scale.
            vec![DrawOp::ApplyDprTransform { dpr: 0.0 }],
        ];
        for (index, ops) in cases.into_iter().enumerate() {
            let mut rec = Recording::new(header());
            rec.push_frame(anchored_frame(0, 0, ops));
            assert!(
                ValidatedRecording::try_from(rec).is_err(),
                "case {index} must be rejected"
            );
        }
    }

    #[test]
    fn validated_recording_rejects_grid_ops_before_a_committed_anchor() {
        let mut rec = Recording::new(header());
        let mut blit = anchored_frame(
            0,
            0,
            vec![DrawOp::ClearRect {
                rect: pix(0, 0, 1, 1),
            }],
        );
        // A blit frame: painted grid ops, but no committed FullRebuild has
        // been seen, so a replay has no pixels to composite onto.
        blit.trace = trace(RenderStrategy::ScrollBlit, 0b0100);
        rec.push_frame(blit);

        assert!(ValidatedRecording::try_from(rec).is_err());
    }

    #[test]
    fn validated_recording_rejects_empty_frame_list() {
        let rec = Recording::new(header());

        assert!(ValidatedRecording::try_from(rec).is_err());
    }
}
