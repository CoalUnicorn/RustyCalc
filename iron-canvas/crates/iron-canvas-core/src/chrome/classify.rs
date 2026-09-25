use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::CanvasModel;
use crate::frame::{FrameDelta, FrameInputs, RebuildReason};
use crate::geometry::slot::scroll_first;

use super::Chrome;

/// In-process digest of a formatted cell value. `DefaultHasher` output is
/// only stable within one std version, so the digest must never be
/// persisted or compared across builds. The newtype shape blocks
/// accidental serialization / cross-process comparison at the type
/// system level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellValueHash(u64);

/// Snapshot of the active cell at paint time. `Chrome::classify` re-hashes
/// the live model's value at the stored coords; a mismatch means the cell
/// was edited since this `Chrome` was painted, and the blit's kept band
/// would carry stale pixels. Catches the canonical edit-then-scroll case
/// without requiring the consumer to call `markContentDirty`.
#[derive(Clone, Debug, Default)]
pub struct ActiveCellSnapshot {
    pub row: i32,
    pub col: i32,
    /// `None` when the fetch was `BridgeFailed` — an *unknown* value, distinct
    /// from a known-empty (`Absent`) cell which hashes as `""`. An unknown on
    /// either side of `matches` can't prove the cell is unchanged, so it must
    /// reject the blit rather than blit stale pixels.
    pub value_hash: Option<CellValueHash>,
}

// `None` for `BridgeFailed` (value unknown); `Absent` is a known-empty cell and
// hashes as `""` so it stays comparable across frames.
fn hash_cell_value(
    model: &dyn CanvasModel,
    sheet: u32,
    row: i32,
    col: i32,
) -> Option<CellValueHash> {
    let fetched = model.get_formatted_cell_value(sheet, row, col);
    if fetched.is_bridge_failed() {
        return None;
    }
    let value = fetched.value().unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    Some(CellValueHash(hasher.finish()))
}

impl ActiveCellSnapshot {
    pub fn capture(model: &dyn CanvasModel, sheet: u32, row: i32, col: i32) -> Self {
        Self {
            row,
            col,
            value_hash: hash_cell_value(model, sheet, row, col),
        }
    }

    pub fn matches(&self, model: &dyn CanvasModel, sheet: u32) -> bool {
        // Blit only when both fetches are known AND equal. A `BridgeFailed`
        // (`None`) at capture or compare time means "can't prove unchanged" ->
        // reject, forcing a fresh repaint instead of reusing stale pixels.
        match (
            self.value_hash,
            hash_cell_value(model, sheet, self.row, self.col),
        ) {
            (Some(captured), Some(live)) => captured == live,
            _ => false,
        }
    }
}

