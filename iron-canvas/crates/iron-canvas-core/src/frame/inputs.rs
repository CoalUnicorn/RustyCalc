//! Frame-level input capture.
//!
//! [`FrameInputs`] snapshots every scalar model/geometry read a paint
//! attempt needs — selected sheet, selected view, frozen counts, header
//! visibility, selection visibility — exactly once, before any geometry
//! walk, cache invalidation, or paint runs. [`FrameInputs::capture`] is a
//! fallible constructor: any one bridge failure holds the whole attempt
//! instead of silently painting fabricated state (see
//! `Orchestrator::render_pending`), and [`FrameInputFailure`] names which
//! input regressed.

use std::rc::Rc;

use crate::geometry::CanvasMetrics;
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
/// Fields are private: only [`Self::capture`] in this module may read or
/// write them, so the read order, the frozen-count checks, and the
/// sheet/view consistency check cannot be bypassed by assembling a
/// snapshot elsewhere in the crate. The read-only accessors below are the
/// crate-wide and out-of-crate read surface.
#[derive(Clone)]
pub struct FrameInputs {
    /// Validated canvas metrics. The orchestrator parsed the host's resize
    /// arguments once; carrying the parsed value (rather than a raw size/DPR
    /// pair) is what makes every downstream `logical_extent`/`backing_size`
    /// cast infallible.
    metrics: CanvasMetrics,
    theme: Rc<CanvasTheme>,
    model_generation: u64,
    sheet: u32,
    view: CanvasView,
    frozen_rows: FrozenCount,
    frozen_cols: FrozenCount,
    show_row_headers: bool,
    show_col_headers: bool,
    show_selection: bool,
}

/// Which scalar input a failed [`FrameInputs::capture`] attempt could not
/// accept. Either an accessor read failed (the bridge returned `None`) or —
/// for the two frozen counts — the model returned a value outside the axis's
/// valid range. Named per accessor (rather than one generic "bridge failed")
/// so a held frame's diagnostics — and `FrameOutcome::HeldOnInputFailure` —
/// can say which input regressed.
///
/// The discriminant is the wire code a `.icr` recording stores (the recorder
/// writes `failure as u8` and validates the decoded value against
/// [`Self::LAST_CODE`]). The values are explicit and append-only: renumbering
/// would silently relabel every recorded frame. Declaration order is the read
/// order of `capture`, which is why the numbers are not in ascending order.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameInputFailure {
    SelectedSheet = 0,
    SelectedView = 1,
    /// The standalone selected-sheet read and `CanvasView.sheet` disagreed.
    /// Building a frame from one accessor's sheet and the other's
    /// coordinates is not a valid fallback — see the module docs.
    SheetMismatch = 2,
    /// The frozen-row-count read itself failed (`None` from the bridge).
    FrozenRows = 3,
    /// The frozen-column-count read itself failed (`None` from the bridge).
    FrozenColumns = 4,
    /// The frozen-row-count read succeeded but returned a value outside
    /// `0..=LAST_ROW`. Rejected at capture so the value can never reach
    /// `Vec::reserve` or the frozen-band walk as an unchecked signed count.
    InvalidFrozenRowCount = 7,
    /// Column mirror of [`Self::InvalidFrozenRowCount`], bounded by
    /// `LAST_COLUMN`.
    InvalidFrozenColumnCount = 8,
    RowHeaderVisibility = 5,
    ColumnHeaderVisibility = 6,
}

impl FrameInputFailure {
    /// Highest wire code. The variants carry every code in `0..=LAST_CODE`
    /// (in declaration order 0-4, 7, 8, 5, 6), so `code > LAST_CODE` is
    /// exactly "no variant carries this code" — the check a reader applies to
    /// a decoded recording.
    pub const LAST_CODE: u8 = Self::InvalidFrozenColumnCount as u8;
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
    /// `metrics`, `theme`, and `model_generation` come from the caller
    /// (`Orchestrator`) rather than the model — they are host/orchestrator
    /// state, not bridge reads, and `metrics` was already parsed from the
    /// host's resize arguments. Selection visibility
    /// (`CanvasModel::get_show_selection`) is infallible by design (default
    /// `true`), so it cannot itself hold the attempt.
    ///
    /// Any other bridge failure holds the whole attempt: there is no
    /// partial `FrameInputs`, and no fallback default is substituted for a
    /// failed read.
    pub fn capture(
        model: &dyn CanvasModel,
        metrics: CanvasMetrics,
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
            metrics,
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

    /// Validated canvas metrics for this attempt.
    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics
    }

    pub fn size(&self) -> CanvasSize {
        self.metrics.size()
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
        self.metrics.dpr()
    }

    pub fn model_generation(&self) -> u64 {
        self.model_generation
    }
}
