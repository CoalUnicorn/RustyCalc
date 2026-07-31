//! Blue selection border + semi-transparent fill + autofill handle for
//! the active selection. Three-phase paint: fill (under), renderer's
//! active-cell repaint, stroke + handle (over). The orchestrator drives
//! the phases by name.

use crate::CanvasModel;
use crate::chrome::{ActiveCellSnapshot, Chrome};
use crate::decoration::Layer;
use crate::geometry::constants::{AUTOFILL_HANDLE_BORDER_PX, SELECTION_BORDER_WIDTH};
use crate::model_adapter::CanvasView;
use crate::painter::{GroupClass, PaintColor, Painter};
use crate::types::coord::RCRange;

#[derive(Default)]
pub struct SelectionLayer {
    /// `None` when the model has no selected view — sheet swap mid-tick,
    /// workbook reload, or pre-first-refresh. Every consumer that paints
    /// or queries against the selection must gate on `Some` so stale
    /// state from the previous sheet can't bleed through.
    pub selection_range: Option<RCRange>,
    /// Retained even when `show_selection` is false — `Chrome::classify`'s
    /// scroll-safety re-hash reads this directly, independent of whether
    /// the active cell is actually painted this frame. Only
    /// `active_cell_repaint`'s paint hook gates on `show_selection`; this
    /// field itself never clears just because selection painting is off.
    pub active_cell: Option<ActiveCellSnapshot>,
    /// Last captured selection visibility, so `active_cell_repaint` can
    /// suppress its paint hook independently of `active_cell`'s presence.
    show_selection: bool,
}

/// Coordinates of the active cell the renderer must repaint between the
/// selection fill and stroke phases.
pub struct RepaintActiveCell {
    pub row: i32,
    pub col: i32,
}

impl SelectionLayer {
    /// Pull selection state from this paint attempt's captured
    /// `FrameInputs` (`sheet`, `view`, `show_selection` — already read once
    /// and validated by `FrameInputs::capture`), not a fresh model read.
    /// Called by the orchestrator before any paint or hit-test, and before
    /// the next attempt's `Chrome::classify` so the active-cell snapshot
    /// reflects the same inputs a later blit-reuse decision would compare
    /// against. `active_cell` is always refreshed, even when the model has
    /// deliberately turned selection painting off (`show_selection == false`
    /// — the data-grid adapter's `show_selection(false)`): it is the scroll-
    /// safety snapshot `Chrome::classify` re-hashes against, independent of
    /// whether anything paints. `selection_range` clears to `None` in that
    /// case instead, so a selection-less host paints no selection fill or
    /// stroke; `active_cell_repaint` independently suppresses its own paint
    /// hook via the captured `show_selection` flag.
    pub fn refresh(
        &mut self,
        model: &dyn CanvasModel,
        sheet: u32,
        view: &CanvasView,
        show_selection: bool,
    ) {
        self.show_selection = show_selection;
        self.active_cell = Some(ActiveCellSnapshot::capture(
            model,
            sheet,
            view.row,
            view.column,
        ));
        self.selection_range = if show_selection {
            Some(view.selection)
        } else {
            None
        };
    }

    /// Active-cell coords for the renderer's repaint pass, fired between
    /// the selection fill and stroke. `None` when no view is selected, or
    /// when the captured `show_selection` is false — a selection-less host
    /// must draw no active-cell repaint even though `active_cell` itself
    /// stays populated for `Chrome::classify`'s scroll-safety re-hash.
    pub fn active_cell_repaint(&self) -> Option<RepaintActiveCell> {
        if !self.show_selection {
            return None;
        }
        self.active_cell.as_ref().map(|a| RepaintActiveCell {
            row: a.row,
            col: a.col,
        })
    }

    /// Stroke + autofill-handle. Painted *after* the renderer's active-cell
    /// repaint so the stroke crisply overlays the cell border.
    pub fn paint_stroke(&self, frame: &Chrome, painter: &dyn Painter) {
        let Some(range) = self.selection_range else {
            return;
        };
        let Some(cell) = frame.range_rect(range.normalized()) else {
            return;
        };

        // Inset by half stroke width so the centered stroke fits inside
        // the cell — without this, the outer half spills past row-1 / col-A
        // onto the chrome buffer pixel.
        let stroke_rect = cell.inset(SELECTION_BORDER_WIDTH / 2, SELECTION_BORDER_WIDTH / 2);
        painter.rect_stroke(
            stroke_rect,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(SELECTION_BORDER_WIDTH),
        );

        if let Some(handle) = frame.autofill_handle_rect(range) {
            painter.rect_fill(
                handle,
                PaintColor::from_theme_str(&frame.theme.selection_color),
            );
            painter.rect_stroke(
                handle,
                PaintColor::from_theme_str(&frame.theme.cell_bg),
                f64::from(AUTOFILL_HANDLE_BORDER_PX),
            );
        }
    }
}

impl Layer for SelectionLayer {
    fn group(&self) -> GroupClass {
        GroupClass::SelectionFill
    }

    fn paint(&self, frame: &Chrome, painter: &dyn Painter) {
        let Some(range) = self.selection_range else {
            return;
        };
        let Some(cell) = frame.range_rect(range.normalized()) else {
            return;
        };
        painter.rect_fill(
            cell,
            PaintColor::from_theme_str(&frame.theme.selection_fill),
        );
    }
}
