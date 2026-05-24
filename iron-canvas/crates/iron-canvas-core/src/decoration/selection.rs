//! Blue selection border + semi-transparent fill + autofill handle for
//! the active selection. Three-phase paint: fill (under), renderer's
//! active-cell repaint, stroke + handle (over). The orchestrator drives
//! the phases by name.

use crate::chrome::{ActiveCellSnapshot, Chrome};
use crate::decoration::Layer;
use crate::geometry::constants::{AUTOFILL_HANDLE_BORDER_PX, SELECTION_BORDER_WIDTH};
use crate::painter::{GroupClass, PaintColor, Painter};
use crate::types::coord::RCRange;
use crate::CanvasModel;

#[derive(Default)]
pub struct SelectionLayer {
    /// `None` when the model has no selected view — sheet swap mid-tick,
    /// workbook reload, or pre-first-refresh. Every consumer that paints
    /// or queries against the selection must gate on `Some` so stale
    /// state from the previous sheet can't bleed through.
    pub selection_range: Option<RCRange>,
    pub active_cell: Option<ActiveCellSnapshot>,
}

/// Coordinates of the active cell the renderer must repaint between the
/// selection fill and stroke phases.
pub struct RepaintActiveCell {
    pub row: i32,
    pub col: i32,
}

impl SelectionLayer {
    /// Pull selection state from the model. Called by the orchestrator
    /// before any paint or hit-test, and before `screen_for_blit` so
    /// the active-cell snapshot reflects the model the blit would
    /// reuse pixels against. Clears to `None` when the model has no
    /// selected view so stale state from the previous sheet cannot
    /// paint a ghost selection.
    pub fn refresh(&mut self, model: &dyn CanvasModel) {
        let Some(view) = model.get_selected_view() else {
            self.selection_range = None;
            self.active_cell = None;
            return;
        };
        self.selection_range = Some(view.selection);
        self.active_cell = Some(ActiveCellSnapshot::capture(
            model,
            model.get_selected_sheet(),
            view.row,
            view.column,
        ));
    }

    /// Active-cell coords for the renderer's repaint pass, fired between
    /// the selection fill and stroke. `None` when no view is selected —
    /// the renderer skips the repaint entirely rather than repainting A1.
    pub fn active_cell_repaint(&self) -> Option<RepaintActiveCell> {
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
