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
use crate::model_adapter::{CanvasModel, CanvasView};
use crate::theme::CanvasTheme;

/// Immutable, once-per-paint-attempt snapshot of the scalar model and
/// geometry inputs a frame needs. The only constructor is [`Self::capture`],
/// which enforces the fixed read order and the sheet/view consistency
/// check — there is no public mutation path that could assemble an
/// internally inconsistent snapshot (e.g. a view from one sheet paired with
/// another sheet's frozen counts).
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
    pub(crate) frozen_rows: i32,
    pub(crate) frozen_cols: i32,
    pub(crate) show_row_headers: bool,
    pub(crate) show_col_headers: bool,
    pub(crate) show_selection: bool,
}

/// Which scalar read a failed [`FrameInputs::capture`] attempt could not
/// complete. Named per accessor (rather than one generic "bridge failed")
/// so a held frame's diagnostics — and `FrameOutcome::HeldOnInputFailure` —
/// can say which read regressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameInputFailure {
    SelectedSheet,
    SelectedView,
    /// The standalone selected-sheet read and `CanvasView.sheet` disagreed.
    /// Building a frame from one accessor's sheet and the other's
    /// coordinates is not a valid fallback — see the module docs.
    SheetMismatch,
    FrozenRows,
    FrozenColumns,
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
        let frozen_rows = model
            .get_frozen_rows_count(sheet)
            .ok_or(FrameInputFailure::FrozenRows)?;
        let frozen_cols = model
            .get_frozen_columns_count(sheet)
            .ok_or(FrameInputFailure::FrozenColumns)?;
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
        self.frozen_rows
    }

    pub fn frozen_cols(&self) -> i32 {
        self.frozen_cols
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