impl Chrome {
    /// The one classifier that replaces the former split verdict
    /// (`FrameValidity` from `is_still_valid`, `Option<BlitPlan>` from
    /// `screen_for_blit`) with a single three-way [`FrameDelta`].
    /// `Chrome::next`/`Chrome::next_blit`'s own dispatch is unchanged; the
    /// orchestrator passes this verdict through `plan_frame` to the strategy
    /// executor.
    ///
    /// `prev = None` (no committed frame — right after a `resize`, or
    /// before the first paint) is itself the first ordered comparison, not
    /// a caller precondition. Every later comparison assumes a committed
    /// frame to compare against, in this fixed order (stable so a given
    /// divergence always reports under the same `RebuildReason`, even when
    /// several fields changed in the same tick):
    ///
    /// 1. no committed frame -> `NoCommittedFrame`
    /// 2. canvas size -> `Size`
    /// 3. DPR -> `Dpr`
    /// 4. theme -> `Theme`
    /// 5. model generation -> `Model`
    /// 6. sheet -> `Sheet`
    /// 7. frozen row/column counts -> `Freeze`
    /// 8. header visibility -> `Headers`
    /// 9. effective top/left scroll origin (see below)
    ///
    /// Steps 1-8 are hard breaks — any divergence rebuilds outright, the
    /// same theme/canvas-size/sheet/freeze rejections `is_still_valid` and
    /// `screen_for_blit` used to duplicate across two functions, plus the
    /// DPR/model-generation/header comparisons neither of them made.
    ///
    /// Step 9 is not itself a hard break; it forks the geometric scroll
    /// question:
    ///
    /// - neither axis moved -> `Stable` (slot vecs, and everything painted
    ///   from them, are reusable as-is);
    /// - both axes moved -> `Rebuild(TwoAxisScroll)` (no single blit shift
    ///   expresses a diagonal scroll);
    /// - exactly one axis moved:
    ///   - no committed active-cell snapshot to re-hash against ->
    ///     `Rebuild(MissingActiveSnapshot)`;
    ///   - the snapshot's value no longer matches the live model
    ///     (edit-then-scroll), or either read is unknown (`BridgeFailed`) ->
    ///     `Rebuild(ActiveCellChangedOrUnknown)`;
    ///   - the axis-specific overlap probe finds a safe kept band ->
    ///     `Scroll(plan)`;
    ///   - otherwise -> `Rebuild(IncompatibleScrollOverlap)`.
    ///
    /// Reads `model` only for the two checks that were always live-model
    /// reads, never part of `FrameInputs`: the active-cell re-hash and the
    /// overlap probe's row-height/col-width lookups. Never builds a
    /// `Chrome`, mutates cache state, calls a painter, or fetches cell
    /// content — qualification only. Construction (and its own independent
    /// reject, e.g. the row-header digit boundary) stays in
    /// `Chrome::next_blit`.
    pub fn classify(
        prev: Option<&Chrome>,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        active_cell: Option<&ActiveCellSnapshot>,
    ) -> FrameDelta {
        let Some(prev) = prev else {
            return FrameDelta::Rebuild(RebuildReason::NoCommittedFrame);
        };
        if inputs.size() != prev.canvas_size() {
            return FrameDelta::Rebuild(RebuildReason::Size);
        }
        if inputs.dpr() != prev.dpr() {
            return FrameDelta::Rebuild(RebuildReason::Dpr);
        }
        if inputs.theme() != &prev.theme {
            return FrameDelta::Rebuild(RebuildReason::Theme);
        }
        if inputs.model_generation() != prev.model_generation {
            return FrameDelta::Rebuild(RebuildReason::Model);
        }
        let sheet = inputs.sheet();
        if sheet != prev.sheet {
            return FrameDelta::Rebuild(RebuildReason::Sheet);
        }
        let frozen_rows = inputs.frozen_rows();
        let frozen_cols = inputs.frozen_cols();
        if frozen_rows != prev.pane_set.rows.frozen_count()
            || frozen_cols != prev.pane_set.cols.frozen_count()
        {
            return FrameDelta::Rebuild(RebuildReason::Freeze);
        }
        if inputs.show_row_headers() != prev.show_row_headers
            || inputs.show_col_headers() != prev.show_col_headers
        {
            return FrameDelta::Rebuild(RebuildReason::Headers);
        }

        let view = inputs.view();
        let new_top = scroll_first(frozen_rows, view.top_row);
        let new_left = scroll_first(frozen_cols, view.left_column);
        let top_changed = new_top != prev.pane_set.top_row();
        let left_changed = new_left != prev.pane_set.left_column();

        match (top_changed, left_changed) {
            (false, false) => FrameDelta::Stable,
            (true, true) => FrameDelta::Rebuild(RebuildReason::TwoAxisScroll),
            _ => {
                // Defensive content check: if the cell painted as active no
                // longer matches the live model, the blit's kept band would
                // shift pre-edit pixels (canonical edit-then-scroll bug when
                // the consumer missed `markContentDirty`). An absent snapshot
                // (nothing captured yet this attempt) can't be re-hashed at
                // all, so it gets its own, more specific reason.
                let Some(active) = active_cell else {
                    return FrameDelta::Rebuild(RebuildReason::MissingActiveSnapshot);
                };
                if !active.matches(model, sheet) {
                    return FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown);
                }
                let plan = if top_changed {
                    super::blit::try_blit_rows(prev, model, sheet, new_top)
                } else {
                    super::blit::try_blit_cols(prev, model, sheet, new_left)
                };
                match plan {
                    Some(plan) => FrameDelta::Scroll(plan),
                    None => FrameDelta::Rebuild(RebuildReason::IncompatibleScrollOverlap),
                }
            }
        }
    }
}
