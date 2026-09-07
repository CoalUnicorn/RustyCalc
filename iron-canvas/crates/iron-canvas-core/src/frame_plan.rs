//! Frame-level input capture.
//!
//! [`FrameInputs`] snapshots every scalar model/geometry read a paint
//! attempt needs — selected sheet, selected view, frozen counts, header
//! visibility, selection visibility — exactly once, before any geometry
//! walk, cache invalidation, or paint runs. Stage 2 (and everything before
//! it) treated a failed scalar bridge read as license to substitute a
//! synthetic default (an all-A1 view, sheet `0`, "headers visible"): the
//! frame still built, just against fabricated state. [`FrameInputs::capture`]
//! replaces that with a fallible constructor — any one bridge failure holds
//! the whole attempt (see `Orchestrator::render_pending`'s capture-failure
//! handling) instead of silently painting the wrong sheet.
//!
//! [`FrameDelta`] and [`RebuildReason`] are the classification types
//! [`Chrome::classify`](crate::chrome::Chrome::classify) produces: comparing
//! a captured `FrameInputs` against the previously committed `Chrome` to
//! decide whether the next frame is a Stable reuse, a safe scroll, or a
//! full rebuild (and why). They live here — rather than in `chrome/mod.rs`
//! alongside the classifier itself — so the crate's public re-export
//! surface stays stable regardless of which module owns the comparison
//! logic; `Chrome::classify` is the sole producer.

use std::rc::Rc;

use crate::geometry::CanvasSize;
use crate::geometry::constants::{LAST_COLUMN, LAST_ROW};
use crate::model_adapter::{CanvasModel, CanvasView};
use crate::theme::CanvasTheme;

/// Validated count of frozen leading rows or columns along one axis.
///
/// The model reports frozen counts as raw signed `i32` values (see
/// [`CanvasModel::get_frozen_rows_count`]). Those values feed
/// `Vec::reserve(frozen_count as usize)` and the `1..=frozen_count`
/// frozen-band walk in `AxisSlots::fill` (`geometry/slot.rs`), so an
/// unchecked count is not a harmless geometry quirk: a negative value
/// becomes an enormous `usize` capacity request, and a count past the
/// axis's last id asks the frozen-band walk to read rows or columns the
/// sheet cannot contain. This type is the one checked path into that
/// geometry — construction requires `0 <= count <= axis_last_id`, and no
/// unchecked constructor exists — so [`FrameInputs::capture`] stores frozen
/// counts only as `FrozenCount` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrozenCount {
    count: i32,
}

impl FrozenCount {
    /// The only constructor: `Ok` only when `count` lies in
    /// `0..=axis_last_id`; `Err(out_of_range)` otherwise. `axis_last_id` is
    /// the axis's last addressable id (`LAST_ROW` for rows,
    /// `LAST_COLUMN` for columns).
    fn checked(
        count: i32,
        axis_last_id: i32,
        out_of_range: FrameInputFailure,
    ) -> Result<Self, FrameInputFailure> {
        if (0..=axis_last_id).contains(&count) {
            Ok(Self { count })
        } else {
            Err(out_of_range)
        }
    }

    /// The validated raw count for the geometry walks.
    pub(crate) fn get(self) -> i32 {
        self.count
    }
}

/// Immutable, once-per-paint-attempt snapshot of the scalar model and
/// geometry inputs a frame needs. The only constructor is [`Self::capture`],
/// which enforces the fixed read order, the frozen-count range checks, and
/// the sheet/view consistency check — there is no public mutation path that
/// could assemble an internally inconsistent snapshot (e.g. a view from one
/// sheet paired with another sheet's frozen counts).
///
/// Fields stay `pub(crate)`: `Chrome`'s constructor and the classifier this
/// struct feeds are both in-crate. A handful of read-only accessors below
/// expose values to out-of-crate integration tests without opening a
/// mutation path.
#[derive(Clone)]
pub struct FrameInputs {
    pub(crate) size: CanvasSize,
    pub(crate) dpr: f64,
    pub(crate) theme: Rc<CanvasTheme>,
    pub(crate) model_generation: u64,
    pub(crate) sheet: u32,
    pub(crate) view: CanvasView,
    pub(crate) frozen_rows: FrozenCount,
    pub(crate) frozen_cols: FrozenCount,
    pub(crate) show_row_headers: bool,
    pub(crate) show_col_headers: bool,
    pub(crate) show_selection: bool,
}

/// Which scalar input a failed [`FrameInputs::capture`] attempt could not
/// accept. Either an accessor read failed (the bridge returned `None`) or —
/// for the two frozen counts — the model returned a value outside the axis's
/// valid range. Named per accessor (rather than one generic "bridge failed")
/// so a held frame's diagnostics — and `FrameOutcome::HeldOnInputFailure` —
/// can say which input regressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameInputFailure {
    SelectedSheet,
    SelectedView,
    /// The standalone selected-sheet read and `CanvasView.sheet` disagreed.
    /// Building a frame from one accessor's sheet and the other's
    /// coordinates is not a valid fallback — see the module docs.
    SheetMismatch,
    /// The frozen-row-count read itself failed (`None` from the bridge).
    FrozenRows,
    /// The frozen-column-count read itself failed (`None` from the bridge).
    FrozenColumns,
    /// The frozen-row-count read succeeded but returned a value outside
    /// `0..=LAST_ROW`. Rejected at capture so the value can never reach
    /// `Vec::reserve` or the frozen-band walk as an unchecked signed count.
    InvalidFrozenRowCount,
    /// Column mirror of [`Self::InvalidFrozenRowCount`], bounded by
    /// `LAST_COLUMN`.
    InvalidFrozenColumnCount,
    RowHeaderVisibility,
    ColumnHeaderVisibility,
}

