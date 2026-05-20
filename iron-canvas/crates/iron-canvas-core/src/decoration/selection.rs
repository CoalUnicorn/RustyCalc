//! Blue selection border + semi-transparent fill + autofill handle for
//! the active selection.

use crate::chrome::{ActiveCellSnapshot, Chrome};
use crate::decoration::{Layer, RepaintActiveCell};
use crate::geometry::constants::{AUTOFILL_HANDLE_BORDER_PX, SELECTION_BORDER_WIDTH};
use crate::painter::{PaintColor, Painter};
use crate::types::coord::RCRange;
use crate::CanvasModel;

#[derive(Default)]
pub struct SelectionLayer {
    pub selection_range: RCRange,
    pub active_cell: ActiveCellSnapshot,
}

impl SelectionLayer {
    /// Pull selection state from the model. Called by the orchestrator
    /// before any paint or hit-test, and before `screen_for_blit` so
    /// the active-cell snapshot reflects the model the blit would
    /// reuse pixels against.
    pub fn refresh(&mut self, model: &dyn CanvasModel) {
        if let Some(view) = model.get_selected_view() {
            self.selection_range = view.selection;
            self.active_cell = ActiveCellSnapshot::capture(
                model,
                model.get_selected_sheet(),
                view.row,
                view.column,
            );
        }
    }
}

impl Layer for SelectionLayer {
    fn paint(&self, _model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter) {
        let Some(cell) = frame.range_rect(self.selection_range.normalized()) else {
            return;
        };
        painter.rect_fill(
            cell,
            PaintColor::from_theme_str(&frame.theme.selection_fill),
        );
    }

    fn after_paint_renderer_hook(
        &self,
        model: &dyn CanvasModel,
        _frame: &Chrome,
    ) -> Option<RepaintActiveCell> {
        let view = model.get_selected_view()?;
        Some(RepaintActiveCell {
            row: view.row,
            col: view.column,
        })
    }

    fn paint_after_hook(&self, _model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter) {
        let Some(cell) = frame.range_rect(self.selection_range.normalized()) else {
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

        if let Some(handle) = frame.autofill_handle_rect(self.selection_range) {
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
