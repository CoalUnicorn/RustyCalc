//! Phase-3 overlays: selection rectangle, autofill drag preview, clipboard
//! marching ants, point-mode range, formula-ref highlights.
//!
//! Everything here is drawn *after* cell backgrounds + headers but *before*
//! cell text (Phase 4). Each helper bails early via
//! `range_pixel_bounds(...)?` when the range is entirely outside the
//! drawable fold, so overlays never leak onto the canvas for off-screen
//! refs like `=BB3`.

use crate::model::{FormulaRef, RCRange, SheetArea};
use crate::theme::{FORMULA_REF_COLORS, FORMULA_REF_TINTS};
use crate::{CanvasModel, Point};

use super::super::geometry::{PixelRect, AUTOFILL_HANDLE_PX};
use super::super::model::CellAddress;
use super::{
    CanvasRenderer, FrameContext, DASHED_BORDER_WIDTH, SELECTION_BORDER_WIDTH,
    STANDARD_BORDER_WIDTH,
};

/// The target cell during an autofill-handle drag.
///
/// Replaces the anonymous `Option<(i32, i32)>` in `RenderOverlays` with a
/// named struct so the fields are self-documenting at every call site.
#[derive(Copy, Clone, PartialEq)]
pub struct AutofillTarget {
    pub row: i32,
    pub col: i32,
}

/// Controls whether `draw_dashed_range` fills the interior with a light tint.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum DashFill {
    /// Outline only (used for clipboard marching ants).
    Outline,
    /// Outline + semi-transparent fill tint (used for point-mode range and
    /// formula refs). Carries a precomputed `rgba(...)` tint so paint never
    /// allocates per frame.
    Tinted(&'static str),
}

impl CanvasRenderer {
    /// Draw the blue selection border, semi-transparent fill, and autofill
    /// handle for the current selection.
    pub(super) fn draw_selection(&self, model: &dyn CanvasModel, frame: &FrameContext) {
        let view = model.get_selected_view();
        let sheet = model.get_selected_sheet();
        let addr = CellAddress {
            sheet,
            row: view.row,
            column: view.column,
        };
        let Some(b) = self.range_pixel_bounds(frame, RCRange::from_view(model).normalized()) else {
            return;
        };

        self.rect_fill(b, self.theme.selection_fill);

        // Restore the active cell's fill + borders on top of the selection
        // tint so its actual style shows through while selected. Phase 4
        // paints text over everything later.
        self.repaint_active_cell(model, addr, frame);

        self.rect_stroke(b, self.theme.selection_color, SELECTION_BORDER_WIDTH);

        let handle = PixelRect {
            top_left: Point {
                x: b.right() - AUTOFILL_HANDLE_PX / 2.0,
                y: b.bottom() - AUTOFILL_HANDLE_PX / 2.0,
            },
            width: AUTOFILL_HANDLE_PX,
            height: AUTOFILL_HANDLE_PX,
        };
        self.rect_fill(handle, self.theme.selection_color);
    }

    /// Dashed preview of the autofill-handle drag target.
    pub(super) fn draw_extend_preview(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        target: AutofillTarget,
    ) {
        let sel = RCRange::from_view(model).normalized();
        let range = RCRange {
            r1: sel.r1.min(target.row),
            c1: sel.c1.min(target.col),
            r2: sel.r2.max(target.row),
            c2: sel.c2.max(target.col),
        };
        let Some(b) = self.range_pixel_bounds(frame, range) else {
            return;
        };

        self.rect_dashed(b, self.theme.selection_color, STANDARD_BORDER_WIDTH);
    }

    /// Clipboard marching-ants border around the last Ctrl+C copied range.
    /// No-op when the clipboard is empty or lives on another sheet.
    pub(super) fn draw_clipboard_overlay(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        clipboard: Option<&SheetArea>,
    ) {
        let sheet = model.get_selected_sheet();
        let Some(cb) = clipboard else { return };
        if cb.sheet != sheet {
            return;
        }
        self.draw_dashed_range(
            frame,
            cb.range.normalized(),
            self.theme.selection_color,
            DashFill::Outline,
        );
    }

    /// Point-mode range highlight - blue dashed outline with an 8% fill tint.
    pub(super) fn draw_point_overlay(&self, frame: &FrameContext, point_range: Option<RCRange>) {
        let Some(pr) = point_range else { return };
        self.draw_dashed_range(
            frame,
            pr.normalized(),
            self.theme.pointing,
            DashFill::Tinted(self.theme.pointing_tint),
        );
    }

    /// Per-reference formula highlights for the in-edit cell. One color per
    /// `color_idx` (mod the palette), off-sheet refs silently skipped.
    pub(super) fn draw_formula_ref_overlays(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        refs: &[FormulaRef],
    ) {
        let sheet = model.get_selected_sheet();
        for fr in refs {
            if fr.sheet_area.sheet != sheet {
                continue;
            }
            let idx = fr.color_idx % FORMULA_REF_COLORS.len();
            self.draw_dashed_range(
                frame,
                fr.sheet_area.range.normalized(),
                FORMULA_REF_COLORS[idx],
                DashFill::Tinted(FORMULA_REF_TINTS[idx]),
            );
        }
    }

    /// Dashed rectangle over `range`. Used for clipboard marching ants
    /// (`DashFill::Outline`) and point-mode / formula-ref highlights
    /// (`DashFill::Tinted`, which also draws the carried 8% fill).
    pub(super) fn draw_dashed_range(
        &self,
        frame: &FrameContext,
        range: RCRange,
        color: &str,
        fill: DashFill,
    ) {
        let Some(b) = self.range_pixel_bounds(frame, range) else {
            return;
        };

        self.rect_dashed(b, color, DASHED_BORDER_WIDTH);

        if let DashFill::Tinted(tint) = fill {
            self.rect_fill(b, tint);
        }
    }
}