impl FrameInputs {
    /// Capture every scalar frame input exactly once, in the fixed order
    /// the Stage 3 plan requires:
    ///
    /// 1. selected sheet;
    /// 2. selected view (asserted to agree with the sheet);
    /// 3. frozen row count;
    /// 4. frozen column count;
    /// 5. row-header visibility;
    /// 6. column-header visibility;
    /// 7. selection visibility.
    ///
    /// `size`, `dpr`, `theme`, and `model_generation` come from the caller
    /// (`Orchestrator`) rather than the model — they are host/orchestrator
    /// state, not bridge reads. Selection visibility
    /// (`CanvasModel::get_show_selection`) is infallible by design (default
    /// `true`), so it cannot itself hold the attempt.
    ///
    /// Any other bridge failure holds the whole attempt: there is no
    /// partial `FrameInputs`, and no fallback default is substituted for a
    /// failed read.
    pub fn capture(
        model: &dyn CanvasModel,
        size: CanvasSize,
        dpr: f64,
        theme: Rc<CanvasTheme>,
        model_generation: u64,
    ) -> Result<Self, FrameInputFailure> {
        let sheet = model
            .get_selected_sheet()
            .ok_or(FrameInputFailure::SelectedSheet)?;
        let view = model
            .get_selected_view()
            .ok_or(FrameInputFailure::SelectedView)?;
        if view.sheet != sheet {
            return Err(FrameInputFailure::SheetMismatch);
        }
        // Frozen counts are range-checked before they can reach the slot
        // geometry: `AxisSlots::fill` reserves `frozen_count as usize` and
        // walks `1..=frozen_count`, so a negative value would be an enormous
        // capacity request and an over-limit value would read rows/columns
        // the sheet cannot contain. Reads 3 and 4 in the order above.
        let frozen_rows = FrozenCount::checked(
            model
                .get_frozen_rows_count(sheet)
                .ok_or(FrameInputFailure::FrozenRows)?,
            LAST_ROW,
            FrameInputFailure::InvalidFrozenRowCount,
        )?;
        let frozen_cols = FrozenCount::checked(
            model
                .get_frozen_columns_count(sheet)
                .ok_or(FrameInputFailure::FrozenColumns)?,
            LAST_COLUMN,
            FrameInputFailure::InvalidFrozenColumnCount,
        )?;
        let show_row_headers = model
            .get_show_row_headers(sheet)
            .ok_or(FrameInputFailure::RowHeaderVisibility)?;
        let show_col_headers = model
            .get_show_col_headers(sheet)
            .ok_or(FrameInputFailure::ColumnHeaderVisibility)?;
        let show_selection = model.get_show_selection();

        Ok(FrameInputs {
            size,
            dpr,
            theme,
            model_generation,
            sheet,
            view,
            frozen_rows,
            frozen_cols,
            show_row_headers,
            show_col_headers,
            show_selection,
        })
    }

    pub fn size(&self) -> CanvasSize {
        self.size
    }

    pub fn theme(&self) -> &Rc<CanvasTheme> {
        &self.theme
    }

    pub fn sheet(&self) -> u32 {
        self.sheet
    }

    pub fn view(&self) -> CanvasView {
        self.view
    }

    pub fn frozen_rows(&self) -> i32 {
        self.frozen_rows.get()
    }

    pub fn frozen_cols(&self) -> i32 {
        self.frozen_cols.get()
    }

    pub fn show_row_headers(&self) -> bool {
        self.show_row_headers
    }

    pub fn show_col_headers(&self) -> bool {
        self.show_col_headers
    }

    pub fn show_selection(&self) -> bool {
        self.show_selection
    }

    pub fn dpr(&self) -> f64 {
        self.dpr
    }

    pub fn model_generation(&self) -> u64 {
        self.model_generation
    }
}

/// Outcome of classifying a captured [`FrameInputs`] against the previously
/// committed `Chrome`. Produced by
/// [`Chrome::classify`](crate::chrome::Chrome::classify); consumed by the
/// orchestrator's planner (`plan_frame` in `orchestrator.rs`), which turns
/// one `FrameDelta` plus the attempt's taken `PendingWork` into a closed
/// `FramePlan` — see that module's doc comment for the complete
/// `PendingWork` x `FrameDelta` table.
#[derive(Clone)]
pub enum FrameDelta {
    Stable,
    Scroll(crate::chrome::BlitPlan),
    Rebuild(RebuildReason),
}

/// Why [`FrameDelta::Rebuild`] fired. Named per hard-break check (rather
/// than one generic "geometry changed") so a rebuilt frame's diagnostics can
/// say which committed field diverged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebuildReason {
    NoCommittedFrame,
    Size,
    Dpr,
    Theme,
    Model,
    Sheet,
    Freeze,
    Headers,
    TwoAxisScroll,
    MissingActiveSnapshot,
    ActiveCellChangedOrUnknown,
    IncompatibleScrollOverlap,
}
