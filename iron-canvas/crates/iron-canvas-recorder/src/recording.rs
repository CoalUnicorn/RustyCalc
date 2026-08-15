//! `.icr` (iron-canvas-recording) format — single-document JSON artifact
//! capturing a paint-level recording for bug-repro / dev tooling.
//!
//! This module is the single source of truth for the on-disk schema.
//! Any change to a field name, variant tag, or layout is a format change
//! — bump `ICR_SCHEMA_VERSION` and regenerate the golden fixture
//! (`tests/fixtures/fresh_paint.icr` via `ICR_REGEN=1 cargo test
//! -p iron-canvas-recorder --test golden_fixture`).
//!
//! # On-disk layout (v5)
//!
//! UTF-8 bytes. One JSON object — a `Recording` with `header` and
//! `frames` fields. Standard JSON, so `jq .` and any JSON validator
//! reads it without special-casing:
//!
//! ```text
//! {"header":{"schema_version":5,"iron_canvas_version":"0.1.0-alpha.1",...},
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
//! | `schema_version`      | `u32`           | Always `ICR_SCHEMA_VERSION` (currently `5`). Mismatch -> load fails.  |
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

use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{FrameOutcome, FrameTrace, GridVerdict, PaintRegimeTag};

use crate::DrawOp;

/// Bumped only on breaking changes to the on-disk shape (added fields
/// with defaults don't bump). The loader rejects mismatched versions.
pub const ICR_SCHEMA_VERSION: u32 = 5;

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
    pub regime: Option<PaintRegimeTag>,
    pub effective: Option<PaintRegimeTag>,
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
                },
            },
        };
        let blit_fallback = trace.blit_fallback.map(|fallback| TraceBlitFallback {
            cold_cache: fallback.cold_cache,
        });
        Self {
            attempt_seq: trace.attempt_seq,
            committed_seq: trace.committed_seq,
            regime: trace.regime,
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
    pub fn deserialize(bytes: &[u8]) -> Result<Self, IcrError> {
        let rec: Recording = serde_json::from_slice(bytes)?;
        if rec.header.schema_version != ICR_SCHEMA_VERSION {
            return Err(IcrError::Format(format!(
                "schema_version mismatch: file={}, reader={}",
                rec.header.schema_version, ICR_SCHEMA_VERSION,
            )));
        }
        Ok(rec)
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
    use iron_canvas_core::theme::CanvasTheme;

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

    fn trace(regime: PaintRegimeTag, work: u8) -> TraceRecord {
        let core_trace = FrameTrace {
            attempt_seq: 1,
            committed_seq: Some(1),
            regime: Some(regime),
            effective: Some(regime),
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
            trace: trace(PaintRegimeTag::Fresh, 0b0100), // GEOMETRY
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
            trace: trace(PaintRegimeTag::Overlay, 0b1000), // OVERLAY
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
}
